use super::super::common::*;
use crate::state::*;
use anchor_lang::prelude::*;
use bytemuck::Zeroable;

#[derive(Accounts)]
pub struct InitializeMarket<'info> {
    pub authority: Signer<'info>,
    #[account(
        mut,
        constraint = global_governance.load()?.authority_for(GovernanceCategory::Resources, authority.key()) @ StateError::InvalidAuthority
    )]
    pub global_governance: AccountLoader<'info, GlobalGovernanceConfig>,
    #[account(
        zero,
        constraint = market.to_account_info().data_len() >= MarketState::SPACE @ StateError::InvalidAccountLength
    )]
    pub market: AccountLoader<'info, MarketState>,
}

pub fn handle_initialize_market(
    context: Context<InitializeMarket>,
    args: CreateMarketArgs,
) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;

    let mut global = context.accounts.global_governance.load_mut()?;

    let market_id = global.next_market_id;

    let next_market_id = market_id
        .checked_add(1)
        .ok_or(StateError::ArithmeticOverflow)?;

    global.next_market_id = next_market_id;

    let mut adapter = VenueAdapterConfig::default();

    match args.adapter {
        MarketAdapterArgs::Jupiter {
            max_source_delta,
            custody,
            pool,
            mint,
            max_source_age_seconds,
        } => {
            adapter.kind = VenueAdapterKind(ADAPTER_JUPITER);

            *adapter.jupiter_mut()? = JupiterAdapterPayload {
                max_source_delta,
                custody,
                pool,
                mint,
                max_source_age_seconds,
                layout: VenueLayout(1),
                ..JupiterAdapterPayload::default()
            };
        }
        MarketAdapterArgs::Phoenix {
            max_abs_native_delta,
            global_configuration,
            perp_asset_map,
            market_account,
            tick_size,
            funding_interval_seconds,
            funding_period_seconds,
            max_funding_age_seconds,
            max_mark_age_slots,
            symbol,
            asset_slot,
            asset_id,
            base_lot_decimals,
        } => {
            adapter.kind = VenueAdapterKind(ADAPTER_PHOENIX);

            *adapter.phoenix_mut()? = PhoenixAdapterPayload {
                max_abs_native_delta,
                global_configuration,
                perp_asset_map,
                market_account,
                tick_size,
                funding_interval_seconds,
                funding_period_seconds,
                max_funding_age_seconds,
                max_mark_age_slots,
                symbol,
                asset_slot,
                asset_id,
                base_lot_decimals,
                layout: VenueLayout(1),
                ..PhoenixAdapterPayload::default()
            };
        }
    }

    let mut config = MarketConfig::zeroed();

    config.initialize_in_place(
        args.hard_gross_open_interest_cap_atoms,
        args.start_timestamp,
        args.maturity_timestamp,
        OracleConfig {
            feed_id: args.underlying_feed_id,
            max_age_seconds: args.underlying_max_age_seconds,
            _pad: [0; 8],
            max_confidence_ratio: FixedPoint::from_raw_bits(
                args.underlying_max_confidence_ratio_raw,
            ),
            max_ema_deviation: FixedPoint::from_raw_bits(args.underlying_max_ema_deviation_raw),
        },
        adapter,
        FixedPoint::from_raw_bits(args.rate_floor_raw),
        FixedPoint::from_raw_bits(args.initial_margin_factor_raw),
        FixedPoint::from_raw_bits(args.maintenance_margin_factor_raw),
        FixedPoint::from_raw_bits(args.order_rate_bound_factor_raw),
        FixedPoint::from_raw_bits(args.closing_margin_relief_bound_factor_raw),
        FixedPoint::from_raw_bits(args.trade_mark_update_bound_factor_raw),
        FixedPoint::from_raw_bits(args.taker_fee_rate_raw),
        FixedPoint::from_raw_bits(args.annualized_settlement_fee_rate_raw),
        FixedPoint::from_raw_bits(args.liquidation_base_incentive_raw),
        FixedPoint::from_raw_bits(args.liquidation_incentive_slope_raw),
        FixedPoint::from_raw_bits(args.liquidation_fee_rate_raw),
        market_id,
        args.mark_observation_window_seconds,
        args.margin_time_threshold_seconds,
        args.tick_scale,
        args.underlying_decimals,
    )?;

    let mut market = context.accounts.market.load_init()?;

    market.initialize_in_place(
        config,
        now,
        args.tick_step,
        args.minimum_order_size_atoms,
        args.maximum_order_size_atoms,
        args.maximum_open_orders_per_subaccount,
    )?;

    Ok(())
}
