use anchor_lang::prelude::*;

pub mod instructions;
pub mod state;

pub use instructions::administration::*;
pub use instructions::cleanup::*;
pub use instructions::close_empty_markets::*;
pub use instructions::close_market::*;
pub use instructions::close_subaccount::*;
pub use instructions::create_subaccount::*;
pub use instructions::deposit_asset::*;
pub use instructions::finalize_market::*;
pub use instructions::materialize_subaccount::*;
pub use instructions::order::*;
pub use instructions::refresh_market::*;
pub use instructions::settle_market::*;
pub use instructions::withdraw_asset::*;

declare_id!("7NcM5j4x6ZW9yGeryVxKp5SVrUtpFdLcUYKYK4SX8i8K");

#[program]
pub mod fixed_perp {
    use super::*;

    pub fn create_subaccount(context: Context<CreateSubaccount>, subaccount_id: u16) -> Result<()> {
        instructions::create_subaccount::handle_create_subaccount(context, subaccount_id)
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

    pub fn allocate_market(context: Context<AllocateMarket>, market_id: u32) -> Result<()> {
        instructions::administration::market::allocate_market::handle_allocate_market(
            context, market_id,
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
        context: Context<'info, OrderAccounts<'info>>,
        args: PlaceOrderArgs,
    ) -> Result<()> {
        instructions::order::handle_place_order(context, args)
    }

    pub fn cancel_order<'info>(
        context: Context<'info, OrderAccounts<'info>>,
        sequence: u64,
        slab_index: u64,
    ) -> Result<()> {
        instructions::order::handle_cancel_order(context, sequence, slab_index)
    }

    pub fn modify_order<'info>(
        context: Context<'info, OrderAccounts<'info>>,
        args: ModifyOrderArgs,
    ) -> Result<()> {
        instructions::order::handle_modify_order(context, args)
    }

    pub fn close_all_orders<'info>(context: Context<'info, OrderAccounts<'info>>) -> Result<()> {
        instructions::order::handle_close_all_orders(context)
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

    pub fn seed_market_source<'info>(context: Context<'info, RefreshMarket<'info>>) -> Result<()> {
        instructions::refresh_market::handle_seed_market_source(context)
    }

    pub fn propose_market_group_membership(
        context: Context<ProposeMarketGroupMembership>,
    ) -> Result<()> {
        instructions::administration::market::propose_market_group_membership::handle_propose_market_group_membership(context)
    }

    pub fn activate_market_group_membership(
        context: Context<ActivateMarketGroupMembership>,
    ) -> Result<()> {
        instructions::administration::market::activate_market_group_membership::handle_activate_market_group_membership(context)
    }

    pub fn activate_asset(context: Context<ActivateAsset>) -> Result<()> {
        instructions::administration::vault::activate_asset::handle_activate_asset(context)
    }

    pub fn execute_market_governance(
        context: Context<ExecuteMarketGovernance>,
        category: u8,
    ) -> Result<()> {
        instructions::administration::market::execute_market_governance::handle_execute_market_governance(context, category)
    }

    pub fn propose_global_governance(
        context: Context<ProposeGlobalGovernance>,
        args: GlobalGovernanceArgs,
    ) -> Result<()> {
        instructions::administration::global::propose_global_governance::handle_propose_global_governance(context, args)
    }

    pub fn cancel_global_governance(context: Context<CancelGlobalGovernance>) -> Result<()> {
        instructions::administration::global::cancel_global_governance::handle_cancel_global_governance(context)
    }

    pub fn execute_global_governance(context: Context<ExecuteGlobalGovernance>) -> Result<()> {
        instructions::administration::global::execute_global_governance::handle_execute_global_governance(context)
    }

    pub fn propose_market_margin(
        context: Context<ProposeMarketMargin>,
        args: MarketMarginGovernanceArgs,
    ) -> Result<()> {
        instructions::administration::market::propose_market_margin::handle_propose_market_margin(
            context, args,
        )
    }

    pub fn propose_group_margin(
        context: Context<ProposeGroupMargin>,
        args: GroupMarginGovernanceArgs,
    ) -> Result<()> {
        instructions::administration::group::propose_group_margin::handle_propose_group_margin(
            context, args,
        )
    }

    pub fn propose_liquidation(
        context: Context<ProposeLiquidation>,
        args: LiquidationGovernanceArgs,
    ) -> Result<()> {
        instructions::administration::market::propose_liquidation::handle_propose_liquidation(
            context, args,
        )
    }

    pub fn propose_fee_rate(
        context: Context<ProposeFeeRate>,
        args: FeeRateGovernanceArgs,
    ) -> Result<()> {
        instructions::administration::market::propose_fee_rate::handle_propose_fee_rate(
            context, args,
        )
    }

    pub fn propose_market_risk(
        context: Context<ProposeMarketRisk>,
        args: MarketRiskGovernanceArgs,
    ) -> Result<()> {
        instructions::administration::market::propose_market_risk::handle_propose_market_risk(
            context, args,
        )
    }

    pub fn propose_market_status(
        context: Context<ProposeMarketStatus>,
        args: MarketStatusGovernanceArgs,
    ) -> Result<()> {
        instructions::administration::market::propose_market_status::handle_propose_market_status(
            context, args,
        )
    }

    pub fn propose_asset_circuit_breaker(
        context: Context<ProposeAssetCircuitBreaker>,
        args: CircuitBreakerGovernanceArgs,
    ) -> Result<()> {
        instructions::administration::vault::propose_asset_circuit_breaker::handle_propose_asset_circuit_breaker(
            context, args,
        )
    }

    pub fn execute_margin_group_governance(
        context: Context<ExecuteMarginGroupGovernance>,
    ) -> Result<()> {
        instructions::administration::group::execute_margin_group_governance::handle_execute_margin_group_governance(context)
    }

    pub fn execute_asset_governance(context: Context<ExecuteAssetGovernance>) -> Result<()> {
        instructions::administration::vault::execute_asset_governance::handle_execute_asset_governance(context)
    }

    pub fn cancel_market_governance(
        context: Context<CancelMarketGovernance>,
        category: u8,
    ) -> Result<()> {
        instructions::administration::market::cancel_market_governance::handle_cancel_market_governance(context, category)
    }

    pub fn cancel_margin_group_governance(
        context: Context<CancelMarginGroupGovernance>,
    ) -> Result<()> {
        instructions::administration::group::cancel_margin_group_governance::handle_cancel_margin_group_governance(context)
    }

    pub fn cancel_asset_governance(context: Context<CancelAssetGovernance>) -> Result<()> {
        instructions::administration::vault::cancel_asset_governance::handle_cancel_asset_governance(
            context,
        )
    }

    pub fn propose_protocol_fee_collection(
        context: Context<ProposeProtocolFeeCollection>,
    ) -> Result<()> {
        instructions::administration::vault::propose_protocol_fee_collection::handle_propose_protocol_fee_collection(context)
    }

    pub fn cancel_protocol_fee_collection(
        context: Context<CancelProtocolFeeCollection>,
    ) -> Result<()> {
        instructions::administration::vault::cancel_protocol_fee_collection::handle_cancel_protocol_fee_collection(context)
    }

    pub fn execute_protocol_fee_collection<'info>(
        context: Context<'info, ExecuteProtocolFeeCollection<'info>>,
    ) -> Result<()> {
        instructions::administration::vault::execute_protocol_fee_collection::handle_execute_protocol_fee_collection(context)
    }
}
