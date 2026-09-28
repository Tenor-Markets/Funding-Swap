// CHECKED
use super::super::common::*;
use crate::state::*;
use anchor_lang::prelude::*;
use bytemuck::Zeroable;

#[derive(Accounts)]
#[instruction(args: CreateMarketArgs)]
pub struct InitializeMarket<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,
    #[account(constraint = global_governance.load()?.authority_for(GovernanceCategory::Creation, authority.key()) @ StateError::InvalidAuthority)]
    pub global_governance: AccountLoader<'info, GlobalGovernanceConfig>,
    #[account(
        zero,
        seeds = [b"market_state", args.market_id.to_le_bytes().as_ref()],
        bump
    )]
    pub market: AccountLoader<'info, MarketState>,
    #[account(
        init,
        payer = authority,
        space = MarketGovernance::SPACE,
        seeds = [b"market_governance", market.key().as_ref()],
        bump
    )]
    pub market_governance: AccountLoader<'info, MarketGovernance>,
    #[account(constraint = collateral_state.load()?.is_active() @ StateError::InvalidStatus)]
    pub collateral_state: AccountLoader<'info, AssetState>,
    #[account(constraint = settlement_state.load()?.is_active() @ StateError::InvalidStatus)]
    pub settlement_state: AccountLoader<'info, AssetState>,
    #[account(constraint = underlying_state.load()?.is_active() @ StateError::InvalidStatus)]
    pub underlying_state: AccountLoader<'info, AssetState>,
    pub system_program: Program<'info, System>,
}

pub fn handle_initialize_market(
    context: Context<InitializeMarket>,
    args: CreateMarketArgs,
) -> Result<()> {
    // Context binds the zeroed market, governance, and all three immutable
    // AssetState accounts before configuration is initialized.
    let now = Clock::get()?.unix_timestamp;
    let global = context.accounts.global_governance.load()?;
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
            canonical_quote_mint,
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
                canonical_quote_mint,
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
        args.minimum_position_size_atoms,
        args.maximum_position_size_atoms,
        context.accounts.collateral_state.key(),
        context.accounts.settlement_state.key(),
        context.accounts.underlying_state.key(),
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
        args.market_id,
        args.mark_observation_window_seconds,
        args.margin_time_threshold_seconds,
        args.tick_scale,
        context.accounts.underlying_state.load()?.decimals,
        context.accounts.settlement_state.load()?.decimals,
    )?;
    let mut market = context.accounts.market.load_init()?;
    market.initialize_in_place(
        context.bumps.market,
        config,
        now,
        args.tick_step,
        args.minimum_order_size_atoms,
        args.maximum_order_size_atoms,
        args.maximum_open_orders_per_subaccount,
        context.accounts.market_governance.key(),
    )?;

    context
        .accounts
        .market_governance
        .load_init()?
        .initialize_in_place(creation_window(&global, now)?)?;
    Ok(())
}
