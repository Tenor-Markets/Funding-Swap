use super::super::common::{
    add_change, optional_fixed, optional_u128, optional_u64, MarketRiskGovernanceArgs,
};
use crate::state::*;
use anchor_lang::prelude::*;

#[derive(Accounts)]
pub struct ProposeMarketRisk<'info> {
    pub authority: Signer<'info>,
    #[account(constraint = global_governance.load()?.authority_for(GovernanceCategory::MarketRisk, authority.key()) @ StateError::InvalidAuthority)]
    pub global_governance: AccountLoader<'info, GlobalGovernanceConfig>,
    #[account(constraint = !market.load()?.is_pending() @ StateError::InvalidStatus)]
    pub market: AccountLoader<'info, MarketState>,
    #[account(
        mut,
        constraint = market.load()?.is_governance_key(market_governance.key()) @ StateError::InvalidKey
    )]
    pub market_governance: AccountLoader<'info, MarketGovernance>,
}

pub fn handle_propose_market_risk(
    context: Context<ProposeMarketRisk>,
    args: MarketRiskGovernanceArgs,
) -> Result<()> {
    // Context binds the category authority, canonical market, and its
    // resource-local governance PDA.
    let now = Clock::get()?.unix_timestamp;
    let global = context.accounts.global_governance.load()?;
    let market = context.accounts.market.load()?;
    let proposal = MarketRiskProposal {
        minimum_position_size_atoms: optional_u64(args.minimum_position_size_atoms),
        maximum_position_size_atoms: optional_u64(args.maximum_position_size_atoms),
        hard_gross_open_interest_cap_atoms: optional_u128(args.hard_gross_open_interest_cap_atoms),
        mark_observation_window_seconds: optional_u64(args.mark_observation_window_seconds),
        margin_time_threshold_seconds: optional_u64(args.margin_time_threshold_seconds),
        order_rate_bound_factor: optional_fixed(args.order_rate_bound_factor_raw),
        trade_mark_update_bound_factor: optional_fixed(args.trade_mark_update_bound_factor_raw),
        ..Default::default()
    };
    let mut changes = [(0, 0, ChangePolicy::BothDelayed); 7];
    let mut count = 0;
    add_change!(
        &mut changes,
        &mut count,
        args.minimum_position_size_atoms,
        market.config.minimum_position_size_atoms,
        ChangePolicy::BothDelayed
    );
    add_change!(
        &mut changes,
        &mut count,
        args.maximum_position_size_atoms,
        market.config.maximum_position_size_atoms,
        ChangePolicy::DecreaseProtective
    );
    add_change!(
        &mut changes,
        &mut count,
        args.hard_gross_open_interest_cap_atoms,
        market.config.hard_gross_open_interest_cap_atoms,
        ChangePolicy::DecreaseProtective
    );
    add_change!(
        &mut changes,
        &mut count,
        args.mark_observation_window_seconds,
        u64::from(market.config.mark_observation_window_seconds),
        ChangePolicy::BothDelayed
    );
    add_change!(
        &mut changes,
        &mut count,
        args.margin_time_threshold_seconds,
        u64::from(market.config.margin_time_threshold_seconds),
        ChangePolicy::IncreaseProtective
    );
    add_change!(
        &mut changes,
        &mut count,
        args.order_rate_bound_factor_raw,
        market.config.order_rate_bound_factor.to_raw_bits(),
        ChangePolicy::DecreaseProtective
    );
    add_change!(
        &mut changes,
        &mut count,
        args.trade_mark_update_bound_factor_raw,
        market.config.trade_mark_update_bound_factor.to_raw_bits(),
        ChangePolicy::DecreaseProtective
    );
    global.replace_typed_proposal_value(
        &mut context.accounts.market_governance.load_mut()?.market_risk,
        proposal,
        GovernanceCategory::MarketRisk,
        now,
        &changes[..count],
    )?;
    Ok(())
}
