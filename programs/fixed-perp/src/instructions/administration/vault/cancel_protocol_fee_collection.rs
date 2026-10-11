use crate::state::*;
use anchor_lang::prelude::*;

#[derive(Accounts)]
pub struct CancelProtocolFeeCollection<'info> {
    pub authority: Signer<'info>,
    #[account(constraint = global_governance.load()?.authority_for(GovernanceCategory::AssetCustody, authority.key()) @ StateError::InvalidAuthority)]
    pub global_governance: AccountLoader<'info, GlobalGovernanceConfig>,
    #[account(
        mut,
        close = rent_recipient,
        constraint = pending_collection.is_same_asset(asset.key()) @ StateError::InvalidKey,
        constraint = pending_collection.is_same_rent_recipient(rent_recipient.key()) @ StateError::InvalidKey
    )]
    pub pending_collection: Account<'info, PendingProtocolFeeCollection>,
    pub asset: AccountLoader<'info, AssetState>,
    /// CHECK: Address is fixed by the pending proposal; it only receives rent.
    #[account(mut)]
    pub rent_recipient: UncheckedAccount<'info>,
}

pub fn handle_cancel_protocol_fee_collection(
    _context: Context<CancelProtocolFeeCollection>,
) -> Result<()> {
    Ok(())
}
