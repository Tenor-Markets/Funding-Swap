use crate::state::GlobalGovernanceConfig;
use anchor_lang::prelude::*;

#[derive(Accounts)]
pub struct ExecuteGlobalGovernance<'info> {
    #[account(mut)]
    pub global_governance: AccountLoader<'info, GlobalGovernanceConfig>,
}

pub fn handle_execute_global_governance(context: Context<ExecuteGlobalGovernance>) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;

    let mut global = context.accounts.global_governance.load_mut()?;

    global.execute_governance_proposal(now)?;

    Ok(())
}
