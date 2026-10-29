use crate::state::*;
use anchor_lang::prelude::*;

#[derive(Accounts)]
pub struct TightenGroupRisk<'info> {
    pub authority: Signer<'info>,
    #[account(
        constraint = global_governance.load()?.authority_for(GovernanceCategory::RiskTightening, authority.key()) @ StateError::InvalidAuthority
    )]
    pub global_governance: AccountLoader<'info, GlobalGovernanceConfig>,
    #[account(
        mut,
        constraint = !margin_group.load()?.is_pending() @ StateError::InvalidStatus
    )]
    pub margin_group: AccountLoader<'info, MarginGroup>,
}

pub fn handle_tighten_group_risk(
    context: Context<TightenGroupRisk>,
    args: UpdateGroupRiskArgs,
) -> Result<()> {
    let mut resource = context.accounts.margin_group.load_mut()?;

    args.apply(&mut resource, true)?;

    Ok(())
}
