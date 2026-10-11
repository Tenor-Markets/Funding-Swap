use super::super::common::{optional_pubkey, optional_u64, optional_u8, GlobalGovernanceArgs};
use crate::state::*;
use anchor_lang::prelude::*;

#[derive(Accounts)]
pub struct ProposeGlobalGovernance<'info> {
    pub authority: Signer<'info>,
    #[account(
        mut,
        constraint = global_governance.load()?.authority_for(GovernanceCategory::Governance, authority.key()) @ StateError::InvalidAuthority
    )]
    pub global_governance: AccountLoader<'info, GlobalGovernanceConfig>,
}

pub fn handle_propose_global_governance(
    context: Context<ProposeGlobalGovernance>,
    args: GlobalGovernanceArgs,
) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;

    let mut global = context.accounts.global_governance.load_mut()?;

    let replacement = GlobalGovernanceProposal {
        new_authority: optional_pubkey(args.new_authority),
        subaccount_creation_fee_destination: optional_pubkey(
            args.subaccount_creation_fee_destination,
        ),
        increase_delay_seconds: optional_u64(args.increase_delay_seconds),
        decrease_delay_seconds: optional_u64(args.decrease_delay_seconds),
        creation_delay_seconds: optional_u64(args.creation_delay_seconds),
        proposal_expiry_seconds: optional_u64(args.proposal_expiry_seconds),
        subaccount_creation_fee_lamports: optional_u64(args.subaccount_creation_fee_lamports),
        target_category: optional_u8(args.target_category),
        ..Default::default()
    };

    global.replace_governance_proposal(replacement, now)?;

    Ok(())
}
