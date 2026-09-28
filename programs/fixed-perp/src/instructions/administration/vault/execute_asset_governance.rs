// CHECKED
use crate::state::*;
use anchor_lang::prelude::*;

#[derive(Accounts)]
pub struct ExecuteAssetGovernance<'info> {
    #[account(mut)]
    pub asset: AccountLoader<'info, AssetState>,
    #[account(
        mut,
        constraint = asset.load()?.is_governance_key(asset_governance.key()) @ StateError::InvalidKey
    )]
    pub asset_governance: AccountLoader<'info, AssetGovernance>,
}

pub fn handle_execute_asset_governance(
    context: Context<ExecuteAssetGovernance>,
) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let mut asset = context.accounts.asset.load_mut()?;
    context
        .accounts
        .asset_governance
        .load_mut()?
        .circuit_breaker
        .execute_into_asset(&mut asset, now)?;
    Ok(())
}
