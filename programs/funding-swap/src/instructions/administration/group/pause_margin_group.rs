use crate::state::*;
use anchor_lang::prelude::*;

#[derive(Accounts)]
pub struct PauseMarginGroup<'info> {
    pub authority: Signer<'info>,
    #[account(
        constraint = global_governance.load()?.authority_for(GovernanceCategory::Emergency, authority.key()) @ StateError::InvalidAuthority
    )]
    pub global_governance: AccountLoader<'info, GlobalGovernanceConfig>,
    #[account(
        mut,
        constraint = margin_group.load()?.is_active() @ StateError::InvalidStatus
    )]
    pub margin_group: AccountLoader<'info, MarginGroup>,
}

pub fn handle_pause_margin_group(context: Context<PauseMarginGroup>) -> Result<()> {
    context.accounts.margin_group.load_mut()?.pause()?;

    Ok(())
}
