use crate::state::*;
use anchor_lang::prelude::*;

#[derive(Accounts)]
pub struct ProposeMarketGroupMembership<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,
    #[account(constraint = global_governance.load()?.authority_for(GovernanceCategory::Creation, authority.key()) @ StateError::InvalidAuthority)]
    pub global_governance: AccountLoader<'info, GlobalGovernanceConfig>,
    #[account(constraint = margin_group.load()?.is_active() @ StateError::InvalidStatus)]
    pub margin_group: AccountLoader<'info, MarginGroup>,
    #[account(constraint = market.load()?.is_pending() @ StateError::InvalidStatus)]
    pub market: AccountLoader<'info, MarketState>,
    #[account(
        init,
        payer = authority,
        space = PendingMarketGroupMembership::SPACE,
        seeds = [
            b"pending_market_group_membership",
            margin_group.key().as_ref(),
            market.key().as_ref()
        ],
        bump
    )]
    pub pending_membership: Account<'info, PendingMarketGroupMembership>,
    pub system_program: Program<'info, System>,
}

pub fn handle_propose_market_group_membership(
    context: Context<ProposeMarketGroupMembership>,
) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let global = context.accounts.global_governance.load()?;
    let group = context.accounts.margin_group.load()?;
    let market = context.accounts.market.load()?;
    if now >= market.config.start_timestamp {
        return Err(StateError::InvalidStatus.into());
    }
    group.ensure_market_assets_match(&market)?;
    context.accounts.pending_membership.initialize_in_place(
        context.accounts.margin_group.key(),
        context.accounts.market.key(),
        context.accounts.authority.key(),
        ProposalWindow::create(
            now,
            global.creation_delay_seconds,
            global.proposal_expiry_seconds,
        )?,
    )?;
    Ok(())
}
