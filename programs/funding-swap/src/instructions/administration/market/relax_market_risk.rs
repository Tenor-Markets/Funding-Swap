use crate::state::*;
use anchor_lang::prelude::*;

#[derive(Accounts)]
pub struct RelaxMarketRisk<'info> {
    pub authority: Signer<'info>,
    #[account(
        constraint = global_governance.load()?.authority_for(GovernanceCategory::RiskRecovery, authority.key()) @ StateError::InvalidAuthority
    )]
    pub global_governance: AccountLoader<'info, GlobalGovernanceConfig>,
    #[account(
        mut,
        constraint = !market.load()?.is_pending() @ StateError::InvalidStatus
    )]
    pub market: AccountLoader<'info, MarketState>,
}

pub fn handle_relax_market_risk(
    context: Context<RelaxMarketRisk>,
    args: UpdateMarketRiskArgs,
) -> Result<()> {
    let mut resource = context.accounts.market.load_mut()?;

    args.apply(&mut resource.config, false)?;

    Ok(())
}
