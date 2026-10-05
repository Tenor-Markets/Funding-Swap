// CHECKED
use crate::state::*;
use anchor_lang::prelude::*;

#[derive(Accounts)]
pub struct ProposeProtocolFeeCollection<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,
    #[account(constraint = global_governance.load()?.authority_for(GovernanceCategory::AssetCustody, authority.key()) @ StateError::InvalidAuthority)]
    pub global_governance: AccountLoader<'info, GlobalGovernanceConfig>,
    #[account(constraint = asset.load()?.is_active() @ StateError::InvalidStatus)]
    pub asset: AccountLoader<'info, AssetState>,
    /// CHECK: The mint is checked against the custody vault when the withdrawal executes.
    pub destination: UncheckedAccount<'info>,
    #[account(
        init,
        payer = authority,
        space = PendingProtocolFeeCollection::SPACE,
        seeds = [b"pending_fee_collection", asset.key().as_ref()],
        bump
    )]
    pub pending_collection: Account<'info, PendingProtocolFeeCollection>,
    pub system_program: Program<'info, System>,
}

pub fn handle_propose_protocol_fee_collection(
    context: Context<ProposeProtocolFeeCollection>,
) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let asset = context.accounts.asset.load()?;
    if !asset.has_custody() {
        return Err(StateError::InvalidConfiguration.into());
    }
    let global = context.accounts.global_governance.load()?;
    let window = global.proposal_window_for_values(
        GovernanceCategory::AssetCustody,
        now,
        &[(0, 1, ChangePolicy::BothDelayed)],
    )?;
    context.accounts.pending_collection.initialize_in_place(
        context.accounts.asset.key(),
        context.accounts.destination.key(),
        context.accounts.authority.key(),
        window,
    )?;
    Ok(())
}
