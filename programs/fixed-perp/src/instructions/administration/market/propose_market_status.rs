use super::super::common::{add_change, optional_u8, MarketStatusGovernanceArgs};
use crate::state::*;
use anchor_lang::prelude::*;

#[derive(Accounts)]
pub struct ProposeMarketStatus<'info> {
    pub authority: Signer<'info>,
    #[account(constraint = global_governance.load()?.authority_for(GovernanceCategory::MarketStatus, authority.key()) @ StateError::InvalidAuthority)]
    pub global_governance: AccountLoader<'info, GlobalGovernanceConfig>,
    #[account(constraint = !market.load()?.is_pending() @ StateError::InvalidStatus)]
    pub market: AccountLoader<'info, MarketState>,
    #[account(
        mut,
        constraint = market.load()?.is_governance_key(market_governance.key()) @ StateError::InvalidKey
    )]
    pub market_governance: AccountLoader<'info, MarketGovernance>,
}

pub fn handle_propose_market_status(
    context: Context<ProposeMarketStatus>,
    args: MarketStatusGovernanceArgs,
) -> Result<()> {
    // Context binds the market-status authority, canonical market, and its
    // resource-local governance PDA.
    let now = Clock::get()?.unix_timestamp;
    let global = context.accounts.global_governance.load()?;
    let market = context.accounts.market.load()?;
    let proposal = MarketStatusProposal {
        status: optional_u8(args.status),
        ..Default::default()
    };
    let mut changes = [(0, 0, ChangePolicy::BothDelayed); 1];
    let mut count = 0;
    add_change!(
        &mut changes,
        &mut count,
        args.status,
        market.status.0,
        if args.status == Some(crate::state::MARKET_PAUSED)
            || args.status == Some(crate::state::MARKET_CLOSE_ONLY)
        {
            ChangePolicy::IncreaseProtective
        } else {
            ChangePolicy::BothDelayed
        }
    );
    global.replace_typed_proposal_value(
        &mut context.accounts.market_governance.load_mut()?.market_status,
        proposal,
        GovernanceCategory::MarketStatus,
        now,
        &changes[..count],
    )?;
    Ok(())
}
