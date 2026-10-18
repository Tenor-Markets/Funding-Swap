use anchor_lang::prelude::*;

use crate::state::{GlobalGovernanceConfig, MarginGroup, StateError, Subaccount};

#[derive(Accounts)]
#[instruction(subaccount_id: u16)]
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
        constraint = margin_group.load()?.is_active() @ StateError::InvalidStatus
    )]
    pub margin_group: AccountLoader<'info, MarginGroup>,
    pub global_governance: AccountLoader<'info, GlobalGovernanceConfig>,
    /// CHECK: Its key is constrained to the destination stored in governance.
    #[account(
        mut,
        constraint = global_governance
            .load()?
            .is_creation_fee_destination(creation_fee_destination.key()) @ StateError::InvalidCreationFeeDestination
    )]
    pub creation_fee_destination: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
}

/// Initializes one authority-selected subaccount ID and records it in the group count.
pub fn handle_create_subaccount(
    context: Context<CreateSubaccount>,
    subaccount_id: u16,
) -> Result<()> {
    let creation_fee_lamports = context
        .accounts
        .global_governance
        .load()?
        .subaccount_creation_fee_lamports;

    if creation_fee_lamports != 0 {
        anchor_lang::system_program::transfer(
            CpiContext::new(
                anchor_lang::system_program::ID,
                anchor_lang::system_program::Transfer {
                    from: context.accounts.authority.to_account_info(),
                    to: context.accounts.creation_fee_destination.to_account_info(),
                },
            ),
            creation_fee_lamports,
        )?;
    }

    let group_id = context.accounts.margin_group.load()?.group_id;
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
