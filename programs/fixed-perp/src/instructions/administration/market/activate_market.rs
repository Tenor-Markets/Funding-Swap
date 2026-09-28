// CHECKED
use crate::state::*;
use anchor_lang::prelude::*;

#[derive(Accounts)]
pub struct ActivateMarket<'info> {
    #[account(mut)]
    pub market: AccountLoader<'info, MarketState>,
    #[account(
        mut,
        constraint = market.load()?.is_governance_key(market_governance.key()) @ StateError::InvalidKey
    )]
    pub market_governance: AccountLoader<'info, MarketGovernance>,
}

pub fn handle_activate_market(context: Context<ActivateMarket>) -> Result<()> {
    // Execution is permissionless once the typed records' activation window
    // accepts the clock.
    let now = Clock::get()?.unix_timestamp;
    let mut market = context.accounts.market.load_mut()?;
    market.activate(
        &mut context
            .accounts
            .market_governance
            .load_mut()?
            .activation_window,
        now,
    )?;
    Ok(())
}
