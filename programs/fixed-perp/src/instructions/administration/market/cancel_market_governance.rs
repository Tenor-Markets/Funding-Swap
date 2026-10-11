use crate::state::*;
use anchor_lang::prelude::*;

fn is_category_authority(
    governance: &GlobalGovernanceConfig,
    category: u8,
    authority: Pubkey,
) -> bool {
    GovernanceCategory::try_from(category)
        .is_ok_and(|category| governance.authority_for(category, authority))
}

#[derive(Accounts)]
#[instruction(category: u8)]
pub struct CancelMarketGovernance<'info> {
    pub authority: Signer<'info>,
    #[account(
        constraint = is_category_authority(&*global_governance.load()?, category, authority.key())
            @ StateError::InvalidAuthority
    )]
    pub global_governance: AccountLoader<'info, GlobalGovernanceConfig>,
    pub market: AccountLoader<'info, MarketState>,
    #[account(
        mut,
        constraint = market.load()?.is_governance_key(market_governance.key()) @ StateError::InvalidKey
    )]
    pub market_governance: AccountLoader<'info, MarketGovernance>,
}

pub fn handle_cancel_market_governance(
    context: Context<CancelMarketGovernance>,
    category: u8,
) -> Result<()> {
    let category = GovernanceCategory::try_from(category)?;
    let mut governance = context.accounts.market_governance.load_mut()?;
    match category {
        GovernanceCategory::MarketMargin => governance.market_margin.cancel(),
        GovernanceCategory::Liquidation => governance.liquidation.cancel(),
        GovernanceCategory::FeeRate => governance.fee_rate.cancel(),
        GovernanceCategory::MarketRisk => governance.market_risk.cancel(),
        GovernanceCategory::MarketStatus => governance.market_status.cancel(),
        _ => return Err(StateError::InvalidTag.into()),
    }
    Ok(())
}
