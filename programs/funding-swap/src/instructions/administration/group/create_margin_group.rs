use anchor_lang::prelude::*;

use crate::state::{GlobalGovernanceConfig, GovernanceCategory, MarginGroup, StateError};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug)]
pub struct CreateMarginGroupArgs {
    pub group_id: u16,
    pub critical_health_ratio_raw: i128,
    pub risky_order_cancellation_ratio_raw: i128,
}

#[derive(Accounts)]
#[instruction(args: CreateMarginGroupArgs)]
pub struct CreateMarginGroup<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,
    #[account(
        constraint = global_governance.load()?.authority_for(GovernanceCategory::Resources, authority.key()) @ StateError::InvalidAuthority
    )]
    pub global_governance: AccountLoader<'info, GlobalGovernanceConfig>,
    #[account(
        init,
        payer = authority,
        space = MarginGroup::SPACE,
        seeds = [b"margin_group", args.group_id.to_le_bytes().as_ref()],
        bump
    )]
    pub margin_group: AccountLoader<'info, MarginGroup>,
    pub system_program: Program<'info, System>,
}

pub fn handle_create_margin_group(
    context: Context<CreateMarginGroup>,
    args: CreateMarginGroupArgs,
) -> Result<()> {
    let mut group = context.accounts.margin_group.load_init()?;

    group.initialize_in_place(
        context.bumps.margin_group,
        args.group_id,
        crate::state::FixedPoint::from_raw_bits(args.critical_health_ratio_raw),
        crate::state::FixedPoint::from_raw_bits(args.risky_order_cancellation_ratio_raw),
    )?;

    Ok(())
}
