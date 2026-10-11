use super::super::common::{add_change, optional_fixed, FeeRateGovernanceArgs};
use crate::state::*;
use anchor_lang::prelude::*;

#[derive(Accounts)]
pub struct ProposeFeeRate<'info> {
    pub authority: Signer<'info>,
    #[account(constraint = global_governance.load()?.authority_for(GovernanceCategory::FeeRate, authority.key()) @ StateError::InvalidAuthority)]
    pub global_governance: AccountLoader<'info, GlobalGovernanceConfig>,
    #[account(constraint = !market.load()?.is_pending() @ StateError::InvalidStatus)]
    pub market: AccountLoader<'info, MarketState>,
    #[account(
        mut,
        constraint = market.load()?.is_governance_key(market_governance.key()) @ StateError::InvalidKey
    )]
    pub market_governance: AccountLoader<'info, MarketGovernance>,
}

pub fn handle_propose_fee_rate(
    context: Context<ProposeFeeRate>,
    args: FeeRateGovernanceArgs,
) -> Result<()> {
    // Context binds the category authority, canonical market, and its
    // resource-local governance PDA.
    let now = Clock::get()?.unix_timestamp;
    let global = context.accounts.global_governance.load()?;
    let market = context.accounts.market.load()?;
    let proposal = FeeRateProposal {
        taker_fee_rate: optional_fixed(args.taker_fee_rate_raw),
        annualized_settlement_fee_rate: optional_fixed(args.annualized_settlement_fee_rate_raw),
        rate_floor: optional_fixed(args.rate_floor_raw),
        ..Default::default()
    };
    let mut changes = [(0, 0, ChangePolicy::BothDelayed); 3];
    let mut count = 0;
    add_change!(
        &mut changes,
        &mut count,
        args.taker_fee_rate_raw,
        market.config.taker_fee_rate.to_raw_bits(),
        ChangePolicy::BothDelayed
    );
    add_change!(
        &mut changes,
        &mut count,
        args.annualized_settlement_fee_rate_raw,
        market.config.annualized_settlement_fee_rate.to_raw_bits(),
        ChangePolicy::BothDelayed
    );
    add_change!(
        &mut changes,
        &mut count,
        args.rate_floor_raw,
        market.config.rate_floor.to_raw_bits(),
        ChangePolicy::IncreaseProtective
    );
    global.replace_typed_proposal_value(
        &mut context.accounts.market_governance.load_mut()?.fee_rate,
        proposal,
        GovernanceCategory::FeeRate,
        now,
        &changes[..count],
    )?;
    Ok(())
}
