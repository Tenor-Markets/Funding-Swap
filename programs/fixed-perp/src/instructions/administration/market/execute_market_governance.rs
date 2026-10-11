use crate::state::*;
use anchor_lang::prelude::*;

#[derive(Accounts)]
pub struct ExecuteMarketGovernance<'info> {
    #[account(mut)]
    pub market: AccountLoader<'info, MarketState>,
    #[account(
        mut,
        constraint = market.load()?.is_governance_key(market_governance.key()) @ StateError::InvalidKey
    )]
    pub market_governance: AccountLoader<'info, MarketGovernance>,
}

pub fn handle_execute_market_governance(
    context: Context<ExecuteMarketGovernance>,
    category: u8,
) -> Result<()> {
    // Context binds the typed market and governance records; ready
    // category execution is permissionless.
    let now = Clock::get()?.unix_timestamp;
    let category = GovernanceCategory::try_from(category)?;
    let mut market = context.accounts.market.load_mut()?;
    let mut governance = context.accounts.market_governance.load_mut()?;
    match category {
        GovernanceCategory::MarketMargin => governance
            .market_margin
            .execute_into(&mut market.config, now)?,
        GovernanceCategory::Liquidation => governance
            .liquidation
            .execute_into(&mut market.config, now)?,
        GovernanceCategory::FeeRate => governance.fee_rate.execute_into(&mut market.config, now)?,
        GovernanceCategory::MarketRisk => governance
            .market_risk
            .execute_into(&mut market.config, now)?,
        GovernanceCategory::MarketStatus => {
            governance.market_status.execute_into(&mut market, now)?
        }
        _ => return Err(StateError::InvalidTag.into()),
    }
    Ok(())
}
