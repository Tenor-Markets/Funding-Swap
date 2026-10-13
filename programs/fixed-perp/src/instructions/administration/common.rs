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
