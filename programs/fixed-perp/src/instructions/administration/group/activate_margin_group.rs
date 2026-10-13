use crate::state::*;
use anchor_lang::prelude::*;

/// Squads enforces approval and delay; resource activation only checks lifecycle prerequisites.
#[derive(Accounts)]
pub struct ActivateMarginGroup<'info> {
    pub authority: Signer<'info>,
    #[account(
        constraint = global_governance.load()?.authority_for(GovernanceCategory::Resources, authority.key()) @ StateError::InvalidAuthority
    )]
    pub global_governance: AccountLoader<'info, GlobalGovernanceConfig>,
    #[account(
        mut, 
        constraint = margin_group.load()?.is_pending() @ StateError::InvalidStatus
    )]
    pub margin_group: AccountLoader<'info, MarginGroup>,
}

pub fn handle_activate_margin_group(context: Context<ActivateMarginGroup>) -> Result<()> {
    context.accounts.margin_group.load_mut()?.activate();

    Ok(())
}
