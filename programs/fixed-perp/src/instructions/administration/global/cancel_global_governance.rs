// CHECKED
use crate::state::{GlobalGovernanceConfig, GovernanceCategory, StateError};
use anchor_lang::prelude::*;

#[derive(Accounts)]
pub struct CancelGlobalGovernance<'info> {
    pub authority: Signer<'info>,
    #[account(
        mut,
        constraint = global_governance.load()?.authority_for(GovernanceCategory::Governance, authority.key()) @ StateError::InvalidAuthority
    )]
    pub global_governance: AccountLoader<'info, GlobalGovernanceConfig>,
}

pub fn handle_cancel_global_governance(context: Context<CancelGlobalGovernance>) -> Result<()> {
    let mut global = context.accounts.global_governance.load_mut()?;

    global.cancel_governance_proposal();

    Ok(())
}
