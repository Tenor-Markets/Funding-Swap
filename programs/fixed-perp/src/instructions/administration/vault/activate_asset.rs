use crate::state::*;
use anchor_lang::prelude::*;

/// Permissionless activation of a pending AssetState after its governance
/// account's snapshotted creation window opens.
#[derive(Accounts)]
pub struct ActivateAsset<'info> {
    #[account(mut)]
    pub asset: AccountLoader<'info, AssetState>,
    #[account(
        mut,
        constraint = asset.load()?.is_governance_key(asset_governance.key()) @ StateError::InvalidKey
    )]
    pub asset_governance: AccountLoader<'info, AssetGovernance>,
}

pub fn handle_activate_asset(context: Context<ActivateAsset>) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let mut asset = context.accounts.asset.load_mut()?;
    asset.activate(
        &mut context
            .accounts
            .asset_governance
            .load_mut()?
            .activation_window,
        now,
    )?;
    Ok(())
}
