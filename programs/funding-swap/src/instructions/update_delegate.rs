use anchor_lang::prelude::*;

use crate::state::{MarginGroup, StateError, Subaccount};

#[derive(Accounts)]
pub struct UpdateDelegate<'info> {
    pub authority: Signer<'info>,
    #[account(
        mut,
        constraint = subaccount.load()?.is_authority(authority.key()) @ StateError::InvalidAuthority
    )]
    pub subaccount: AccountLoader<'info, Subaccount>,
    #[account(
        constraint = margin_group.load()?.is_active() @ StateError::InvalidStatus,
        constraint = subaccount.load()?.is_same_group_id(margin_group.load()?.group_id) @ StateError::InvalidGroupID
    )]
    pub margin_group: AccountLoader<'info, MarginGroup>,
}

/// The owner appoints a delegate or revokes delegation with the default key.
pub fn handle_update_delegate(context: Context<UpdateDelegate>, delegate: Pubkey) -> Result<()> {
    context
        .accounts
        .subaccount
        .load_mut()?
        .set_delegate(delegate)?;

    Ok(())
}
