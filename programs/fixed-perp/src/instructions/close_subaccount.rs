use anchor_lang::prelude::*;

use crate::state::{MarginGroup, StateError, Subaccount};

#[derive(Accounts)]
#[instruction(group_id: u16)]
pub struct CloseSubaccount<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,
    #[account(
        mut,
        close = authority,
        seeds = [
            b"subaccount",
            authority.key().as_ref(),
            &subaccount.load()?.subaccount_id.to_le_bytes()
        ],
        bump = subaccount.load()?.bump,
        constraint = subaccount.load()?.authority == authority.key(),
        constraint = subaccount.load()?.group_id == group_id
    )]
    pub subaccount: AccountLoader<'info, Subaccount>,
    #[account(
        mut,
        seeds = [b"margin_group", &group_id.to_le_bytes()],
        bump = margin_group.load()?.bump,
        constraint = margin_group.load()?.group_id == group_id
    )]
    pub margin_group: AccountLoader<'info, MarginGroup>,
}

/// Closes an empty subaccount and removes it from its group's live count.
pub fn handle_close_subaccount(context: Context<CloseSubaccount>, _group_id: u16) -> Result<()> {
    let mut subaccount = context.accounts.subaccount.load_mut()?;
    subaccount.close_empty_markets();
    if !subaccount.is_empty() {
        return Err(StateError::Conflict.into());
    }
    drop(subaccount);
    context
        .accounts
        .margin_group
        .load_mut()?
        .remove_subaccount()?;
    Ok(())
}
