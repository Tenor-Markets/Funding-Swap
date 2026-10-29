use crate::state::*;
use anchor_lang::prelude::*;

#[derive(Accounts)]
pub struct RestrictMarketStatus<'info> {
    pub authority: Signer<'info>,
    #[account(
        constraint = global_governance.load()?.authority_for(GovernanceCategory::Emergency, authority.key()) @ StateError::InvalidAuthority
    )]
    pub global_governance: AccountLoader<'info, GlobalGovernanceConfig>,
    #[account(
        mut,
        constraint = !market.load()?.is_pending() @ StateError::InvalidStatus
    )]
    pub market: AccountLoader<'info, MarketState>,
}

pub fn handle_restrict_market_status(
    context: Context<RestrictMarketStatus>,
    status: u8,
) -> Result<()> {
    update_market_status(&mut *context.accounts.market.load_mut()?, status, true)?;

    Ok(())
}
