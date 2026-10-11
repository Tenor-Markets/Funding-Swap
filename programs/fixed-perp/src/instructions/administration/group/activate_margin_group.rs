use crate::state::*;
use anchor_lang::prelude::*;

#[derive(Accounts)]
pub struct ActivateMarginGroup<'info> {
    #[account(mut)]
    pub margin_group: AccountLoader<'info, MarginGroup>,
    #[account(
        mut,
        constraint = margin_group.load()?.is_governance_key(margin_group_governance.key()) @ StateError::InvalidKey
    )]
    pub margin_group_governance: AccountLoader<'info, MarginGroupGovernance>,
}

pub fn handle_activate_margin_group(context: Context<ActivateMarginGroup>) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;

    let mut group = context.accounts.margin_group.load_mut()?;

    group.activate(
        &mut context
            .accounts
            .margin_group_governance
            .load_mut()?
            .activation_window,
        now,
    )?;

    Ok(())
}
