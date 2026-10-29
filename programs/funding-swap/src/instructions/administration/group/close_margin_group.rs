use crate::state::*;
use anchor_lang::prelude::*;

fn is_zero(value: u32) -> bool {
    value == 0
}

#[derive(Accounts)]
pub struct CloseMarginGroup<'info> {
    pub authority: Signer<'info>,
    #[account(
        constraint = global_governance.load()?.authority_for(GovernanceCategory::Resources, authority.key()) @ StateError::InvalidAuthority
    )]
    pub global_governance: AccountLoader<'info, GlobalGovernanceConfig>,
    #[account(
        mut,
        close = rent_recipient,
        constraint = is_zero(margin_group.load()?.market_count) @ StateError::MarketsNotEmpty,
        constraint = is_zero(margin_group.load()?.subaccount_count) @ StateError::SubaccountsNotEmpty,
    )]
    pub margin_group: AccountLoader<'info, MarginGroup>,
    /// CHECK: Receives rent from the closed margin group.
    #[account(mut)]
    pub rent_recipient: UncheckedAccount<'info>,
}

pub fn handle_close_margin_group(_context: Context<CloseMarginGroup>) -> Result<()> {
    Ok(())
}
