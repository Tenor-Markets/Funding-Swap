use anchor_lang::prelude::*;

use crate::state::Subaccount;

#[derive(Accounts)]
pub struct CloseEmptyMarkets<'info> {
    pub authority: Signer<'info>,
    #[account(
        mut,
        seeds = [
            b"subaccount",
            authority.key().as_ref(),
            &subaccount.load()?.subaccount_id.to_le_bytes()
        ],
        bump = subaccount.load()?.bump,
        constraint = subaccount.load()?.authority == authority.key()
    )]
    pub subaccount: AccountLoader<'info, Subaccount>,
}

/// Clears every empty market slot without loading any market account.
pub fn handle_close_empty_markets(context: Context<CloseEmptyMarkets>) -> Result<()> {
    context
        .accounts
        .subaccount
        .load_mut()?
        .close_empty_markets();
    Ok(())
}
