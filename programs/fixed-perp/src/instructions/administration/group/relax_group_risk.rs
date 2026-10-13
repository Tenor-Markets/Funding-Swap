use crate::state::*;
use anchor_lang::prelude::*;

#[derive(Accounts)]
pub struct RelaxGroupRisk<'info> {
    pub authority: Signer<'info>,
    #[account(
        constraint = global_governance.load()?.authority_for(GovernanceCategory::RiskRecovery, authority.key()) @ StateError::InvalidAuthority
    )]
    pub global_governance: AccountLoader<'info, GlobalGovernanceConfig>,
    #[account(
        mut, 
        constraint = !margin_group.load()?.is_pending() @ StateError::InvalidStatus
    )]
    pub margin_group: AccountLoader<'info, MarginGroup>,
}

pub fn handle_relax_group_risk(
    context: Context<RelaxGroupRisk>,
    args: UpdateGroupRiskArgs,
) -> Result<()> {
    let mut resource = context.accounts.margin_group.load_mut()?;

    args.apply(&mut resource, false)?;

    Ok(())
}
