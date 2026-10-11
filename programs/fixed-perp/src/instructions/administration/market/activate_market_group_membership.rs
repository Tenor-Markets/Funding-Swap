use crate::state::*;
use anchor_lang::prelude::*;

#[derive(Accounts)]
pub struct ActivateMarketGroupMembership<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(
        mut,
        constraint = margin_group.load()?.is_active() @ StateError::InvalidStatus
    )]
    pub margin_group: AccountLoader<'info, MarginGroup>,
    #[account(constraint = market.load()?.is_pending() @ StateError::InvalidStatus)]
    pub market: AccountLoader<'info, MarketState>,
    #[account(
        mut,
        close = rent_recipient,
        constraint = pending_membership.is_same_margin_group(margin_group.key()) @ StateError::InvalidKey,
        constraint = pending_membership.is_same_market(market.key()) @ StateError::InvalidKey,
        constraint = pending_membership.is_same_rent_recipient(rent_recipient.key()) @ StateError::InvalidKey
    )]
    pub pending_membership: Account<'info, PendingMarketGroupMembership>,
    /// CHECK: Address is fixed by the pending proposal; it only receives rent.
    #[account(mut)]
    pub rent_recipient: UncheckedAccount<'info>,
    #[account(
        init,
        payer = payer,
        space = 8,
        seeds = [
            b"market_group_membership",
            margin_group.key().as_ref(),
            market.key().as_ref()
        ],
        bump
    )]
    pub membership: Account<'info, MarketGroupMembership>,
    pub system_program: Program<'info, System>,
}

pub fn handle_activate_market_group_membership(
    context: Context<ActivateMarketGroupMembership>,
) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let mut group = context.accounts.margin_group.load_mut()?;
    let market = context.accounts.market.load()?;
    if now >= market.config.start_timestamp {
        return Err(StateError::InvalidStatus.into());
    }
    group.ensure_market_assets_match(&market)?;
    context.accounts.pending_membership.window.accepts(now)?;
    group.add_market()?;
    Ok(())
}
