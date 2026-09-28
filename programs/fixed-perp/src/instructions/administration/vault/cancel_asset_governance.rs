// CHECKED
use crate::state::*;
use anchor_lang::prelude::*;

#[derive(Accounts)]
pub struct CancelAssetGovernance<'info> {
    pub authority: Signer<'info>,
    #[account(constraint = global_governance.load()?.authority_for(GovernanceCategory::CircuitBreaker, authority.key()) @ StateError::InvalidAuthority)]
    pub global_governance: AccountLoader<'info, GlobalGovernanceConfig>,
    pub asset: AccountLoader<'info, AssetState>,
    #[account(
        mut,
        constraint = asset.load()?.is_governance_key(asset_governance.key()) @ StateError::InvalidKey
    )]
    pub asset_governance: AccountLoader<'info, AssetGovernance>,
}

pub fn handle_cancel_asset_governance(
    context: Context<CancelAssetGovernance>,
) -> Result<()> {
    context
        .accounts
        .asset_governance
        .load_mut()?
        .circuit_breaker
        .cancel();
    Ok(())
}
