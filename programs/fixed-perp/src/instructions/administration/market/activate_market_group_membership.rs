use crate::state::*;
use anchor_lang::prelude::*;

#[derive(Accounts)]
pub struct ActivateMarketGroupMembership<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,
    #[account(
        constraint = global_governance.load()?.authority_for(GovernanceCategory::Resources, authority.key()) @ StateError::InvalidAuthority
    )]
    pub global_governance: AccountLoader<'info, GlobalGovernanceConfig>,
    #[account(
        mut,
        constraint = margin_group.load()?.is_active() @ StateError::InvalidStatus
    )]
    pub margin_group: AccountLoader<'info, MarginGroup>,
    #[account(
        constraint = market.load()?.is_pending() @ StateError::InvalidStatus
    )]
    pub market: AccountLoader<'info, MarketState>,
    #[account(
        init,
        payer = authority,
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

    group.add_market()?;

    Ok(())
}
