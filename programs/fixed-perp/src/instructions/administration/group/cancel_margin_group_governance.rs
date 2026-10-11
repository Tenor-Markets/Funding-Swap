use crate::state::*;
use anchor_lang::prelude::*;

#[derive(Accounts)]
pub struct CancelMarginGroupGovernance<'info> {
    pub authority: Signer<'info>,
    #[account(constraint = global_governance.load()?.authority_for(GovernanceCategory::GroupMargin, authority.key()) @ StateError::InvalidAuthority)]
    pub global_governance: AccountLoader<'info, GlobalGovernanceConfig>,
    pub margin_group: AccountLoader<'info, MarginGroup>,
    #[account(
        mut,
        constraint = margin_group.load()?.is_governance_key(margin_group_governance.key()) @ StateError::InvalidKey
    )]
    pub margin_group_governance: AccountLoader<'info, MarginGroupGovernance>,
}

pub fn handle_cancel_margin_group_governance(
    context: Context<CancelMarginGroupGovernance>,
) -> Result<()> {
    context
        .accounts
        .margin_group_governance
        .load_mut()?
        .group_margin
        .cancel();

    Ok(())
}
