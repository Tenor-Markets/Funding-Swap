use super::super::common::{add_change, optional_fixed, LiquidationGovernanceArgs};
use crate::state::*;
use anchor_lang::prelude::*;

#[derive(Accounts)]
pub struct ProposeLiquidation<'info> {
    pub authority: Signer<'info>,
    #[account(constraint = global_governance.load()?.authority_for(GovernanceCategory::Liquidation, authority.key()) @ StateError::InvalidAuthority)]
    pub global_governance: AccountLoader<'info, GlobalGovernanceConfig>,
    #[account(constraint = !market.load()?.is_pending() @ StateError::InvalidStatus)]
    pub market: AccountLoader<'info, MarketState>,
    #[account(
        mut,
        constraint = market.load()?.is_governance_key(market_governance.key()) @ StateError::InvalidKey
    )]
    pub market_governance: AccountLoader<'info, MarketGovernance>,
}

pub fn handle_propose_liquidation(
    context: Context<ProposeLiquidation>,
    args: LiquidationGovernanceArgs,
) -> Result<()> {
    // Context binds the category authority, canonical market, and its
    // resource-local governance PDA.
    let now = Clock::get()?.unix_timestamp;
    let global = context.accounts.global_governance.load()?;
    let market = context.accounts.market.load()?;
    let proposal = LiquidationProposal {
        base_incentive: optional_fixed(args.base_incentive_raw),
        incentive_slope: optional_fixed(args.incentive_slope_raw),
        fee_rate: optional_fixed(args.fee_rate_raw),
        ..Default::default()
    };
    let mut changes = [(0, 0, ChangePolicy::BothDelayed); 3];
    let mut count = 0;
    add_change!(
        &mut changes,
        &mut count,
        args.base_incentive_raw,
        market.config.liquidation_base_incentive.to_raw_bits(),
        ChangePolicy::BothDelayed
    );
    add_change!(
        &mut changes,
        &mut count,
        args.incentive_slope_raw,
        market.config.liquidation_incentive_slope.to_raw_bits(),
        ChangePolicy::BothDelayed
    );
    add_change!(
        &mut changes,
        &mut count,
        args.fee_rate_raw,
        market.config.liquidation_fee_rate.to_raw_bits(),
        ChangePolicy::BothDelayed
    );
    global.replace_typed_proposal_value(
        &mut context.accounts.market_governance.load_mut()?.liquidation,
        proposal,
        GovernanceCategory::Liquidation,
        now,
        &changes[..count],
    )?;
    Ok(())
}
