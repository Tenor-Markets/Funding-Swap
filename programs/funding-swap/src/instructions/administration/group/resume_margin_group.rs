use crate::state::*;
use anchor_lang::prelude::*;

#[derive(Accounts)]
pub struct ResumeMarginGroup<'info> {
    pub authority: Signer<'info>,
    #[account(
        constraint = global_governance.load()?.authority_for(GovernanceCategory::RiskRecovery, authority.key()) @ StateError::InvalidAuthority
    )]
    pub global_governance: AccountLoader<'info, GlobalGovernanceConfig>,
    #[account(
        mut,
        constraint = margin_group.load()?.is_paused() @ StateError::InvalidStatus
    )]
    pub margin_group: AccountLoader<'info, MarginGroup>,
}

pub fn handle_resume_margin_group(context: Context<ResumeMarginGroup>) -> Result<()> {
    context.accounts.margin_group.load_mut()?.resume()?;

    Ok(())
}
