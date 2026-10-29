use crate::state::*;
use anchor_lang::prelude::*;

#[derive(Accounts)]
pub struct RestoreMarketStatus<'info> {
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

pub fn handle_restore_market_status(
    context: Context<RestoreMarketStatus>,
    status: u8,
) -> Result<()> {
    update_market_status(&mut *context.accounts.market.load_mut()?, status, false)?;

    Ok(())
}
