// CHECKED
use crate::state::{
    GlobalGovernanceConfig, GovernanceCategory, MarketGovernance, MarketState, StateError,
};
use anchor_lang::prelude::*;

fn is_zero(num: u32) -> bool {
    num == 0
}

#[derive(Accounts)]
pub struct CloseMarket<'info> {
    pub authority: Signer<'info>,
    #[account(
        constraint = global_governance.load()?.authority_for(GovernanceCategory::Creation, authority.key()) @ StateError::InvalidAuthority
    )]
    pub global_governance: AccountLoader<'info, GlobalGovernanceConfig>,
    #[account(
        mut,
        close = rent_recipient,
        constraint = is_zero(market.load()?.subaccount_count) @ StateError::Conflict
    )]
    pub market: AccountLoader<'info, MarketState>,
    #[account(
        mut,
        close = rent_recipient,
        constraint = market.load()?.is_governance_key(market_governance.key()) @ StateError::InvalidKey,
    )]
    pub market_governance: AccountLoader<'info, MarketGovernance>,
    /// CHECK: Receives rent from both closed protocol accounts.
    #[account(mut)]
    pub rent_recipient: UncheckedAccount<'info>,
}

pub fn handle_close_market(_: Context<CloseMarket>) -> Result<()> {
    Ok(())
}
