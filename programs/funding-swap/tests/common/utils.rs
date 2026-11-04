#![allow(dead_code)]
#![allow(clippy::result_large_err)]

use std::path::Path;

use anchor_lang::{system_program, InstructionData};
use anyhow::{anyhow, Context, Result};
use fixed::types::I64F64;
use funding_swap::{
    instruction,
    state::{
        AuthorityUpdate, GlobalGovernanceConfig, GovernanceCategory, MarginGroup,
        UpdateGroupRiskArgs, GOVERNANCE_CATEGORIES, GROUP_PENDING,
    },
    CreateMarginGroupArgs, GovernanceInitializationArgs, ID as FUNDING_SWAP_PROGRAM_ID,
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

pub struct GovernanceSetup {
    pub governance: Pubkey,
    pub authorities: [Keypair; GOVERNANCE_CATEGORIES],
}

impl GovernanceSetup {
    pub fn authority(&self, category: GovernanceCategory) -> &Keypair {
        &self.authorities[category.index()]
    }

    pub fn authority_key(&self, category: GovernanceCategory) -> Pubkey {
        self.authority(category).pubkey()
    }
}

pub fn setup_governance(svm: &mut LiteSVM) -> Result<GovernanceSetup> {
    let upgrade_authority = airdrop_keypair(svm);

    set_program_upgrade_authority(svm, upgrade_authority.pubkey())?;

    let mut authorities: [Keypair; GOVERNANCE_CATEGORIES] =
        core::array::from_fn(|_| airdrop_keypair(svm));

    authorities[GovernanceCategory::AuthorityAdministration.index()] = upgrade_authority;

    let governance = governance_address();
    let initializer = &authorities[GovernanceCategory::AuthorityAdministration.index()];

    let args = GovernanceInitializationArgs {
        authorities: core::array::from_fn(|index| authorities[index].pubkey()),
        subaccount_creation_fee_lamports: 0,
        subaccount_creation_fee_destination: Pubkey::new_unique(),
    };

    let ix = initialize_governance_instruction(initializer.pubkey(), args);

    assert_transaction_success(send_tx(svm, initializer, vec![ix], &[]));

    Ok(GovernanceSetup {
        governance,
        authorities,
    })
}

pub fn authority_update(category: GovernanceCategory, authority: Pubkey) -> AuthorityUpdate {
    AuthorityUpdate {
        category: category.as_u8(),
        authority,
    }
}

pub fn fixed_raw(numerator: i128, denominator: i128) -> i128 {
    assert!(denominator > 0, "denominator must be positive");

    (numerator << I64F64::FRAC_NBITS) / denominator
}

pub fn margin_group_address(group_id: u16) -> Pubkey {
    Pubkey::find_program_address(
        &[b"margin_group", &group_id.to_le_bytes()],
        &FUNDING_SWAP_PROGRAM_ID,
    )
    .0
}

pub fn read_margin_group(svm: &LiteSVM, margin_group: Pubkey) -> Result<MarginGroup> {
    let account = svm
        .get_account(&margin_group)
        .ok_or_else(|| anyhow!("margin group account not found"))?;

    let data = account
        .data
        .get(8..)
        .ok_or_else(|| anyhow!("margin group account is smaller than its discriminator"))?;

    Ok(bytemuck::pod_read_unaligned::<MarginGroup>(data))
}

pub fn create_margin_group_args(
    group_id: u16,
    critical_health_ratio_raw: i128,
    risky_order_cancellation_ratio_raw: i128,
) -> CreateMarginGroupArgs {
    CreateMarginGroupArgs {
        group_id,
        critical_health_ratio_raw,
        risky_order_cancellation_ratio_raw,
    }
}

pub fn create_margin_group_instruction(
    authority: Pubkey,
    args: CreateMarginGroupArgs,
) -> Instruction {
    Instruction {
        program_id: FUNDING_SWAP_PROGRAM_ID,
        accounts: vec![
            AccountMeta::new(authority, true),
            AccountMeta::new_readonly(governance_address(), false),
            AccountMeta::new(margin_group_address(args.group_id), false),
            AccountMeta::new_readonly(system_program::ID, false),
        ],
        data: instruction::CreateMarginGroup { args }.data(),
    }
}

fn margin_group_action_instruction(
    authority: Pubkey,
    margin_group: Pubkey,
    data: Vec<u8>,
) -> Instruction {
    Instruction {
        program_id: FUNDING_SWAP_PROGRAM_ID,
        accounts: vec![
            AccountMeta::new_readonly(authority, true),
            AccountMeta::new_readonly(governance_address(), false),
            AccountMeta::new(margin_group, false),
        ],
        data,
    }
}

pub fn activate_margin_group_instruction(authority: Pubkey, margin_group: Pubkey) -> Instruction {
    margin_group_action_instruction(
        authority,
        margin_group,
        instruction::ActivateMarginGroup {}.data(),
    )
}

pub fn pause_margin_group_instruction(authority: Pubkey, margin_group: Pubkey) -> Instruction {
    margin_group_action_instruction(
        authority,
        margin_group,
        instruction::PauseMarginGroup {}.data(),
    )
}

pub fn resume_margin_group_instruction(authority: Pubkey, margin_group: Pubkey) -> Instruction {
    margin_group_action_instruction(
        authority,
        margin_group,
        instruction::ResumeMarginGroup {}.data(),
    )
}

pub fn tighten_group_risk_instruction(
    authority: Pubkey,
    margin_group: Pubkey,
    args: UpdateGroupRiskArgs,
) -> Instruction {
    margin_group_action_instruction(
        authority,
        margin_group,
        instruction::TightenGroupRisk { args }.data(),
    )
}

pub fn relax_group_risk_instruction(
    authority: Pubkey,
    margin_group: Pubkey,
    args: UpdateGroupRiskArgs,
) -> Instruction {
    margin_group_action_instruction(
        authority,
        margin_group,
        instruction::RelaxGroupRisk { args }.data(),
    )
}

pub fn close_margin_group_instruction(
    authority: Pubkey,
    margin_group: Pubkey,
    rent_recipient: Pubkey,
) -> Instruction {
    Instruction {
        program_id: FUNDING_SWAP_PROGRAM_ID,
        accounts: vec![
            AccountMeta::new_readonly(authority, true),
            AccountMeta::new_readonly(governance_address(), false),
            AccountMeta::new(margin_group, false),
            AccountMeta::new(rent_recipient, false),
        ],
        data: instruction::CloseMarginGroup {}.data(),
    }
}

pub fn group_risk_args(
    critical_health_ratio_raw: Option<i128>,
    risky_order_cancellation_ratio_raw: Option<i128>,
) -> UpdateGroupRiskArgs {
    UpdateGroupRiskArgs {
        critical_health_ratio_raw,
        risky_order_cancellation_ratio_raw,
    }
}

pub fn create_margin_group_result(
    svm: &mut LiteSVM,
    setup: &GovernanceSetup,
    args: CreateMarginGroupArgs,
) -> TransactionResult {
    let authority = setup.authority(GovernanceCategory::Resources);
    let ix = create_margin_group_instruction(authority.pubkey(), args);

    send_tx(svm, authority, vec![ix], &[])
}

pub fn create_margin_group(
    svm: &mut LiteSVM,
    setup: &GovernanceSetup,
    group_id: u16,
    critical_health_ratio_raw: i128,
    risky_order_cancellation_ratio_raw: i128,
) -> Result<Pubkey> {
    let args = create_margin_group_args(
        group_id,
        critical_health_ratio_raw,
        risky_order_cancellation_ratio_raw,
    );

    assert_transaction_success(create_margin_group_result(svm, setup, args));

    Ok(margin_group_address(group_id))
}

pub fn activate_margin_group(
    svm: &mut LiteSVM,
    setup: &GovernanceSetup,
    margin_group: Pubkey,
) -> Result<()> {
    let authority = setup.authority(GovernanceCategory::Resources);
    let ix = activate_margin_group_instruction(authority.pubkey(), margin_group);

    assert_transaction_success(send_tx(svm, authority, vec![ix], &[]));

    Ok(())
}

pub fn pause_margin_group(
    svm: &mut LiteSVM,
    setup: &GovernanceSetup,
    margin_group: Pubkey,
) -> Result<()> {
    let authority = setup.authority(GovernanceCategory::Emergency);
    let ix = pause_margin_group_instruction(authority.pubkey(), margin_group);

    assert_transaction_success(send_tx(svm, authority, vec![ix], &[]));

    Ok(())
}

pub fn resume_margin_group(
    svm: &mut LiteSVM,
    setup: &GovernanceSetup,
    margin_group: Pubkey,
) -> Result<()> {
    let authority = setup.authority(GovernanceCategory::RiskRecovery);
    let ix = resume_margin_group_instruction(authority.pubkey(), margin_group);

    assert_transaction_success(send_tx(svm, authority, vec![ix], &[]));

    Ok(())
}

pub fn setup_active_margin_group(
    svm: &mut LiteSVM,
    setup: &GovernanceSetup,
    group_id: u16,
    critical_health_ratio_raw: i128,
    risky_order_cancellation_ratio_raw: i128,
) -> Result<Pubkey> {
    let margin_group = create_margin_group(
        svm,
        setup,
        group_id,
        critical_health_ratio_raw,
        risky_order_cancellation_ratio_raw,
    )?;

    activate_margin_group(svm, setup, margin_group)?;

    Ok(margin_group)
}

pub fn set_margin_group_counts(
    svm: &mut LiteSVM,
    margin_group: Pubkey,
    market_count: u32,
    subaccount_count: u32,
) -> Result<()> {
    let mut account = svm
        .get_account(&margin_group)
        .ok_or_else(|| anyhow!("margin group account not found"))?;

    let mut data = read_margin_group(svm, margin_group)?;

    data.market_count = market_count;
    data.subaccount_count = subaccount_count;

    account
        .data
        .get_mut(8..)
        .ok_or_else(|| anyhow!("margin group account is smaller than its discriminator"))?
        .copy_from_slice(bytemuck::bytes_of(&data));

    svm.set_account(margin_group, account)
        .context("patch margin group counts")?;

    Ok(())
}

pub fn assert_margin_group_status(svm: &LiteSVM, margin_group: Pubkey, expected_status: u8) {
    let margin_group = read_margin_group(svm, margin_group).expect("read margin group");

    assert_eq!(margin_group.status, expected_status);
}

pub fn assert_margin_group_health_ratios(
    svm: &LiteSVM,
    margin_group: Pubkey,
    expected_critical_health_ratio_raw: i128,
    expected_risky_order_cancellation_ratio_raw: i128,
) {
    let margin_group = read_margin_group(svm, margin_group).expect("read margin group");

    let critical_health_ratio = margin_group.critical_health_ratio.to_raw_bits();
    let risky_order_cancellation_ratio = margin_group.risky_order_cancellation_ratio.to_raw_bits();

    assert_eq!(critical_health_ratio, expected_critical_health_ratio_raw);
    assert_eq!(
        risky_order_cancellation_ratio,
        expected_risky_order_cancellation_ratio_raw
    );
    assert!(critical_health_ratio > 0);
    assert!(risky_order_cancellation_ratio >= critical_health_ratio);
}

pub fn assert_margin_group_counts(
    svm: &LiteSVM,
    margin_group: Pubkey,
    expected_market_count: u32,
    expected_subaccount_count: u32,
) {
    let margin_group = read_margin_group(svm, margin_group).expect("read margin group");

    assert_eq!(margin_group.market_count, expected_market_count);
    assert_eq!(margin_group.subaccount_count, expected_subaccount_count);
}

pub fn assert_margin_group_created(
    svm: &LiteSVM,
    margin_group: Pubkey,
    group_id: u16,
    critical_health_ratio_raw: i128,
    risky_order_cancellation_ratio_raw: i128,
) {
    assert_margin_group_status(svm, margin_group, GROUP_PENDING);
    assert_margin_group_health_ratios(
        svm,
        margin_group,
        critical_health_ratio_raw,
        risky_order_cancellation_ratio_raw,
    );
    assert_margin_group_counts(svm, margin_group, 0, 0);

    assert_eq!(
        read_margin_group(svm, margin_group)
            .expect("read margin group")
            .group_id,
        group_id
    );
}

pub fn assert_margin_group_closed(svm: &LiteSVM, margin_group: Pubkey) {
    if let Some(account) = svm.get_account(&margin_group) {
        assert_eq!(account.lamports, 0);
        assert!(account.data.is_empty());
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
    assert_program_error(
        result,
        instruction_index,
        InstructionError::Custom(u32::from(anchor_lang::error::ErrorCode::AccountNotSigner)),
    );
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
