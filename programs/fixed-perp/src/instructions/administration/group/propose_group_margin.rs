// CHECKED
use super::super::common::{add_change, optional_fixed, GroupMarginGovernanceArgs};
use crate::state::*;
use anchor_lang::prelude::*;

#[derive(Accounts)]
pub struct ProposeGroupMargin<'info> {
    pub authority: Signer<'info>,
    #[account(constraint = global_governance.load()?.authority_for(GovernanceCategory::GroupMargin, authority.key()) @ StateError::InvalidAuthority)]
    pub global_governance: AccountLoader<'info, GlobalGovernanceConfig>,
    #[account(constraint = !margin_group.load()?.is_pending() @ StateError::InvalidStatus)]
    pub margin_group: AccountLoader<'info, MarginGroup>,
    #[account(
        mut,
        constraint = margin_group.load()?.is_governance_key(margin_group_governance.key()) @ StateError::InvalidKey
    )]
    pub margin_group_governance: AccountLoader<'info, MarginGroupGovernance>,
}

pub fn handle_propose_group_margin(
    context: Context<ProposeGroupMargin>,
    args: GroupMarginGovernanceArgs,
) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;

    let global = context.accounts.global_governance.load()?;

    let group = context.accounts.margin_group.load()?;

    let proposal = GroupMarginProposal {
        critical_health_ratio: optional_fixed(args.critical_health_ratio_raw),
        risky_order_cancellation_ratio: optional_fixed(args.risky_order_cancellation_ratio_raw),
        ..Default::default()
    };

    let mut count = 0;
    let mut changes = [(0, 0, ChangePolicy::BothDelayed); 2];

    add_change!(
        &mut changes,
        &mut count,
        args.critical_health_ratio_raw,
        group.critical_health_ratio.to_raw_bits(),
        ChangePolicy::IncreaseProtective
    );
    add_change!(
        &mut changes,
        &mut count,
        args.risky_order_cancellation_ratio_raw,
        group.risky_order_cancellation_ratio.to_raw_bits(),
        ChangePolicy::IncreaseProtective
    );

    global.replace_typed_proposal_value(
        &mut context
            .accounts
            .margin_group_governance
            .load_mut()?
            .group_margin,
        proposal,
        GovernanceCategory::GroupMargin,
        now,
        &changes[..count],
    )?;

    Ok(())
}
