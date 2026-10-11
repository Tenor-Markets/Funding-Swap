use anchor_lang::prelude::*;

use super::super::common::creation_window;
use crate::state::{
    AssetState, GlobalGovernanceConfig, GovernanceCategory, MarginGroup, MarginGroupGovernance,
    StateError,
};

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
    #[account(constraint = global_governance.load()?.authority_for(GovernanceCategory::Creation, authority.key()) @ StateError::InvalidAuthority)]
    pub global_governance: AccountLoader<'info, GlobalGovernanceConfig>,
    #[account(init, payer = authority, space = MarginGroup::SPACE, seeds = [b"margin_group", args.group_id.to_le_bytes().as_ref()], bump)]
    pub margin_group: AccountLoader<'info, MarginGroup>,
    #[account(
        init,
        payer = authority,
        space = MarginGroupGovernance::SPACE,
        seeds = [b"margin_group_governance", margin_group.key().as_ref()],
        bump
    )]
    pub margin_group_governance: AccountLoader<'info, MarginGroupGovernance>,
    #[account(constraint = collateral_asset.load()?.is_active() @ StateError::InvalidStatus)]
    pub collateral_asset: AccountLoader<'info, AssetState>,
    #[account(constraint = settlement_asset.load()?.is_active() @ StateError::InvalidStatus)]
    pub settlement_asset: AccountLoader<'info, AssetState>,
    pub system_program: Program<'info, System>,
}

pub fn handle_create_margin_group(
    context: Context<CreateMarginGroup>,
    args: CreateMarginGroupArgs,
) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let global = context.accounts.global_governance.load()?;

    let mut group = context.accounts.margin_group.load_init()?;
    group.initialize_in_place(
        context.bumps.margin_group,
        args.group_id,
        context.accounts.collateral_asset.key(),
        context.accounts.settlement_asset.key(),
        context.accounts.margin_group_governance.key(),
        crate::state::FixedPoint::from_raw_bits(args.critical_health_ratio_raw),
        crate::state::FixedPoint::from_raw_bits(args.risky_order_cancellation_ratio_raw),
    )?;

    context
        .accounts
        .margin_group_governance
        .load_init()?
        .initialize_in_place(creation_window(&global, now)?)?;

    Ok(())
}
