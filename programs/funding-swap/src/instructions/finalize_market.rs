use crate::instructions::refresh_market::read_fee_price;
use crate::state::{MarketState, StateError, MARKET_ACTIVE, MARKET_CLOSE_ONLY};
use anchor_lang::prelude::*;

#[derive(Accounts)]
pub struct FinalizeMarket<'info> {
    #[account(mut)]
    pub market: AccountLoader<'info, MarketState>,
}

#[derive(Accounts)]
pub struct FinalizeMarkets {}

fn finalize_one_market<'info>(
    loader: &AccountLoader<'info, MarketState>,
    accounts: &'info [AccountInfo<'info>],
    now: i64,
) -> Result<usize> {
    let market = loader.load()?;

    if now < market.config.maturity_timestamp {
        return Err(StateError::MaturityNotReached.into());
    }

    if !matches!(market.status.0, MARKET_ACTIVE | MARKET_CLOSE_ONLY) {
        return Err(StateError::InvalidStatus.into());
    }

    let (consumed, price) = read_fee_price(&market, accounts, now)?;

    drop(market);
    let mut market = loader.load_mut()?;
    let maturity = market.config.maturity_timestamp;
    market.advance_fee_index_with_price(maturity, price)?;
    market.finalize(now)?;
    Ok(consumed)
}

pub fn handle_finalize_market<'info>(context: Context<'info, FinalizeMarket<'info>>) -> Result<()> {
    let consumed = finalize_one_market(
        &context.accounts.market,
        context.remaining_accounts,
        Clock::get()?.unix_timestamp,
    )?;

    if consumed != context.remaining_accounts.len() {
        return Err(StateError::InvalidRemainingAccounts.into());
    }

    Ok(())
}

/// Each market is followed by USDC custody, its Pyth update, and (for Phoenix) an underlying update.
pub fn handle_finalize_markets<'info>(context: Context<'info, FinalizeMarkets>) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let mut remaining = context.remaining_accounts;
    while let Some(info) = remaining.first() {
        let loader = AccountLoader::<MarketState>::try_from(info)?;
        remaining = &remaining[1..];
        let consumed = finalize_one_market(&loader, remaining, now)?;
        remaining = &remaining[consumed..];
    }
    Ok(())
}
