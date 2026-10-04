// CHECKED
use anchor_lang::prelude::*;

use crate::state::{
    AssetState, FixedPoint, MarketState, OracleObservation, SettlementRatio, StateError,
    MAX_MARKETS, PRICE_MODEL_FIXED,
};

#[derive(Clone)]
pub(crate) struct AssetObservation<'info> {
    pub(crate) loader: AccountLoader<'info, AssetState>,
    pub(crate) observation: OracleObservation,
}

/// One market followed immediately by its underlying AssetState and optional
/// Pyth update.
pub(crate) struct MarketObservation<'info> {
    pub(crate) loader: AccountLoader<'info, MarketState>,
    pub(crate) market_id: u32,
    pub(crate) underlying: AssetObservation<'info>,
}

/// All transient observations for one instruction. The account order is:
/// collateral AssetState/optional Pyth, settlement AssetState/optional Pyth,
/// then up to MAX_MARKETS market/underlying AssetState/optional Pyth triples.
pub(crate) struct ObservationSet<'info> {
    pub(crate) collateral: AssetObservation<'info>,
    pub(crate) settlement: AssetObservation<'info>,
    pub(crate) markets: [Option<MarketObservation<'info>>; MAX_MARKETS],
    pub(crate) count: usize,
}

#[inline(never)]
pub(crate) fn read_asset_observation<'info>(
    accounts: &'info [AccountInfo<'info>],
    expected_key: Pubkey,
    now: i64,
) -> Result<(AssetObservation<'info>, &'info [AccountInfo<'info>])> {
    let (asset_info, mut remaining) = accounts
        .split_first()
        .ok_or_else(|| error!(StateError::InvalidRemainingAccounts))?;

    let loader = AccountLoader::<AssetState>::try_from(asset_info)
        .map_err(|_| error!(StateError::InvalidAssetState))?;

    let observation = {
        let asset = loader
            .load()
            .map_err(|_| error!(StateError::InvalidAssetState))?;

        if asset_info.key() != expected_key {
            return Err(error!(StateError::InvalidAssetState));
        }

        if !asset.is_active() {
            return Err(error!(StateError::InvalidStatus));
        }

        if asset.price_model.0 == PRICE_MODEL_FIXED {
            OracleObservation::fixed(&asset, now)?
        } else {
            let (feed_info, rest) = remaining
                .split_first()
                .ok_or_else(|| error!(StateError::InvalidRemainingAccounts))?;

            remaining = rest;

            OracleObservation::from_pyth_account(feed_info, &asset, now)?
        }
    };

    Ok((
        AssetObservation {
            loader,
            observation,
        },
        remaining,
    ))
}

pub(crate) fn settlement_ratio(
    collateral: &AssetObservation<'_>,
    settlement: &AssetObservation<'_>,
) -> Result<SettlementRatio> {
    // One asset account cannot be borrowed twice. Identical keys are a unit
    // ratio and do not need either guard.
    if collateral.loader.key() == settlement.loader.key() {
        return Ok(SettlementRatio::new(1, 1)?);
    }

    let collateral_guard = collateral.loader.load()?;

    let settlement_guard = settlement.loader.load()?;

    settlement_ratio_with_guards(collateral, settlement, &collateral_guard, &settlement_guard)
}

/// Computes the ratio from guards already held by the caller. Identical asset
/// keys always produce a unit ratio.
pub(crate) fn settlement_ratio_with_guards(
    collateral: &AssetObservation<'_>,
    settlement: &AssetObservation<'_>,
    collateral_guard: &AssetState,
    settlement_guard: &AssetState,
) -> Result<SettlementRatio> {
    if collateral.loader.key() == settlement.loader.key() {
        return Ok(SettlementRatio::new(1, 1)?);
    }

    let settlement_price =
        OracleObservation::effective_price(settlement_guard, settlement.observation);

    let collateral_price =
        OracleObservation::effective_price(collateral_guard, collateral.observation);

    Ok(SettlementRatio::from_observations(
        settlement_price,
        settlement.observation,
        collateral_price,
        collateral.observation,
        collateral_guard.decimals,
        settlement_guard.decimals,
    )?)
}

/// Returns the configured underlying value in settlement units. Jupiter's
/// fixed-USD pair therefore produces exactly one.
pub(crate) fn underlying_settlement_price(
    underlying: &AssetObservation<'_>,
    settlement: &AssetObservation<'_>,
) -> Result<FixedPoint> {
    if underlying.loader.key() == settlement.loader.key() {
        return Ok(FixedPoint::from_raw_bits(1i128 << 64));
    }

    let underlying_guard = underlying.loader.load()?;

    let settlement_guard = settlement.loader.load()?;

    let underlying_price =
        OracleObservation::effective_price(&underlying_guard, underlying.observation);

    let settlement_price =
        OracleObservation::effective_price(&settlement_guard, settlement.observation);

    let ratio = underlying_price
        .to_i64f64()
        .checked_div(settlement_price.to_i64f64())
        .ok_or(StateError::ArithmeticOverflow)?;

    Ok(FixedPoint::from_i64f64(ratio))
}

pub(crate) fn market_observation_for<'set, 'info>(
    observations: &'set ObservationSet<'info>,
    market_id: u32,
) -> Result<&'set MarketObservation<'info>> {
    observations.markets[..observations.count]
        .iter()
        .flatten()
        .find(|entry| entry.market_id == market_id)
        .ok_or_else(|| error!(StateError::InvalidMarket))
}

/// Reads the observation prefix in one pass. The first two assets are the
/// group's collateral and settlement states; each following market is
/// immediately followed by its underlying state and optional Pyth update.
/// Returns any unconsumed suffix for callers that have another account type
/// after the observation prefix. Duplicate account metas are consumed normally.
#[inline(never)]
pub(crate) fn read_observation_set<'info>(
    accounts: &'info [AccountInfo<'info>],
    collateral_key: Pubkey,
    settlement_key: Pubkey,
    now: i64,
) -> Result<(ObservationSet<'info>, &'info [AccountInfo<'info>])> {
    let (collateral, mut remaining) = read_asset_observation(accounts, collateral_key, now)?;
    let (settlement, rest) = read_asset_observation(remaining, settlement_key, now)?;

    remaining = rest;

    let mut markets: [Option<MarketObservation<'info>>; MAX_MARKETS] =
        core::array::from_fn(|_| None);

    let mut count = 0usize;

    while count < MAX_MARKETS {
        let Some(market_info) = remaining.first() else {
            break;
        };

        let Ok(loader) = AccountLoader::<MarketState>::try_from(market_info) else {
            break;
        };

        remaining = &remaining[1..];

        let (market_id, underlying_key) = {
            let market = loader
                .load()
                .map_err(|_| error!(StateError::InvalidMarket))?;

            (market.config.market_id, market.config.underlying_state)
        };

        let (underlying, rest) = read_asset_observation(remaining, underlying_key, now)?;

        remaining = rest;

        markets[count] = Some(MarketObservation {
            loader,
            market_id,
            underlying,
        });

        count += 1;
    }

    Ok((
        ObservationSet {
            collateral,
            settlement,
            markets,
            count,
        },
        remaining,
    ))
}
