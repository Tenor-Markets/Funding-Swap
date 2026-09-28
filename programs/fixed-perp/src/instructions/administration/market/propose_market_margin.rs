// CHECKED
use super::super::common::{add_change, optional_fixed, MarketMarginGovernanceArgs};
use crate::state::*;
use anchor_lang::prelude::*;

#[derive(Accounts)]
pub struct ProposeMarketMargin<'info> {
    pub authority: Signer<'info>,
    #[account(constraint = global_governance.load()?.authority_for(GovernanceCategory::MarketMargin, authority.key()) @ StateError::InvalidAuthority)]
    pub global_governance: AccountLoader<'info, GlobalGovernanceConfig>,
    #[account(constraint = !market.load()?.is_pending() @ StateError::InvalidStatus)]
    pub market: AccountLoader<'info, MarketState>,
    #[account(
        mut,
        constraint = market.load()?.is_governance_key(market_governance.key()) @ StateError::InvalidKey
    )]
    pub market_governance: AccountLoader<'info, MarketGovernance>,
}

pub fn handle_propose_market_margin(
    context: Context<ProposeMarketMargin>,
    args: MarketMarginGovernanceArgs,
) -> Result<()> {
    // Context binds the category authority, canonical market, and its
    // resource-local governance PDA.
    let now = Clock::get()?.unix_timestamp;
    let global = context.accounts.global_governance.load()?;
    let market = context.accounts.market.load()?;
    let proposal = MarketMarginProposal {
        initial_margin_factor: optional_fixed(args.initial_margin_factor_raw),
        maintenance_margin_factor: optional_fixed(args.maintenance_margin_factor_raw),
        closing_margin_relief_bound_factor: optional_fixed(
            args.closing_margin_relief_bound_factor_raw,
        ),
        ..Default::default()
    };
    let mut changes = [(0, 0, ChangePolicy::BothDelayed); 3];
    let mut count = 0;
    add_change!(
        &mut changes,
        &mut count,
        args.initial_margin_factor_raw,
        market.config.initial_margin_factor.to_raw_bits(),
        ChangePolicy::IncreaseProtective
    );
    add_change!(
        &mut changes,
        &mut count,
        args.maintenance_margin_factor_raw,
        market.config.maintenance_margin_factor.to_raw_bits(),
        ChangePolicy::IncreaseProtective
    );
    add_change!(
        &mut changes,
        &mut count,
        args.closing_margin_relief_bound_factor_raw,
        market
            .config
            .closing_margin_relief_bound_factor
            .to_raw_bits(),
        ChangePolicy::DecreaseProtective
    );
    global.replace_typed_proposal_value(
        &mut context.accounts.market_governance.load_mut()?.market_margin,
        proposal,
        GovernanceCategory::MarketMargin,
        now,
        &changes[..count],
    )?;
    Ok(())
}
