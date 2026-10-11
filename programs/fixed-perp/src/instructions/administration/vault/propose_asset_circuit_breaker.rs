use super::super::common::{add_change, optional_u128, CircuitBreakerGovernanceArgs};
use crate::state::*;
use anchor_lang::prelude::*;

/// Proposes withdrawal limits for the unified `AssetState`.
#[derive(Accounts)]
pub struct ProposeAssetCircuitBreaker<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,
    #[account(
        mut,
        constraint = global_governance.load()?.authority_for(GovernanceCategory::CircuitBreaker, authority.key()) @ StateError::InvalidAuthority
    )]
    pub global_governance: AccountLoader<'info, GlobalGovernanceConfig>,
    #[account(
        mut,
        constraint = !asset.load()?.is_pending() @ StateError::InvalidStatus
    )]
    pub asset: AccountLoader<'info, AssetState>,
    #[account(
        mut,
        constraint = asset.load()?.is_governance_key(asset_governance.key()) @ StateError::InvalidKey
    )]
    pub asset_governance: AccountLoader<'info, AssetGovernance>,
}

pub fn handle_propose_asset_circuit_breaker(
    context: Context<ProposeAssetCircuitBreaker>,
    args: CircuitBreakerGovernanceArgs,
) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let global = context.accounts.global_governance.load()?;
    let asset = context.accounts.asset.load()?;
    let proposal = CircuitBreakerProposal {
        short_window_limit_atoms: optional_u128(args.short_window_limit_atoms),
        medium_window_limit_atoms: optional_u128(args.medium_window_limit_atoms),
        long_window_limit_atoms: optional_u128(args.long_window_limit_atoms),
        ..Default::default()
    };
    let mut changes = [(0, 0, ChangePolicy::BothDelayed); 3];
    let mut count = 0;
    add_change!(
        &mut changes,
        &mut count,
        args.short_window_limit_atoms,
        asset.short_window.limit_atoms,
        ChangePolicy::DecreaseProtective
    );
    add_change!(
        &mut changes,
        &mut count,
        args.medium_window_limit_atoms,
        asset.medium_window.limit_atoms,
        ChangePolicy::DecreaseProtective
    );
    add_change!(
        &mut changes,
        &mut count,
        args.long_window_limit_atoms,
        asset.long_window.limit_atoms,
        ChangePolicy::DecreaseProtective
    );
    global.replace_typed_proposal_value(
        &mut context
            .accounts
            .asset_governance
            .load_mut()?
            .circuit_breaker,
        proposal,
        GovernanceCategory::CircuitBreaker,
        now,
        &changes[..count],
    )?;
    Ok(())
}
