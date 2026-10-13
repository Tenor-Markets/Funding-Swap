use crate::state::{GlobalGovernanceConfig, GovernanceCategory, MarketState, StateError};
use anchor_lang::prelude::*;

fn is_zero(num: u32) -> bool {
    num == 0
}

#[derive(Accounts)]
pub struct CloseMarket<'info> {
    pub authority: Signer<'info>,
    #[account(
        constraint = global_governance.load()?.authority_for(GovernanceCategory::Resources, authority.key()) @ StateError::InvalidAuthority
    )]
    pub global_governance: AccountLoader<'info, GlobalGovernanceConfig>,
    #[account(
        mut,
        close = rent_recipient,
        constraint = is_zero(market.load()?.subaccount_count) @ StateError::Conflict
    )]
    pub market: AccountLoader<'info, MarketState>,
    /// CHECK: Receives rent from the closed market.
    #[account(mut)]
    pub rent_recipient: UncheckedAccount<'info>,
}

pub fn handle_close_market(_: Context<CloseMarket>) -> Result<()> {
    Ok(())
}
