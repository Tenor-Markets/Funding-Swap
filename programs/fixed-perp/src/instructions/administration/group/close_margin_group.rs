// CHECKED
use crate::state::*;
use anchor_lang::prelude::*;

fn is_zero(value: u32) -> bool {
    value == 0
}

#[derive(Accounts)]
pub struct CloseMarginGroup<'info> {
    pub authority: Signer<'info>,
    #[account(constraint = global_governance.load()?.authority_for(GovernanceCategory::Creation, authority.key()) @ StateError::InvalidAuthority)]
    pub global_governance: AccountLoader<'info, GlobalGovernanceConfig>,
    #[account(
        mut,
        close = rent_recipient
    )]
    pub margin_group_governance: AccountLoader<'info, MarginGroupGovernance>,
    #[account(
        mut,
        close = rent_recipient,
        constraint = is_zero(margin_group.load()?.market_count) @ StateError::MarketsNotEmpty,
        constraint = is_zero(margin_group.load()?.subaccount_count) @ StateError::SubaccountsNotEmpty,
        constraint = margin_group.load()?.is_governance_key(margin_group_governance.key()) @ StateError::InvalidKey
    )]
    pub margin_group: AccountLoader<'info, MarginGroup>,
    /// CHECK: Receives rent from both closed protocol accounts.
    #[account(mut)]
    pub rent_recipient: UncheckedAccount<'info>,
}

pub fn handle_close_margin_group(_context: Context<CloseMarginGroup>) -> Result<()> {
    Ok(())
}
