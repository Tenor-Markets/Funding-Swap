#![allow(dead_code)]
#![allow(clippy::result_large_err)]

use std::path::Path;

use anchor_lang::{system_program, InstructionData};
use anyhow::{anyhow, Context, Result};
use funding_swap::{
    instruction,
    state::{AuthorityUpdate, GlobalGovernanceConfig, GovernanceCategory},
    GovernanceInitializationArgs, ID as FUNDING_SWAP_PROGRAM_ID,
};
use litesvm::{types::TransactionResult, LiteSVM};
use solana_sdk::{
    instruction::{AccountMeta, Instruction, InstructionError},
    pubkey::Pubkey,
    signature::Keypair,
    signer::Signer,
    transaction::{Transaction, TransactionError},
};

pub const BPF_LOADER_UPGRADEABLE_ID: Pubkey =
    solana_sdk::pubkey!("BPFLoaderUpgradeab1e11111111111111111111111");

pub fn add_funding_swap_program(svm: &mut LiteSVM) -> Result<()> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/deploy/funding_swap.so");

    svm.add_program_from_file(FUNDING_SWAP_PROGRAM_ID, &path)
        .context("load funding_swap.so")?;

    Ok(())
}

pub fn new_svm() -> Result<LiteSVM> {
    let mut svm = LiteSVM::new();

    add_funding_swap_program(&mut svm)?;

    Ok(svm)
}

pub fn program_data_address() -> Pubkey {
    Pubkey::find_program_address(
        &[FUNDING_SWAP_PROGRAM_ID.as_ref()],
        &BPF_LOADER_UPGRADEABLE_ID,
    )
    .0
}

/// LiteSVM loads programs with no upgrade authority. The governance bootstrap
/// requires one, so patch the ProgramData metadata in place.
pub fn set_program_upgrade_authority(svm: &mut LiteSVM, authority: Pubkey) -> Result<()> {
    let program_data_address = program_data_address();

    let mut account = svm
        .get_account(&program_data_address)
        .ok_or_else(|| anyhow!("program data account not found"))?;

    // UpgradeableLoaderState::ProgramData layout:
    // u32 discriminant, u64 slot, Option<Pubkey> authority.
    const UPGRADE_AUTHORITY_TAG_OFFSET: usize = 4 + 8;
    const UPGRADE_AUTHORITY_OFFSET: usize = UPGRADE_AUTHORITY_TAG_OFFSET + 1;

    account.data[UPGRADE_AUTHORITY_TAG_OFFSET] = 1;
    account.data[UPGRADE_AUTHORITY_OFFSET..UPGRADE_AUTHORITY_OFFSET + 32]
        .copy_from_slice(authority.as_ref());

    svm.set_account(program_data_address, account)
        .context("patch program upgrade authority")?;

    Ok(())
}

pub fn governance_address() -> Pubkey {
    Pubkey::find_program_address(&[b"governance"], &FUNDING_SWAP_PROGRAM_ID).0
}

pub fn governance_args(authority: Pubkey) -> GovernanceInitializationArgs {
    GovernanceInitializationArgs {
        authorities: default_authorities(authority),
        subaccount_creation_fee_lamports: 25,
        subaccount_creation_fee_destination: Pubkey::new_unique(),
    }
}

pub fn read_governance(svm: &LiteSVM, governance: Pubkey) -> Result<GlobalGovernanceConfig> {
    let account = svm
        .get_account(&governance)
        .ok_or_else(|| anyhow!("governance account not found"))?;

    let data = account
        .data
        .get(8..)
        .ok_or_else(|| anyhow!("governance account is smaller than its discriminator"))?;

    Ok(bytemuck::pod_read_unaligned::<GlobalGovernanceConfig>(data))
}

pub fn default_authorities(
    authority: Pubkey,
) -> [Pubkey; funding_swap::state::governance::GOVERNANCE_CATEGORIES] {
    core::array::from_fn(|index| {
        if index == GovernanceCategory::AuthorityAdministration.index() {
            authority
        } else {
            Pubkey::new_unique()
        }
    })
}

pub fn initialize_governance_instruction(
    authority: Pubkey,
    args: GovernanceInitializationArgs,
) -> Instruction {
    Instruction {
        program_id: FUNDING_SWAP_PROGRAM_ID,
        accounts: vec![
            AccountMeta::new(authority, true),
            AccountMeta::new_readonly(FUNDING_SWAP_PROGRAM_ID, false),
            AccountMeta::new_readonly(program_data_address(), false),
            AccountMeta::new(governance_address(), false),
            AccountMeta::new_readonly(system_program::ID, false),
        ],
        data: instruction::InitializeGovernance { args }.data(),
    }
}

pub fn update_authorities_instruction(
    authority: Pubkey,
    args: funding_swap::state::UpdateAuthoritiesArgs,
) -> Instruction {
    Instruction {
        program_id: FUNDING_SWAP_PROGRAM_ID,
        accounts: vec![
            AccountMeta::new(authority, true),
            AccountMeta::new(governance_address(), false),
        ],
        data: instruction::UpdateAuthorities { args }.data(),
    }
}

/// Sends a transaction with the payer and any extra instruction signers.
pub fn send_tx(
    svm: &mut LiteSVM,
    payer: &Keypair,
    instructions: Vec<Instruction>,
    extra_signers: &[&Keypair],
) -> TransactionResult {
    let mut signers: Vec<&Keypair> = Vec::with_capacity(1 + extra_signers.len());
    signers.push(payer);
    signers.extend_from_slice(extra_signers);

    let transaction = Transaction::new_signed_with_payer(
        &instructions,
        Some(&payer.pubkey()),
        &signers,
        svm.latest_blockhash(),
    );

    svm.send_transaction(transaction)
}

pub fn airdrop(svm: &mut LiteSVM, key: &Keypair) {
    svm.airdrop(&key.pubkey(), 1_000_000_000)
        .expect("airdrop to test key");
}

pub fn airdrop_keypair(svm: &mut LiteSVM) -> Keypair {
    let key = Keypair::new();

    airdrop(svm, &key);

    key
}

pub fn setup_upgrade_authority(svm: &mut LiteSVM) -> Result<Keypair> {
    let authority = airdrop_keypair(svm);

    set_program_upgrade_authority(svm, authority.pubkey())?;

    Ok(authority)
}

pub fn setup_governance(svm: &mut LiteSVM) -> Result<(Keypair, Pubkey)> {
    let authority = Keypair::new();

    airdrop(svm, &authority);
    set_program_upgrade_authority(svm, authority.pubkey())?;

    let governance = governance_address();

    let args = GovernanceInitializationArgs {
        authorities: default_authorities(authority.pubkey()),
        subaccount_creation_fee_lamports: 0,
        subaccount_creation_fee_destination: Pubkey::new_unique(),
    };

    let ix = initialize_governance_instruction(authority.pubkey(), args);

    assert_transaction_success(send_tx(svm, &authority, vec![ix], &[]));

    Ok((authority, governance))
}

pub fn authority_update(category: GovernanceCategory, authority: Pubkey) -> AuthorityUpdate {
    AuthorityUpdate {
        category: category.as_u8(),
        authority,
    }
}

pub fn assert_governance_initialized(svm: &LiteSVM, expected: &GovernanceInitializationArgs) {
    let governance = read_governance(svm, governance_address()).expect("read governance");

    let (_, expected_bump) =
        Pubkey::find_program_address(&[b"governance"], &FUNDING_SWAP_PROGRAM_ID);

    assert_eq!(governance.authorities, expected.authorities);
    assert_eq!(
        governance.subaccount_creation_fee_lamports,
        expected.subaccount_creation_fee_lamports
    );
    assert_eq!(
        governance.subaccount_creation_fee_destination,
        expected.subaccount_creation_fee_destination
    );
    assert_eq!(governance.next_market_id, 1);
    assert_eq!(governance.bump, expected_bump);
}

pub fn assert_governance_updated(
    svm: &LiteSVM,
    governance: Pubkey,
    mut expected: GlobalGovernanceConfig,
    args: &funding_swap::state::UpdateAuthoritiesArgs,
) {
    if let Some(update) = args.authority_update {
        let category =
            GovernanceCategory::try_from(update.category).expect("valid governance category");

        expected.authorities[category.index()] = update.authority;
    }

    if let Some(value) = args.subaccount_creation_fee_lamports {
        expected.subaccount_creation_fee_lamports = value;
    }

    if let Some(value) = args.subaccount_creation_fee_destination {
        expected.subaccount_creation_fee_destination = value;
    }

    let actual = read_governance(svm, governance).expect("read governance");

    assert_eq!(actual, expected);
}

pub fn send_without_signer(
    svm: &mut LiteSVM,
    payer: &Keypair,
    mut instruction: Instruction,
    signer_index: usize,
) -> TransactionResult {
    instruction.accounts[signer_index].is_signer = false;

    send_tx(svm, payer, vec![instruction], &[])
}

#[track_caller]
pub fn assert_account_not_signer(result: TransactionResult, instruction_index: u8) {
    assert_program_error(result, instruction_index, InstructionError::Custom(3010));
}

#[track_caller]
pub fn assert_instruction_error(
    result: TransactionResult,
    instruction_index: u8,
    expected_code: u32,
) {
    assert_transaction_error(
        result,
        TransactionError::InstructionError(
            instruction_index,
            InstructionError::Custom(expected_code),
        ),
    );
}

#[track_caller]
pub fn assert_program_error(
    result: TransactionResult,
    instruction_index: u8,
    expected: InstructionError,
) {
    assert_transaction_error(
        result,
        TransactionError::InstructionError(instruction_index, expected),
    );
}

#[track_caller]
pub fn assert_transaction_error(result: TransactionResult, expected: TransactionError) {
    match result {
        Ok(metadata) => panic!(
            "expected transaction failure {expected:?}, but it succeeded (compute units: {})",
            metadata.compute_units_consumed
        ),
        Err(error) => assert_eq!(error.err, expected),
    }
}

#[track_caller]
pub fn assert_transaction_success(result: TransactionResult) {
    match result {
        Ok(_) => {}
        Err(error) => panic!("expected transaction success, but it failed: {error:?}"),
    }
}
