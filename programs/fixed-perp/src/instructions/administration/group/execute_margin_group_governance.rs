// CHECKED
use crate::state::*;
use anchor_lang::prelude::*;

#[derive(Accounts)]
pub struct ExecuteMarginGroupGovernance<'info> {
    #[account(mut)]
    pub margin_group: AccountLoader<'info, MarginGroup>,
    #[account(
        mut,
        constraint = margin_group.load()?.is_governance_key(margin_group_governance.key()) @ StateError::InvalidKey
    )]
    pub margin_group_governance: AccountLoader<'info, MarginGroupGovernance>,
}

pub fn handle_execute_margin_group_governance(
    context: Context<ExecuteMarginGroupGovernance>,
) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;

    let mut group = context.accounts.margin_group.load_mut()?;

    context
        .accounts
        .margin_group_governance
        .load_mut()?
        .group_margin
        .execute_into(&mut group, now)?;

    Ok(())
}
