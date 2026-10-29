use anchor_lang::prelude::*;

use crate::state::{
    usdc_asset_address, AssetState, MarketState, OracleObservation, StateError, MAX_MARKETS,
};

pub(crate) struct MarketObservation<'info> {
    pub(crate) loader: AccountLoader<'info, MarketState>,
    pub(crate) market_id: u64,
}

/// USDC custody and its Pyth update precede up to N market accounts.
/// Liquidation and deleveraging allow the union of two portfolios.
pub(crate) struct ObservationSet<'info, const N: usize = MAX_MARKETS> {
    pub(crate) markets: [Option<MarketObservation<'info>>; N],
    pub(crate) count: usize,
}

/// Risk-sensitive actions count USDC at one only after authenticating its peg.
pub(crate) fn read_usdc_observation<'info>(
    accounts: &'info [AccountInfo<'info>],
    now: i64,
) -> Result<&'info [AccountInfo<'info>]> {
    let (asset_info, remaining) = accounts
        .split_first()
        .ok_or(StateError::InvalidRemainingAccounts)?;
    if asset_info.key() != usdc_asset_address() {
        return Err(StateError::InvalidAccountKey.into());
    }
    let loader = AccountLoader::<AssetState>::try_from(asset_info)?;
    let asset = loader.load()?;

    if !asset.is_active() {
        return Err(StateError::InvalidStatus.into());
    }

    let (feed, remaining) = remaining
        .split_first()
        .ok_or(StateError::InvalidRemainingAccounts)?;

    let observation = OracleObservation::from_pyth_account(feed, &asset.oracle, now)?;

    asset.validate_usdc_price(observation)?;

    Ok(remaining)
}

pub(crate) fn ensure_markets_not_paused<'info, const N: usize>(
    observations: &ObservationSet<'info, N>,
) -> Result<()> {
    for entry in observations.markets[..observations.count].iter().flatten() {
        if entry.loader.load()?.is_paused() {
            return Err(StateError::InvalidStatus.into());
        }
    }

    Ok(())
}

pub(crate) fn market_observation_for<'set, 'info, const N: usize>(
    observations: &'set ObservationSet<'info, N>,
    market_id: u64,
) -> Result<&'set MarketObservation<'info>> {
    observations.markets[..observations.count]
        .iter()
        .flatten()
        .find(|entry| entry.market_id == market_id)
        .ok_or_else(|| error!(StateError::MissingMarketObservation))
}

#[inline(never)]
pub(crate) fn read_observation_set<'info, const N: usize>(
    accounts: &'info [AccountInfo<'info>],
    now: i64,
) -> Result<(ObservationSet<'info, N>, &'info [AccountInfo<'info>])> {
    let mut remaining = read_usdc_observation(accounts, now)?;
    let mut markets = core::array::from_fn(|_| None);
    let mut count = 0;
    while count < N {
        let Some(info) = remaining.first() else {
            break;
        };
        let Ok(loader) = AccountLoader::<MarketState>::try_from(info) else {
            break;
        };
        let market = loader.load()?;
        market.ensure_index_fresh(now)?;
        let market_id = market.config.market_id;
        drop(market);
        markets[count] = Some(MarketObservation { loader, market_id });

        count += 1;
        remaining = &remaining[1..];
    }
    Ok((ObservationSet { markets, count }, remaining))
}
