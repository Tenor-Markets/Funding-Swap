#![allow(dead_code)]
#![allow(clippy::result_large_err)]

use std::mem::offset_of;
use std::path::Path;

use anchor_lang::solana_program::program_option::COption;
use anchor_lang::solana_program::program_pack::Pack;
use anchor_lang::{system_program, InstructionData};
use anchor_spl::token::spl_token::state::{Account as SplTokenAccount, AccountState};
use anyhow::{anyhow, Context, Result};
use fixed::types::I64F64;
pub use funding_swap::state::{usdc_asset_address, ASSET_PENDING, USDC_MINT, USDC_USD_FEED_ID};
use funding_swap::{
    instruction,
    state::market::MARKET_U128_PADDING,
    state::{
        AssetState, AuthorityUpdate, FixedPoint, GlobalGovernanceConfig, GovernanceCategory,
        MarginGroup, MarketConfig, MarketState, UpdateFeesAndLiquidationArgs, UpdateGroupRiskArgs,
        UpdateMarketRiskArgs, UpdateWithdrawalLimitsArgs, GOVERNANCE_CATEGORIES, GROUP_PENDING,
        MARKET_PENDING,
    },
    CreateAssetArgs, CreateMarginGroupArgs, CreateMarketArgs, GovernanceInitializationArgs,
    MarketAdapterArgs, ID as FUNDING_SWAP_PROGRAM_ID,
};
use funding_swap_adapters::{
    jupiter::JUPITER_PROGRAM_ID,
    phoenix::{PHOENIX_GLOBAL_CONFIGURATION, PHOENIX_PERP_ASSET_MAP, PHOENIX_PROGRAM_ID},
};
use litesvm::{types::TransactionResult, LiteSVM};
use solana_sdk::{
    account::Account,
    clock::Clock,
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

pub const JUPITER_SOL_CUSTODY: Pubkey =
    solana_sdk::pubkey!("7xS2gz2bTp3fwCC7knJvUWTEU9Tycczu6VhJYKgi1wdz");
pub const JUPITER_POOL: Pubkey =
    solana_sdk::pubkey!("5BUwFW4nRbftYTDMbgxykoFWqWHPzahFSNAaaaJtVKsq");
pub const JUPITER_SOL_MINT: Pubkey =
    solana_sdk::pubkey!("So11111111111111111111111111111111111111112");

// Jupiter SOL custody captured from mainnet at slot 436_948_909
// (2026-08-03T09:54:17Z).
pub const JUPITER_FIXTURE_SLOT: u64 = 436_948_909;
pub const JUPITER_FIXTURE_LAST_UPDATE: i64 = 1_785_750_857;
pub const JUPITER_FIXTURE_CHECKPOINT: u128 = 1_044_371_865;

pub fn jupiter_sol_custody_fixture() -> Vec<u8> {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;

    STANDARD
        .decode(include_str!("../fixtures/jupiter/sol.base64").trim())
        .expect("decode Jupiter SOL fixture")
}

// Phoenix/Rise global configuration and perp asset map captured from mainnet
// at slot 436_979_123 (1_785_763_629, 2026-08-03T13:27:09Z).
pub const PHOENIX_FIXTURE_SLOT: u64 = 436_979_123;
pub const PHOENIX_FIXTURE_LAST_UPDATE: i64 = 1_785_763_629;
pub const PHOENIX_FIXTURE_CHECKPOINT: u128 = 2_190;

pub const PHOENIX_SOL_MARKET: Pubkey =
    solana_sdk::pubkey!("71Si24E4uc3oCaPbPZTozC1ptSNNqygjjebxSmErSsC2");
pub const PHOENIX_SOL_SYMBOL: [u8; 16] = [b'S', b'O', b'L', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];

pub fn phoenix_global_fixture() -> Vec<u8> {
    include_bytes!("../fixtures/phoenix/global_configuration.bin").to_vec()
}

pub fn phoenix_asset_map_fixture() -> Vec<u8> {
    include_bytes!("../fixtures/phoenix/perp_asset_map.bin").to_vec()
}

pub fn set_phoenix_source_accounts(svm: &mut LiteSVM) -> Result<()> {
    for (key, data) in [
        (PHOENIX_GLOBAL_CONFIGURATION, phoenix_global_fixture()),
        (PHOENIX_PERP_ASSET_MAP, phoenix_asset_map_fixture()),
    ] {
        let lamports = svm.minimum_balance_for_rent_exemption(data.len());

        svm.set_account(
            key,
            Account {
                lamports,
                data,
                owner: PHOENIX_PROGRAM_ID,
                executable: false,
                rent_epoch: 0,
            },
        )
        .context("set Phoenix source fixture")?;
    }

    Ok(())
}

pub fn phoenix_sol_market_adapter() -> MarketAdapterArgs {
    MarketAdapterArgs::Phoenix {
        max_abs_native_delta: 10_000,
        global_configuration: PHOENIX_GLOBAL_CONFIGURATION,
        perp_asset_map: PHOENIX_PERP_ASSET_MAP,
        market_account: PHOENIX_SOL_MARKET,
        tick_size: 100,
        funding_interval_seconds: 3_600,
        funding_period_seconds: 86_400,
        max_funding_age_seconds: 3_600,
        max_mark_age_slots: 100,
        symbol: PHOENIX_SOL_SYMBOL,
        asset_slot: 0,
        asset_id: 0,
        base_lot_decimals: 2,
    }
}

pub fn phoenix_sol_market_args() -> CreateMarketArgs {
    let mut args = default_market_args_with_adapter(phoenix_sol_market_adapter());

    args.underlying_decimals = 9;
    args.tick_scale = 9;
    args.start_timestamp = PHOENIX_FIXTURE_LAST_UPDATE + 3_600;
    args.maturity_timestamp = args.start_timestamp + 31_536_000;

    args
}

pub fn phoenix_seed_market_source_instruction(market: Pubkey) -> Instruction {
    Instruction {
        program_id: FUNDING_SWAP_PROGRAM_ID,
        accounts: vec![
            AccountMeta::new(market, false),
            AccountMeta::new_readonly(PHOENIX_GLOBAL_CONFIGURATION, false),
            AccountMeta::new_readonly(PHOENIX_PERP_ASSET_MAP, false),
        ],
        data: instruction::SeedMarketSource {}.data(),
    }
}

pub fn create_uninitialized_market_account(svm: &mut LiteSVM) -> Result<Pubkey> {
    let market = Pubkey::new_unique();
    let data = vec![0u8; MarketState::SPACE];
    let lamports = svm.minimum_balance_for_rent_exemption(data.len());

    svm.set_account(
        market,
        Account {
            lamports,
            data,
            owner: FUNDING_SWAP_PROGRAM_ID,
            executable: false,
            rent_epoch: 0,
        },
    )
    .context("create zeroed market account")?;

    Ok(market)
}

pub fn read_market_config(svm: &LiteSVM, market: Pubkey) -> Result<MarketConfig> {
    let account = svm
        .get_account(&market)
        .ok_or_else(|| anyhow!("market account not found"))?;

    let start = 8 + offset_of!(MarketState, config);
    let end = start + core::mem::size_of::<MarketConfig>();

    Ok(bytemuck::pod_read_unaligned::<MarketConfig>(
        &account.data[start..end],
    ))
}

/// `offset_of!` reflects the host's 16-byte `u128` alignment. `MarketState`
/// carries host-only padding before its first post-timestamp `u128`, so every
/// field after that point is `MARKET_U128_PADDING` bytes later on the host.
fn persisted_tail_offset(field_offset: usize) -> usize {
    field_offset - MARKET_U128_PADDING
}

pub fn read_market_status(svm: &LiteSVM, market: Pubkey) -> Result<u8> {
    let account = svm
        .get_account(&market)
        .ok_or_else(|| anyhow!("market account not found"))?;

    let offset = 8 + persisted_tail_offset(offset_of!(MarketState, status));

    Ok(account.data[offset])
}

pub fn read_market_source(svm: &LiteSVM, market: Pubkey) -> Result<(u128, i64, bool)> {
    let account = svm
        .get_account(&market)
        .ok_or_else(|| anyhow!("market account not found"))?;

    let checkpoint_offset = 8 + offset_of!(MarketState, source_native_checkpoint);
    let timestamp_offset = 8 + offset_of!(MarketState, source_last_update_timestamp);
    let seeded_offset = 8 + persisted_tail_offset(offset_of!(MarketState, source_seeded));

    let checkpoint = u128::from_le_bytes(
        account.data[checkpoint_offset..checkpoint_offset + 16]
            .try_into()
            .expect("checkpoint bytes"),
    );
    let timestamp = i64::from_le_bytes(
        account.data[timestamp_offset..timestamp_offset + 8]
            .try_into()
            .expect("source timestamp bytes"),
    );

    Ok((checkpoint, timestamp, account.data[seeded_offset] == 1))
}

pub fn set_market_status(svm: &mut LiteSVM, market: Pubkey, status: u8) -> Result<()> {
    let mut account = svm
        .get_account(&market)
        .ok_or_else(|| anyhow!("market account not found"))?;

    let offset = 8 + persisted_tail_offset(offset_of!(MarketState, status));

    account.data[offset] = status;

    svm.set_account(market, account)
        .context("patch market status")?;

    Ok(())
}

pub fn set_market_source_seeded(svm: &mut LiteSVM, market: Pubkey, seeded: bool) -> Result<()> {
    let mut account = svm
        .get_account(&market)
        .ok_or_else(|| anyhow!("market account not found"))?;

    let offset = 8 + persisted_tail_offset(offset_of!(MarketState, source_seeded));

    account.data[offset] = u8::from(seeded);

    svm.set_account(market, account)
        .context("patch market source seeded flag")?;

    Ok(())
}

pub fn set_clock(svm: &mut LiteSVM, unix_timestamp: i64, slot: u64) {
    svm.set_sysvar(&Clock {
        slot,
        unix_timestamp,
        ..Clock::default()
    });
}

pub fn jupiter_market_adapter() -> MarketAdapterArgs {
    MarketAdapterArgs::Jupiter {
        max_source_delta: 1_000_000_000_000,
        custody: Pubkey::new_unique(),
        pool: Pubkey::new_unique(),
        mint: Pubkey::new_unique(),
        max_source_age_seconds: 3_600,
    }
}

pub fn jupiter_sol_market_adapter() -> MarketAdapterArgs {
    MarketAdapterArgs::Jupiter {
        max_source_delta: 1_000_000_000_000,
        custody: JUPITER_SOL_CUSTODY,
        pool: JUPITER_POOL,
        mint: JUPITER_SOL_MINT,
        max_source_age_seconds: 3_600,
    }
}

pub fn default_market_args_with_adapter(adapter: MarketAdapterArgs) -> CreateMarketArgs {
    CreateMarketArgs {
        underlying_decimals: 6,
        underlying_feed_id: [1; 32],
        underlying_max_age_seconds: 60,
        underlying_max_confidence_ratio_raw: fixed_raw(1, 100),
        underlying_max_ema_deviation_raw: fixed_raw(1, 100),
        start_timestamp: 100,
        maturity_timestamp: 10_000,
        hard_gross_open_interest_cap_atoms: 1_000_000_000,
        mark_observation_window_seconds: 60,
        margin_time_threshold_seconds: 60,
        tick_scale: 6,
        tick_step: 1,
        minimum_order_size_atoms: 1,
        maximum_order_size_atoms: 1_000_000_000,
        maximum_open_orders_per_subaccount: 32,
        rate_floor_raw: fixed_raw(1, 100),
        initial_margin_factor_raw: fixed_raw(1, 5),
        maintenance_margin_factor_raw: fixed_raw(1, 10),
        order_rate_bound_factor_raw: fixed_raw(100, 1),
        closing_margin_relief_bound_factor_raw: fixed_raw(1, 4),
        trade_mark_update_bound_factor_raw: fixed_raw(100, 1),
        taker_fee_rate_raw: fixed_raw(1, 1_000),
        annualized_settlement_fee_rate_raw: fixed_raw(1, 100),
        liquidation_base_incentive_raw: fixed_raw(1, 100),
        liquidation_incentive_slope_raw: fixed_raw(1, 100),
        liquidation_fee_rate_raw: fixed_raw(1, 100),
        adapter,
    }
}

pub fn default_market_args() -> CreateMarketArgs {
    default_market_args_with_adapter(jupiter_market_adapter())
}

pub fn jupiter_sol_market_args() -> CreateMarketArgs {
    let mut args = default_market_args_with_adapter(jupiter_sol_market_adapter());

    args.underlying_decimals = 9;
    args.tick_scale = 9;
    args.start_timestamp = JUPITER_FIXTURE_LAST_UPDATE + 3_600;
    args.maturity_timestamp = args.start_timestamp + 31_536_000;

    args
}

pub fn initialize_market_instruction(
    authority: Pubkey,
    market: Pubkey,
    args: CreateMarketArgs,
) -> Instruction {
    Instruction {
        program_id: FUNDING_SWAP_PROGRAM_ID,
        accounts: vec![
            AccountMeta::new_readonly(authority, true),
            AccountMeta::new(governance_address(), false),
            AccountMeta::new(market, false),
        ],
        data: instruction::InitializeMarket { args }.data(),
    }
}

pub fn send_initialize_market(
    svm: &mut LiteSVM,
    setup: &GovernanceSetup,
    market: Pubkey,
    args: CreateMarketArgs,
) -> TransactionResult {
    let authority = setup.authority(GovernanceCategory::Resources);
    let ix = initialize_market_instruction(authority.pubkey(), market, args);

    send_tx(svm, authority, vec![ix], &[])
}

pub fn initialize_market(
    svm: &mut LiteSVM,
    setup: &GovernanceSetup,
    args: CreateMarketArgs,
) -> Result<Pubkey> {
    let market = create_uninitialized_market_account(svm)?;

    assert_transaction_success(send_initialize_market(svm, setup, market, args));

    Ok(market)
}

pub fn activate_market_instruction(authority: Pubkey, market: Pubkey) -> Instruction {
    Instruction {
        program_id: FUNDING_SWAP_PROGRAM_ID,
        accounts: vec![
            AccountMeta::new_readonly(authority, true),
            AccountMeta::new_readonly(governance_address(), false),
            AccountMeta::new(market, false),
        ],
        data: instruction::ActivateMarket {}.data(),
    }
}

pub fn activate_market(svm: &mut LiteSVM, setup: &GovernanceSetup, market: Pubkey) -> Result<()> {
    let authority = setup.authority(GovernanceCategory::Resources);
    let ix = activate_market_instruction(authority.pubkey(), market);

    assert_transaction_success(send_tx(svm, authority, vec![ix], &[]));

    Ok(())
}

pub fn setup_active_market(
    svm: &mut LiteSVM,
    setup: &GovernanceSetup,
    args: CreateMarketArgs,
) -> Result<Pubkey> {
    let market = initialize_market(svm, setup, args)?;

    set_market_source_seeded(svm, market, true)?;
    activate_market(svm, setup, market)?;

    Ok(market)
}

pub fn market_group_membership_address(margin_group: Pubkey, market: Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[
            b"market_group_membership",
            margin_group.as_ref(),
            market.as_ref(),
        ],
        &FUNDING_SWAP_PROGRAM_ID,
    )
    .0
}

pub fn activate_market_membership_instruction(
    authority: Pubkey,
    margin_group: Pubkey,
    market: Pubkey,
    membership: Pubkey,
) -> Instruction {
    Instruction {
        program_id: FUNDING_SWAP_PROGRAM_ID,
        accounts: vec![
            AccountMeta::new(authority, true),
            AccountMeta::new_readonly(governance_address(), false),
            AccountMeta::new(margin_group, false),
            AccountMeta::new_readonly(market, false),
            AccountMeta::new(membership, false),
            AccountMeta::new_readonly(system_program::ID, false),
        ],
        data: instruction::ActivateMarketGroupMembership {}.data(),
    }
}

pub fn activate_market_membership(
    svm: &mut LiteSVM,
    setup: &GovernanceSetup,
    margin_group: Pubkey,
    market: Pubkey,
) -> Result<Pubkey> {
    let authority = setup.authority(GovernanceCategory::Resources);
    let membership = market_group_membership_address(margin_group, market);
    let ix = activate_market_membership_instruction(
        authority.pubkey(),
        margin_group,
        market,
        membership,
    );

    assert_transaction_success(send_tx(svm, authority, vec![ix], &[]));

    Ok(membership)
}

fn market_governance_instruction(authority: Pubkey, market: Pubkey, data: Vec<u8>) -> Instruction {
    Instruction {
        program_id: FUNDING_SWAP_PROGRAM_ID,
        accounts: vec![
            AccountMeta::new_readonly(authority, true),
            AccountMeta::new_readonly(governance_address(), false),
            AccountMeta::new(market, false),
        ],
        data,
    }
}

pub fn tighten_market_risk_instruction(
    authority: Pubkey,
    market: Pubkey,
    args: UpdateMarketRiskArgs,
) -> Instruction {
    market_governance_instruction(
        authority,
        market,
        instruction::TightenMarketRisk { args }.data(),
    )
}

pub fn relax_market_risk_instruction(
    authority: Pubkey,
    market: Pubkey,
    args: UpdateMarketRiskArgs,
) -> Instruction {
    market_governance_instruction(
        authority,
        market,
        instruction::RelaxMarketRisk { args }.data(),
    )
}

pub fn update_fees_and_liquidation_instruction(
    authority: Pubkey,
    market: Pubkey,
    args: UpdateFeesAndLiquidationArgs,
) -> Instruction {
    market_governance_instruction(
        authority,
        market,
        instruction::UpdateFeesAndLiquidation { args }.data(),
    )
}

pub fn restrict_market_status_instruction(
    authority: Pubkey,
    market: Pubkey,
    status: u8,
) -> Instruction {
    market_governance_instruction(
        authority,
        market,
        instruction::RestrictMarketStatus { status }.data(),
    )
}

pub fn restore_market_status_instruction(
    authority: Pubkey,
    market: Pubkey,
    status: u8,
) -> Instruction {
    market_governance_instruction(
        authority,
        market,
        instruction::RestoreMarketStatus { status }.data(),
    )
}

pub fn jupiter_seed_market_source_instruction(market: Pubkey) -> Instruction {
    Instruction {
        program_id: FUNDING_SWAP_PROGRAM_ID,
        accounts: vec![
            AccountMeta::new(market, false),
            AccountMeta::new_readonly(JUPITER_SOL_CUSTODY, false),
        ],
        data: instruction::SeedMarketSource {}.data(),
    }
}

pub fn set_jupiter_sol_custody_account(svm: &mut LiteSVM) -> Result<()> {
    let data = jupiter_sol_custody_fixture();
    let lamports = svm.minimum_balance_for_rent_exemption(data.len());

    svm.set_account(
        JUPITER_SOL_CUSTODY,
        Account {
            lamports,
            data,
            owner: JUPITER_PROGRAM_ID,
            executable: false,
            rent_epoch: 0,
        },
    )
    .context("set Jupiter SOL custody fixture")?;

    Ok(())
}

pub fn assert_market_status(svm: &LiteSVM, market: Pubkey, expected_status: u8) {
    assert_eq!(
        read_market_status(svm, market).expect("read market status"),
        expected_status
    );
}

pub fn assert_market_initialized(
    svm: &LiteSVM,
    governance: Pubkey,
    market: Pubkey,
    args: &CreateMarketArgs,
    expected_market_id: u64,
) {
    let config = read_market_config(svm, market).expect("read market config");

    assert_eq!(config.market_id, expected_market_id);
    assert_eq!(config.start_timestamp, args.start_timestamp);
    assert_eq!(config.maturity_timestamp, args.maturity_timestamp);
    assert_eq!(config.underlying_decimals, args.underlying_decimals);
    assert_eq!(config.tick_scale, args.tick_scale);
    assert_eq!(
        config.hard_gross_open_interest_cap_atoms,
        args.hard_gross_open_interest_cap_atoms
    );
    assert_eq!(
        config.mark_observation_window_seconds,
        args.mark_observation_window_seconds
    );
    assert_eq!(
        config.margin_time_threshold_seconds,
        args.margin_time_threshold_seconds
    );
    assert_eq!(config.rate_floor.to_raw_bits(), args.rate_floor_raw);
    assert_eq!(
        config.initial_margin_factor.to_raw_bits(),
        args.initial_margin_factor_raw
    );
    assert_eq!(
        config.maintenance_margin_factor.to_raw_bits(),
        args.maintenance_margin_factor_raw
    );

    assert_market_status(svm, market, MARKET_PENDING);

    let (_, _, seeded) = read_market_source(svm, market).expect("read market source");
    assert!(!seeded);

    let governance = read_governance(svm, governance).expect("read governance");
    assert_eq!(governance.next_market_id, expected_market_id + 1);
}

pub fn assert_market_risk_updated(
    svm: &LiteSVM,
    market: Pubkey,
    mut expected: MarketConfig,
    args: &UpdateMarketRiskArgs,
) {
    if let Some(value) = args.initial_margin_factor_raw {
        expected.initial_margin_factor = FixedPoint::from_raw_bits(value);
    }
    if let Some(value) = args.maintenance_margin_factor_raw {
        expected.maintenance_margin_factor = FixedPoint::from_raw_bits(value);
    }
    if let Some(value) = args.closing_margin_relief_bound_factor_raw {
        expected.closing_margin_relief_bound_factor = FixedPoint::from_raw_bits(value);
    }
    if let Some(value) = args.rate_floor_raw {
        let previous_rate_floor = expected.rate_floor;

        expected.rate_floor = FixedPoint::from_raw_bits(value);

        if expected.rate_floor != previous_rate_floor {
            expected.pm_cache_nonce += 1;
        }
    }
    if let Some(value) = args.hard_gross_open_interest_cap_atoms {
        expected.hard_gross_open_interest_cap_atoms = value;
    }
    if let Some(value) = args.mark_observation_window_seconds {
        expected.mark_observation_window_seconds = value;
    }
    if let Some(value) = args.margin_time_threshold_seconds {
        expected.margin_time_threshold_seconds = value;
    }
    if let Some(value) = args.order_rate_bound_factor_raw {
        expected.order_rate_bound_factor = FixedPoint::from_raw_bits(value);
    }
    if let Some(value) = args.trade_mark_update_bound_factor_raw {
        expected.trade_mark_update_bound_factor = FixedPoint::from_raw_bits(value);
    }

    assert_eq!(
        read_market_config(svm, market).expect("read market config"),
        expected
    );
}

pub fn assert_market_fees_updated(
    svm: &LiteSVM,
    market: Pubkey,
    mut expected: MarketConfig,
    args: &UpdateFeesAndLiquidationArgs,
) {
    if let Some(value) = args.taker_fee_rate_raw {
        expected.taker_fee_rate = FixedPoint::from_raw_bits(value);
    }
    if let Some(value) = args.annualized_settlement_fee_rate_raw {
        expected.annualized_settlement_fee_rate = FixedPoint::from_raw_bits(value);
    }
    if let Some(value) = args.liquidation_base_incentive_raw {
        expected.liquidation_base_incentive = FixedPoint::from_raw_bits(value);
    }
    if let Some(value) = args.liquidation_incentive_slope_raw {
        expected.liquidation_incentive_slope = FixedPoint::from_raw_bits(value);
    }
    if let Some(value) = args.liquidation_fee_rate_raw {
        expected.liquidation_fee_rate = FixedPoint::from_raw_bits(value);
    }

    assert_eq!(
        read_market_config(svm, market).expect("read market config"),
        expected
    );
}

pub fn assert_market_source_seeded(
    svm: &LiteSVM,
    market: Pubkey,
    expected_checkpoint: u128,
    expected_timestamp: i64,
) {
    let (checkpoint, timestamp, seeded) =
        read_market_source(svm, market).expect("read market source");

    assert!(seeded);
    assert_eq!(checkpoint, expected_checkpoint);
    assert_eq!(timestamp, expected_timestamp);
}

pub fn assert_account_exists(svm: &LiteSVM, account: Pubkey) {
    assert!(svm.get_account(&account).is_some());
}

pub fn create_asset_args() -> CreateAssetArgs {
    CreateAssetArgs {
        max_age_seconds: 60,
        max_confidence_ratio_raw: fixed_raw(1, 100),
        max_ema_deviation_raw: fixed_raw(1, 100),
        max_peg_deviation_raw: fixed_raw(1, 100),
        window_durations: [3_600, 86_400, 604_800],
        short_window_limit_atoms: 1_000_000,
        medium_window_limit_atoms: 10_000_000,
        long_window_limit_atoms: 100_000_000,
    }
}

pub fn read_asset_state(svm: &LiteSVM, asset: Pubkey) -> Result<AssetState> {
    let account = svm
        .get_account(&asset)
        .ok_or_else(|| anyhow!("asset account not found"))?;

    let data = account
        .data
        .get(8..)
        .ok_or_else(|| anyhow!("asset account is smaller than its discriminator"))?;

    Ok(bytemuck::pod_read_unaligned::<AssetState>(data))
}

pub fn assert_asset_status(svm: &LiteSVM, asset: Pubkey, expected_status: u8) {
    assert_eq!(
        read_asset_state(svm, asset)
            .expect("read asset state")
            .status,
        expected_status
    );
}

pub fn assert_asset_created(
    svm: &LiteSVM,
    asset: Pubkey,
    token_account: Pubkey,
    args: &CreateAssetArgs,
) {
    let state = read_asset_state(svm, asset).expect("read asset state");

    assert_eq!(state.status, ASSET_PENDING);
    assert_eq!(state.token_account, token_account);
    assert_eq!(state.oracle.feed_id, USDC_USD_FEED_ID);
    assert_eq!(state.oracle.max_age_seconds, args.max_age_seconds);
    assert_eq!(
        state.oracle.max_confidence_ratio.to_raw_bits(),
        args.max_confidence_ratio_raw
    );
    assert_eq!(
        state.oracle.max_ema_deviation.to_raw_bits(),
        args.max_ema_deviation_raw
    );
    assert_eq!(
        state.max_peg_deviation.to_raw_bits(),
        args.max_peg_deviation_raw
    );
    assert_eq!(
        state.short_window.duration_seconds,
        args.window_durations[0]
    );
    assert_eq!(
        state.medium_window.duration_seconds,
        args.window_durations[1]
    );
    assert_eq!(state.long_window.duration_seconds, args.window_durations[2]);
    assert_eq!(
        state.short_window.limit_atoms,
        args.short_window_limit_atoms
    );
    assert_eq!(
        state.medium_window.limit_atoms,
        args.medium_window_limit_atoms
    );
    assert_eq!(state.long_window.limit_atoms, args.long_window_limit_atoms);
}

pub fn assert_asset_withdrawal_limits(
    svm: &LiteSVM,
    asset: Pubkey,
    short_limit_atoms: u128,
    medium_limit_atoms: u128,
    long_limit_atoms: u128,
) {
    let state = read_asset_state(svm, asset).expect("read asset state");

    assert_eq!(state.short_window.limit_atoms, short_limit_atoms);
    assert_eq!(state.medium_window.limit_atoms, medium_limit_atoms);
    assert_eq!(state.long_window.limit_atoms, long_limit_atoms);
}

fn option_to_coption<T>(option: Option<T>) -> COption<T> {
    option.map_or(COption::None, COption::Some)
}

pub fn create_token_account_for_mint(
    svm: &mut LiteSVM,
    mint: Pubkey,
    owner: Pubkey,
    delegate: Option<Pubkey>,
    close_authority: Option<Pubkey>,
) -> Result<Pubkey> {
    let key = Pubkey::new_unique();
    let token = SplTokenAccount {
        mint,
        owner,
        amount: 0,
        delegate: option_to_coption(delegate),
        state: AccountState::Initialized,
        is_native: COption::None,
        delegated_amount: 0,
        close_authority: option_to_coption(close_authority),
    };

    let mut data = vec![0u8; SplTokenAccount::LEN];
    SplTokenAccount::pack(token, &mut data).context("pack token account")?;

    let lamports = svm.minimum_balance_for_rent_exemption(data.len());

    svm.set_account(
        key,
        Account {
            lamports,
            data,
            owner: anchor_spl::token::spl_token::ID,
            executable: false,
            rent_epoch: 0,
        },
    )
    .context("set token account")?;

    Ok(key)
}

pub fn create_usdc_token_account(svm: &mut LiteSVM, owner: Pubkey) -> Result<Pubkey> {
    create_token_account_for_mint(svm, USDC_MINT, owner, None, None)
}

pub fn create_asset_instruction(
    authority: Pubkey,
    token_account: Pubkey,
    args: CreateAssetArgs,
) -> Instruction {
    Instruction {
        program_id: FUNDING_SWAP_PROGRAM_ID,
        accounts: vec![
            AccountMeta::new(authority, true),
            AccountMeta::new_readonly(governance_address(), false),
            AccountMeta::new(usdc_asset_address(), false),
            AccountMeta::new_readonly(token_account, false),
            AccountMeta::new_readonly(system_program::ID, false),
        ],
        data: instruction::CreateAsset { args }.data(),
    }
}

pub fn send_create_asset(
    svm: &mut LiteSVM,
    setup: &GovernanceSetup,
    token_account: Pubkey,
    args: CreateAssetArgs,
) -> TransactionResult {
    let authority = setup.authority(GovernanceCategory::Resources);
    let ix = create_asset_instruction(authority.pubkey(), token_account, args);

    send_tx(svm, authority, vec![ix], &[])
}

pub fn create_asset(
    svm: &mut LiteSVM,
    setup: &GovernanceSetup,
    args: CreateAssetArgs,
) -> Result<Pubkey> {
    let asset = usdc_asset_address();
    let token_account = create_usdc_token_account(svm, asset)?;

    assert_transaction_success(send_create_asset(svm, setup, token_account, args));

    Ok(asset)
}

pub fn activate_asset_instruction(authority: Pubkey, asset: Pubkey) -> Instruction {
    Instruction {
        program_id: FUNDING_SWAP_PROGRAM_ID,
        accounts: vec![
            AccountMeta::new_readonly(authority, true),
            AccountMeta::new_readonly(governance_address(), false),
            AccountMeta::new(asset, false),
        ],
        data: instruction::ActivateAsset {}.data(),
    }
}

pub fn activate_asset(svm: &mut LiteSVM, setup: &GovernanceSetup, asset: Pubkey) -> Result<()> {
    let authority = setup.authority(GovernanceCategory::Resources);
    let ix = activate_asset_instruction(authority.pubkey(), asset);

    assert_transaction_success(send_tx(svm, authority, vec![ix], &[]));

    Ok(())
}

pub fn setup_active_asset(
    svm: &mut LiteSVM,
    setup: &GovernanceSetup,
) -> Result<(Pubkey, CreateAssetArgs)> {
    let args = create_asset_args();
    let asset = create_asset(svm, setup, args)?;

    activate_asset(svm, setup, asset)?;

    Ok((asset, args))
}

fn withdrawal_limits_instruction(authority: Pubkey, asset: Pubkey, data: Vec<u8>) -> Instruction {
    Instruction {
        program_id: FUNDING_SWAP_PROGRAM_ID,
        accounts: vec![
            AccountMeta::new_readonly(authority, true),
            AccountMeta::new_readonly(governance_address(), false),
            AccountMeta::new(asset, false),
        ],
        data,
    }
}

pub fn tighten_withdrawal_limits_instruction(
    authority: Pubkey,
    asset: Pubkey,
    args: UpdateWithdrawalLimitsArgs,
) -> Instruction {
    withdrawal_limits_instruction(
        authority,
        asset,
        instruction::TightenWithdrawalLimits { args }.data(),
    )
}

pub fn raise_withdrawal_limits_instruction(
    authority: Pubkey,
    asset: Pubkey,
    args: UpdateWithdrawalLimitsArgs,
) -> Instruction {
    withdrawal_limits_instruction(
        authority,
        asset,
        instruction::RaiseWithdrawalLimits { args }.data(),
    )
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
