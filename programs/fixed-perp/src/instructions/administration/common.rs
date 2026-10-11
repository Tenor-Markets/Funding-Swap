use crate::state::*;
use anchor_lang::prelude::*;

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub enum MarketAdapterArgs {
    Jupiter {
        max_source_delta: u128,
        custody: Pubkey,
        pool: Pubkey,
        mint: Pubkey,
        max_source_age_seconds: u64,
    },
    Phoenix {
        max_abs_native_delta: u128,
        global_configuration: Pubkey,
        perp_asset_map: Pubkey,
        canonical_quote_mint: Pubkey,
        market_account: Pubkey,
        tick_size: u64,
        funding_interval_seconds: u64,
        funding_period_seconds: u64,
        max_funding_age_seconds: u64,
        max_mark_age_slots: u64,
        symbol: [u8; 16],
        asset_slot: u16,
        asset_id: u16,
        base_lot_decimals: u8,
    },
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct CreateMarketArgs {
    pub market_id: u32,
    pub start_timestamp: i64,
    pub maturity_timestamp: i64,
    pub minimum_position_size_atoms: u64,
    pub maximum_position_size_atoms: u64,
    pub hard_gross_open_interest_cap_atoms: u128,
    pub mark_observation_window_seconds: u32,
    pub margin_time_threshold_seconds: u32,
    pub tick_scale: u8,
    pub tick_step: u8,
    pub minimum_order_size_atoms: u64,
    pub maximum_order_size_atoms: u64,
    pub maximum_open_orders_per_subaccount: u8,
    pub rate_floor_raw: i128,
    pub initial_margin_factor_raw: i128,
    pub maintenance_margin_factor_raw: i128,
    pub order_rate_bound_factor_raw: i128,
    pub closing_margin_relief_bound_factor_raw: i128,
    pub trade_mark_update_bound_factor_raw: i128,
    pub taker_fee_rate_raw: i128,
    pub annualized_settlement_fee_rate_raw: i128,
    pub liquidation_base_incentive_raw: i128,
    pub liquidation_incentive_slope_raw: i128,
    pub liquidation_fee_rate_raw: i128,
    pub adapter: MarketAdapterArgs,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, Default)]
pub struct MarketMarginGovernanceArgs {
    pub initial_margin_factor_raw: Option<i128>,
    pub maintenance_margin_factor_raw: Option<i128>,
    pub closing_margin_relief_bound_factor_raw: Option<i128>,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, Default)]
pub struct GroupMarginGovernanceArgs {
    pub critical_health_ratio_raw: Option<i128>,
    pub risky_order_cancellation_ratio_raw: Option<i128>,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, Default)]
pub struct LiquidationGovernanceArgs {
    pub base_incentive_raw: Option<i128>,
    pub incentive_slope_raw: Option<i128>,
    pub fee_rate_raw: Option<i128>,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, Default)]
pub struct FeeRateGovernanceArgs {
    pub taker_fee_rate_raw: Option<i128>,
    pub annualized_settlement_fee_rate_raw: Option<i128>,
    pub rate_floor_raw: Option<i128>,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, Default)]
pub struct MarketRiskGovernanceArgs {
    pub minimum_position_size_atoms: Option<u64>,
    pub maximum_position_size_atoms: Option<u64>,
    pub hard_gross_open_interest_cap_atoms: Option<u128>,
    pub mark_observation_window_seconds: Option<u64>,
    pub margin_time_threshold_seconds: Option<u64>,
    pub order_rate_bound_factor_raw: Option<i128>,
    pub trade_mark_update_bound_factor_raw: Option<i128>,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, Default)]
pub struct MarketStatusGovernanceArgs {
    pub status: Option<u8>,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, Default)]
pub struct CircuitBreakerGovernanceArgs {
    pub short_window_limit_atoms: Option<u128>,
    pub medium_window_limit_atoms: Option<u128>,
    pub long_window_limit_atoms: Option<u128>,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, Default)]
pub struct GlobalGovernanceArgs {
    pub target_category: Option<u8>,
    pub new_authority: Option<Pubkey>,
    pub increase_delay_seconds: Option<u64>,
    pub decrease_delay_seconds: Option<u64>,
    pub creation_delay_seconds: Option<u64>,
    pub proposal_expiry_seconds: Option<u64>,
    pub subaccount_creation_fee_lamports: Option<u64>,
    pub subaccount_creation_fee_destination: Option<Pubkey>,
}

pub(crate) fn optional_fixed(value: Option<i128>) -> OptionalFixedPoint {
    value
        .map(|raw| OptionalFixedPoint::some(FixedPoint::from_raw_bits(raw)))
        .unwrap_or_else(OptionalFixedPoint::none)
}

pub(crate) fn optional_u64(value: Option<u64>) -> OptionalU64 {
    value
        .map(OptionalU64::some)
        .unwrap_or_else(OptionalU64::none)
}

pub(crate) fn optional_u128(value: Option<u128>) -> OptionalU128 {
    value
        .map(OptionalU128::some)
        .unwrap_or_else(OptionalU128::none)
}

pub(crate) fn optional_u8(value: Option<u8>) -> OptionalU8 {
    value.map(OptionalU8::some).unwrap_or_else(OptionalU8::none)
}

pub(crate) fn optional_pubkey(value: Option<Pubkey>) -> OptionalPubkey {
    value
        .map(OptionalPubkey::some)
        .unwrap_or_else(OptionalPubkey::none)
}

macro_rules! add_change {
    ($changes:expr, $count:expr, $target:expr, $current:expr, $policy:expr) => {
        if let Some(target) = $target {
            $crate::instructions::administration::common::push_change(
                $changes,
                $count,
                $crate::instructions::administration::common::compressed_change(
                    $current, target, $policy,
                ),
            );
        }
    };
}

pub(crate) use add_change;

pub(crate) fn compressed_change<T: Ord>(
    current: T,
    target: T,
    policy: ChangePolicy,
) -> (i128, i128, ChangePolicy) {
    use core::cmp::Ordering;
    match target.cmp(&current) {
        Ordering::Less => (1, 0, policy),
        Ordering::Equal => (0, 0, policy),
        Ordering::Greater => (0, 1, policy),
    }
}

pub(crate) fn push_change<const N: usize>(
    changes: &mut [(i128, i128, ChangePolicy); N],
    count: &mut usize,
    change: (i128, i128, ChangePolicy),
) {
    changes[*count] = change;
    *count += 1;
}

pub(crate) fn creation_window(global: &GlobalGovernanceConfig, now: i64) -> Result<ProposalWindow> {
    Ok(ProposalWindow::create(
        now,
        global.creation_delay_seconds,
        global.proposal_expiry_seconds,
    )?)
}
