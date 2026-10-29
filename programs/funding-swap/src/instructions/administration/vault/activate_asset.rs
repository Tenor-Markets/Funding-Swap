use crate::state::*;
use anchor_lang::prelude::*;

/// Squads enforces approval and delay; resource activation only checks lifecycle prerequisites.
#[derive(Accounts)]
pub struct ActivateAsset<'info> {
    pub authority: Signer<'info>,
    #[account(
        constraint = global_governance.load()?.authority_for(GovernanceCategory::Resources, authority.key()) @ StateError::InvalidAuthority
    )]
    pub global_governance: AccountLoader<'info, GlobalGovernanceConfig>,
    #[account(
        seeds = [crate::state::USDC_SEED], bump = asset.load()?.bump,
        mut,
        constraint = asset.load()?.is_pending() @ StateError::InvalidStatus
    )]
    pub asset: AccountLoader<'info, AssetState>,
}

pub fn handle_activate_asset(context: Context<ActivateAsset>) -> Result<()> {
    context.accounts.asset.load_mut()?.activate();

    Ok(())
}
