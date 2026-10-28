use crate::state::*;
use anchor_lang::prelude::*;

#[derive(Accounts)]
pub struct RaiseWithdrawalLimits<'info> {
    pub authority: Signer<'info>,
    #[account(
        constraint = global_governance.load()?.authority_for(GovernanceCategory::RiskRecovery, authority.key()) @ StateError::InvalidAuthority
    )]
    pub global_governance: AccountLoader<'info, GlobalGovernanceConfig>,
    #[account(
        seeds = [crate::state::USDC_SEED], bump = asset.load()?.bump,
        mut,
        constraint = !asset.load()?.is_pending() @ StateError::InvalidStatus
    )]
    pub asset: AccountLoader<'info, AssetState>,
}

pub fn handle_raise_withdrawal_limits(
    context: Context<RaiseWithdrawalLimits>,
    args: UpdateWithdrawalLimitsArgs,
) -> Result<()> {
    let mut resource = context.accounts.asset.load_mut()?;

    args.apply(&mut resource, false)?;

    Ok(())
}
