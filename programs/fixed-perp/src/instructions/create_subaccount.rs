use anchor_lang::prelude::*;

use crate::state::{MarginGroup, Subaccount};

#[derive(Accounts)]
#[instruction(subaccount_id: u16, group_id: u16)]
pub struct CreateSubaccount<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,
    #[account(
        init,
        payer = authority,
        space = Subaccount::SPACE,
        seeds = [b"subaccount", authority.key().as_ref(), &subaccount_id.to_le_bytes()],
        bump
    )]
    pub subaccount: AccountLoader<'info, Subaccount>,
    #[account(
        mut,
        seeds = [b"margin_group", &group_id.to_le_bytes()],
        bump = margin_group.load()?.bump,
        constraint = margin_group.load()?.group_id == group_id
    )]
    pub margin_group: AccountLoader<'info, MarginGroup>,
    pub system_program: Program<'info, System>,
}

/// Initializes one authority-selected subaccount ID and records it in the group count.
pub fn handle_create_subaccount(
    context: Context<CreateSubaccount>,
    subaccount_id: u16,
    group_id: u16,
) -> Result<()> {
    let bump = context.bumps.subaccount;
    context
        .accounts
        .subaccount
        .load_init()?
        .initialize_in_place(
            bump,
            context.accounts.authority.key(),
            subaccount_id,
            group_id,
        )?;
    context.accounts.margin_group.load_mut()?.add_subaccount()?;
    Ok(())
}
