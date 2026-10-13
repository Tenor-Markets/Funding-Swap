use crate::state::*;
use anchor_lang::prelude::*;

/// Squads enforces approval and delay; resource activation only checks lifecycle prerequisites.
#[derive(Accounts)]
pub struct ActivateMarket<'info> {
    pub authority: Signer<'info>,
    #[account(
        constraint = global_governance.load()?.authority_for(GovernanceCategory::Resources, authority.key()) @ StateError::InvalidAuthority
    )]
    pub global_governance: AccountLoader<'info, GlobalGovernanceConfig>,
    #[account(
        mut, 
        constraint = market.load()?.is_pending() && market.load()?.is_seeded() @ StateError::InvalidStatus,
        constraint = market.load()?.can_start(Clock::get()?.unix_timestamp) @ StateError::Conflict
    )]
    pub market: AccountLoader<'info, MarketState>,
}

pub fn handle_activate_market(context: Context<ActivateMarket>) -> Result<()> {
    context.accounts.market.load_mut()?.activate();

    Ok(())
}
