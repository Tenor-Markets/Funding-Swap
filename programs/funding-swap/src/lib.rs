use anchor_lang::prelude::*;

pub mod instructions;
pub mod state;

pub use instructions::administration::*;
pub use instructions::cleanup::*;
pub use instructions::close_empty_markets::*;
pub use instructions::close_market::*;
pub use instructions::close_subaccount::*;
pub use instructions::create_subaccount::*;
pub use instructions::deleverage::*;
pub use instructions::deposit_asset::*;
pub use instructions::finalize_market::*;
pub use instructions::liquidate::*;
pub use instructions::materialize_subaccount::*;
pub use instructions::order::*;
pub use instructions::purge_expired_order::*;
pub use instructions::refresh_market::*;
pub use instructions::settle_market::*;
pub use instructions::update_delegate::*;
pub use instructions::withdraw_asset::*;

declare_id!("7NcM5j4x6ZW9yGeryVxKp5SVrUtpFdLcUYKYK4SX8i8K");

#[program]
pub mod funding_swap {
    use super::*;

    pub fn create_subaccount(context: Context<CreateSubaccount>, subaccount_id: u16) -> Result<()> {
        instructions::create_subaccount::handle_create_subaccount(context, subaccount_id)
    }

    pub fn update_delegate(context: Context<UpdateDelegate>, delegate: Pubkey) -> Result<()> {
        instructions::update_delegate::handle_update_delegate(context, delegate)
    }

    pub fn create_asset(context: Context<CreateAsset>, args: CreateAssetArgs) -> Result<()> {
        instructions::administration::vault::create_asset::handle_create_asset(context, args)
    }

    pub fn create_margin_group(
        context: Context<CreateMarginGroup>,
        args: CreateMarginGroupArgs,
    ) -> Result<()> {
        instructions::administration::group::create_margin_group::handle_create_margin_group(
            context, args,
        )
    }

    pub fn activate_margin_group(context: Context<ActivateMarginGroup>) -> Result<()> {
        instructions::administration::group::activate_margin_group::handle_activate_margin_group(
            context,
        )
    }

    pub fn initialize_market(
        context: Context<InitializeMarket>,
        args: CreateMarketArgs,
    ) -> Result<()> {
        instructions::administration::market::initialize_market::handle_initialize_market(
            context, args,
        )
    }

    pub fn close_margin_group(context: Context<CloseMarginGroup>) -> Result<()> {
        instructions::administration::group::close_margin_group::handle_close_margin_group(context)
    }

    pub fn activate_market(context: Context<ActivateMarket>) -> Result<()> {
        instructions::administration::market::activate_market::handle_activate_market(context)
    }

    pub fn pause_margin_group(context: Context<PauseMarginGroup>) -> Result<()> {
        instructions::administration::group::pause_margin_group::handle_pause_margin_group(context)
    }

    pub fn resume_margin_group(context: Context<ResumeMarginGroup>) -> Result<()> {
        instructions::administration::group::resume_margin_group::handle_resume_margin_group(
            context,
        )
    }

    pub fn close_empty_markets<'info>(
        context: Context<'info, CloseEmptyMarkets<'info>>,
    ) -> Result<()> {
        instructions::close_empty_markets::handle_close_empty_markets(context)
    }

    pub fn close_market(context: Context<CloseMarket>) -> Result<()> {
        instructions::close_market::handle_close_market(context)
    }

    pub fn close_subaccount<'info>(context: Context<'info, CloseSubaccount<'info>>) -> Result<()> {
        instructions::close_subaccount::handle_close_subaccount(context)
    }

    pub fn settle_market_into_collateral<'info>(
        context: Context<'info, SettleMarketIntoCollateral<'info>>,
    ) -> Result<()> {
        instructions::settle_market::handle_settle_market_into_collateral(context)
    }

    pub fn settle_all_markets_into_collateral<'info>(
        context: Context<'info, SettleAllMarketsIntoCollateral<'info>>,
    ) -> Result<()> {
        instructions::settle_market::handle_settle_all_markets_into_collateral(context)
    }

    pub fn settle_matured_markets<'info>(
        context: Context<'info, SettleMaturedMarkets<'info>>,
    ) -> Result<()> {
        instructions::settle_market::handle_settle_matured_markets(context)
    }

    pub fn deposit_asset(context: Context<DepositAsset>, amount: u64) -> Result<()> {
        instructions::deposit_asset::handle_deposit_asset(context, amount)
    }

    pub fn finalize_market<'info>(context: Context<'info, FinalizeMarket<'info>>) -> Result<()> {
        instructions::finalize_market::handle_finalize_market(context)
    }

    pub fn finalize_markets<'info>(context: Context<'info, FinalizeMarkets>) -> Result<()> {
        instructions::finalize_market::handle_finalize_markets(context)
    }

    pub fn withdraw_asset<'info>(
        context: Context<'info, WithdrawAsset<'info>>,
        amount: u64,
    ) -> Result<()> {
        instructions::withdraw_asset::handle_withdraw_asset(context, amount)
    }

    pub fn liquidate<'info>(
        context: Context<'info, Liquidate<'info>>,
        market_id: u64,
        size_to_liquidator: i64,
    ) -> Result<()> {
        instructions::liquidate::handle_liquidate(context, market_id, size_to_liquidator)
    }

    pub fn deleverage<'info>(
        context: Context<'info, Deleverage<'info>>,
        market_id: u64,
        size_to_winner: i64,
        alpha_raw: i128,
    ) -> Result<()> {
        instructions::deleverage::handle_deleverage(context, market_id, size_to_winner, alpha_raw)
    }

    pub fn initialize_governance(
        context: Context<InitializeGovernance>,
        args: GovernanceInitializationArgs,
    ) -> Result<()> {
        instructions::administration::global::initialize_governance::handle_initialize_governance(
            context, args,
        )
    }

    pub fn materialize_subaccount<'info>(
        context: Context<'info, MaterializeSubaccount<'info>>,
    ) -> Result<()> {
        instructions::materialize_subaccount::handle_materialize_subaccount(context)
    }

    pub fn place_order<'info>(
        context: Context<'info, PlaceOrderAccounts<'info>>,
        args: PlaceOrderArgs,
    ) -> Result<()> {
        instructions::order::handle_place_order(context, args)
    }

    pub fn cancel_order<'info>(
        context: Context<'info, CancelOrderAccounts<'info>>,
        sequence: u64,
        slab_index: u64,
    ) -> Result<()> {
        instructions::order::handle_cancel_order(context, sequence, slab_index)
    }

    pub fn modify_order<'info>(
        context: Context<'info, ModifyOrderAccounts<'info>>,
        args: ModifyOrderArgs,
    ) -> Result<()> {
        instructions::order::handle_modify_order(context, args)
    }

    pub fn close_all_orders<'info>(
        context: Context<'info, CloseAllOrdersAccounts<'info>>,
    ) -> Result<()> {
        instructions::order::handle_close_all_orders(context)
    }

    pub fn purge_expired_order<'info>(
        context: Context<'info, PurgeExpiredOrder<'info>>,
        sequence: u64,
        slab_index: u64,
    ) -> Result<()> {
        instructions::purge_expired_order::handle_purge_expired_order(context, sequence, slab_index)
    }

    pub fn risky_order_cleanup<'info>(context: Context<'info, GroupCleanup<'info>>) -> Result<()> {
        instructions::cleanup::handle_risky_order_cleanup(context)
    }

    pub fn close_only_cleanup<'info>(context: Context<'info, GroupCleanup<'info>>) -> Result<()> {
        instructions::cleanup::handle_close_only_cleanup(context)
    }

    pub fn refresh_market<'info>(context: Context<'info, RefreshMarket<'info>>) -> Result<()> {
        instructions::refresh_market::handle_refresh_market(context)
    }

    pub fn seed_market_source<'info>(
        context: Context<'info, SeedMarketSource<'info>>,
    ) -> Result<()> {
        instructions::refresh_market::handle_seed_market_source(context)
    }

    pub fn activate_market_group_membership(
        context: Context<ActivateMarketGroupMembership>,
    ) -> Result<()> {
        instructions::administration::market::activate_market_group_membership::handle_activate_market_group_membership(context)
    }

    pub fn activate_asset(context: Context<ActivateAsset>) -> Result<()> {
        instructions::administration::vault::activate_asset::handle_activate_asset(context)
    }

    pub fn tighten_market_risk(
        context: Context<TightenMarketRisk>,
        args: state::UpdateMarketRiskArgs,
    ) -> Result<()> {
        instructions::administration::market::tighten_market_risk::handle_tighten_market_risk(
            context, args,
        )
    }

    pub fn relax_market_risk(
        context: Context<RelaxMarketRisk>,
        args: state::UpdateMarketRiskArgs,
    ) -> Result<()> {
        instructions::administration::market::relax_market_risk::handle_relax_market_risk(
            context, args,
        )
    }

    pub fn tighten_group_risk(
        context: Context<TightenGroupRisk>,
        args: state::UpdateGroupRiskArgs,
    ) -> Result<()> {
        instructions::administration::group::tighten_group_risk::handle_tighten_group_risk(
            context, args,
        )
    }

    pub fn relax_group_risk(
        context: Context<RelaxGroupRisk>,
        args: state::UpdateGroupRiskArgs,
    ) -> Result<()> {
        instructions::administration::group::relax_group_risk::handle_relax_group_risk(
            context, args,
        )
    }

    pub fn tighten_withdrawal_limits(
        context: Context<TightenWithdrawalLimits>,
        args: state::UpdateWithdrawalLimitsArgs,
    ) -> Result<()> {
        instructions::administration::vault::tighten_withdrawal_limits::handle_tighten_withdrawal_limits(context, args)
    }

    pub fn raise_withdrawal_limits(
        context: Context<RaiseWithdrawalLimits>,
        args: state::UpdateWithdrawalLimitsArgs,
    ) -> Result<()> {
        instructions::administration::vault::raise_withdrawal_limits::handle_raise_withdrawal_limits(
            context, args,
        )
    }

    pub fn update_fees_and_liquidation(
        context: Context<UpdateFeesAndLiquidation>,
        args: state::UpdateFeesAndLiquidationArgs,
    ) -> Result<()> {
        instructions::administration::market::update_fees_and_liquidation::handle_update_fees_and_liquidation(context, args)
    }

    pub fn restrict_market_status(
        context: Context<RestrictMarketStatus>,
        status: u8,
    ) -> Result<()> {
        instructions::administration::market::restrict_market_status::handle_restrict_market_status(
            context, status,
        )
    }

    pub fn restore_market_status(context: Context<RestoreMarketStatus>, status: u8) -> Result<()> {
        instructions::administration::market::restore_market_status::handle_restore_market_status(
            context, status,
        )
    }

    pub fn update_authorities(
        context: Context<UpdateAuthorities>,
        args: state::UpdateAuthoritiesArgs,
    ) -> Result<()> {
        instructions::administration::global::update_authorities::handle_update_authorities(
            context, args,
        )
    }

    pub fn collect_protocol_fees<'info>(
        context: Context<'info, CollectProtocolFees<'info>>,
    ) -> Result<()> {
        instructions::administration::vault::collect_protocol_fees::handle_collect_protocol_fees(
            context,
        )
    }
}
