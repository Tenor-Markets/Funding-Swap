// CHECKED
use anchor_lang::prelude::*;

use crate::instructions::oracle::{
    read_asset_observation, settlement_ratio_with_guards, underlying_settlement_price,
};
use crate::state::{MarketState, StateError, MARKET_ACTIVE, MARKET_CLOSE_ONLY};

#[derive(Accounts)]
pub struct FinalizeMarket<'info> {
    #[account(mut)]
    pub market: AccountLoader<'info, MarketState>,
}

#[derive(Accounts)]
pub struct FinalizeMarkets {}

pub fn handle_finalize_market<'info>(context: Context<'info, FinalizeMarket<'info>>) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    finalize_one_market(&context.accounts.market, context.remaining_accounts, now)
}

fn ensure_finalization_ready(market: &MarketState, now: i64) -> Result<()> {
    if now < market.config.maturity_timestamp
        || !matches!(market.status.0, MARKET_ACTIVE | MARKET_CLOSE_ONLY)
    {
        return Err(error!(StateError::InvalidStatus));
    }
    Ok(())
}

fn finalize_one_market<'info>(
    market_loader: &AccountLoader<'info, MarketState>,
    remaining_accounts: &'info [AccountInfo<'info>],
    now: i64,
) -> Result<()> {
    let (collateral_key, settlement_key, underlying_key) = {
        let market_view = market_loader.load()?;
        ensure_finalization_ready(&market_view, now)?;
        (
            market_view.config.collateral_state,
            market_view.config.settlement_state,
            market_view.config.underlying_state,
        )
    };

    let (underlying_asset, remaining) =
        read_asset_observation(remaining_accounts, underlying_key, now)?;
    let (collateral_asset, remaining) = read_asset_observation(remaining, collateral_key, now)?;
    let (settlement_asset, _) = read_asset_observation(remaining, settlement_key, now)?;

    let (ratio, index_price) = {
        let collateral_guard = collateral_asset.loader.load()?;
        let settlement_guard = settlement_asset.loader.load()?;
        let ratio = settlement_ratio_with_guards(
            &collateral_asset,
            &settlement_asset,
            &collateral_guard,
            &settlement_guard,
        )?;
        let index_price = underlying_settlement_price(&underlying_asset, &settlement_asset)?;
        (ratio, index_price)
    };
    let mut market = market_loader.load_mut()?;
    let maturity = market.config.maturity_timestamp;
    market.advance_fee_index_with_price(maturity, index_price)?;
    market.freeze_settlement_ratio(ratio, now)?;
    Ok(())
}

/// Batched form used by keepers. Each market is followed immediately by its
/// underlying, collateral, and settlement AssetState/oracle pairs.
pub fn handle_finalize_markets<'info>(context: Context<'info, FinalizeMarkets>) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let mut remaining = context.remaining_accounts;

    while let Some(market_info) = remaining.first() {
        let loader = AccountLoader::<MarketState>::try_from(market_info)
            .map_err(|_| error!(StateError::InvalidMarket))?;
        remaining = &remaining[1..];

        let (collateral_key, settlement_key, underlying_key) = {
            let market = loader
                .load()
                .map_err(|_| error!(StateError::InvalidMarket))?;
            ensure_finalization_ready(&market, now)?;
            (
                market.config.collateral_state,
                market.config.settlement_state,
                market.config.underlying_state,
            )
        };

        let (underlying, remaining_after_underlying) =
            read_asset_observation(remaining, underlying_key, now)?;
        let (collateral, remaining_after_collateral) =
            read_asset_observation(remaining_after_underlying, collateral_key, now)?;
        let (settlement, rest) =
            read_asset_observation(remaining_after_collateral, settlement_key, now)?;
        remaining = rest;
        let collateral_guard = collateral.loader.load()?;
        let settlement_guard = settlement.loader.load()?;
        let ratio = settlement_ratio_with_guards(
            &collateral,
            &settlement,
            &collateral_guard,
            &settlement_guard,
        )?;
        let index_price = underlying_settlement_price(&underlying, &settlement)?;

        let mut market = loader.load_mut()?;
        let maturity = market.config.maturity_timestamp;
        market.advance_fee_index_with_price(maturity, index_price)?;
        market.freeze_settlement_ratio(ratio, now)?;
    }
    Ok(())
}
