use core::convert::TryFrom;

use anchor_lang::prelude::*;
use fixed_perp_adapters::{
    jupiter::{self, JupiterAccount, JupiterLayoutVersion, JupiterSourceConfig},
    phoenix::{self, PhoenixLayout, PhoenixSourceConfig, PhoenixSourceContext},
    AccountView, ObservationContext, ObservationLimits,
};

use crate::instructions::oracle::{read_asset_observation, underlying_settlement_price};
use crate::state::{MarketState, StateError, VenueAdapterKind, ADAPTER_JUPITER, ADAPTER_PHOENIX};

const REFRESH_EPOCH_SECONDS: i64 = 3_600;

#[derive(Accounts)]
pub struct RefreshMarket<'info> {
    #[account(
        mut,
        constraint = market.load()?.may_refresh() @ StateError::InvalidStatus
    )]
    pub market: AccountLoader<'info, MarketState>,
}

#[derive(Accounts)]
pub struct SeedMarketSource<'info> {
    #[account(
        mut,
        constraint = market.load()?.may_seed_source() @ StateError::InvalidStatus
    )]
    pub market: AccountLoader<'info, MarketState>,
}

fn current_epoch(market: &MarketState, now: i64) -> Result<u64> {
    let elapsed = now
        .checked_sub(market.epoch_origin_timestamp)
        .ok_or(StateError::InvalidClock)?;

    if elapsed < 0 {
        return Err(StateError::InvalidClock.into());
    }

    u64::try_from(elapsed / REFRESH_EPOCH_SECONDS)
        .map_err(|_| StateError::ArithmeticOverflow.into())
}

fn source_view<'a>(account: &'a AccountInfo<'_>, data: &'a [u8]) -> AccountView<'a> {
    AccountView {
        key: account.key(),
        owner: *account.owner,
        data,
    }
}

/// Stores a signed Phoenix checkpoint losslessly in the venue-agnostic field.
fn encode_signed_checkpoint(checkpoint: i64) -> u128 {
    u128::from_le_bytes(i128::from(checkpoint).to_le_bytes())
}

/// Recovers a Phoenix checkpoint and rejects values not produced from an i64.
fn decode_signed_checkpoint(checkpoint: u128) -> Result<i64> {
    let signed = i128::from_le_bytes(checkpoint.to_le_bytes());

    i64::try_from(signed).map_err(|_| StateError::InvalidSourceCheckpoint.into())
}

/// Reads the underlying and settlement observations before the venue source
/// accounts and returns the price used to build settlement-denominated index
/// deltas. Fixed USD Jupiter assets therefore produce a unit price.
fn read_index_price<'info>(
    market: &MarketState,
    accounts: &'info [AccountInfo<'info>],
    now: i64,
) -> Result<(usize, fixed_perp_math::FixedPoint)> {
    let (underlying, remaining) =
        read_asset_observation(accounts, market.config.underlying_state, now)?;

    let (settlement, remaining) =
        read_asset_observation(remaining, market.config.settlement_state, now)?;

    let price = underlying_settlement_price(&underlying, &settlement)?;

    Ok((accounts.len() - remaining.len(), price))
}

pub fn handle_refresh_market<'info>(context: Context<'info, RefreshMarket<'info>>) -> Result<()> {
    // The typed writable market and adapter-specific source
    // accounts are authenticated exactly before the staged index transition.
    let clock = Clock::get()?;
    let now = clock.unix_timestamp;
    let market = context.accounts.market.load()?;

    // Once maturity is reached, the last accepted index is authoritative.
    // A late refresh is therefore a successful no-op; finalization records
    // the terminal settlement ratio separately.
    if now >= market.config.maturity_timestamp {
        return Ok(());
    }

    let epoch = current_epoch(&market, now)?;

    if epoch <= market.checkpoint_epoch {
        return Ok(());
    }

    let (source_offset, underlying_settlement_price) =
        read_index_price(&market, context.remaining_accounts, now)?;

    let adapter_kind = market.config.adapter_kind();
    let previous_checkpoint = market.source_native_checkpoint;
    let previous_protocol_checkpoint_at = market.protocol_checkpoint_timestamp;

    let (rate_delta, next_checkpoint, source_timestamp, already_weighted) = match adapter_kind {
        VenueAdapterKind(ADAPTER_JUPITER) => {
            if context
                .remaining_accounts
                .len()
                .saturating_sub(source_offset)
                != 1
            {
                return Err(StateError::InvalidRemainingAccounts.into());
            }

            let payload = *market.config.adapter.jupiter()?;
            let source_config = JupiterSourceConfig {
                layout: JupiterLayoutVersion::V1,
                custody: payload.custody,
                pool: payload.pool,
                mint: payload.mint,
                limits: ObservationLimits {
                    max_source_delta: payload.max_source_delta,
                    max_source_age_seconds: payload.max_source_age_seconds,
                },
            };

            let custody_info = &context.remaining_accounts[source_offset];

            let custody_data = custody_info.try_borrow_data()?;

            let account = JupiterAccount {
                account: source_view(custody_info, &custody_data),
            };

            let observation = jupiter::observe(
                &source_config,
                &account,
                &ObservationContext {
                    previous_source_checkpoint: previous_checkpoint,
                    previous_protocol_checkpoint_at,
                    protocol_now: now,
                },
            )?;

            (
                observation.rate_delta,
                observation.source_checkpoint,
                observation.source_last_update_timestamp,
                false,
            )
        }
        VenueAdapterKind(ADAPTER_PHOENIX) => {
            if context
                .remaining_accounts
                .len()
                .saturating_sub(source_offset)
                != 2
            {
                return Err(StateError::InvalidRemainingAccounts.into());
            }

            let payload = *market.config.adapter.phoenix()?;

            let source_config = PhoenixSourceConfig {
                layout: PhoenixLayout::V1,
                global_configuration: payload.global_configuration,
                perp_asset_map: payload.perp_asset_map,
                canonical_quote_mint: payload.canonical_quote_mint,
                asset_slot: payload.asset_slot,
                symbol: payload.symbol,
                asset_id: payload.asset_id,
                market_account: payload.market_account,
                base_lot_decimals: payload.base_lot_decimals,
                tick_size: payload.tick_size,
                funding_interval_seconds: payload.funding_interval_seconds,
                funding_period_seconds: payload.funding_period_seconds,
                max_abs_native_delta: payload.max_abs_native_delta,
                max_funding_age_seconds: payload.max_funding_age_seconds,
                max_mark_age_slots: payload.max_mark_age_slots,
            };

            let previous_signed = decode_signed_checkpoint(previous_checkpoint)?;
            let global_info = &context.remaining_accounts[source_offset];
            let asset_map_info = &context.remaining_accounts[source_offset + 1];

            if global_info.key() == asset_map_info.key() {
                return Err(StateError::DuplicateSourceAccount.into());
            }

            let global_data = global_info.try_borrow_data()?;

            let asset_map_data = asset_map_info.try_borrow_data()?;

            let observation = phoenix::observe(
                &source_config,
                &PhoenixSourceContext {
                    protocol_now: now,
                    protocol_slot: clock.slot,
                },
                source_view(global_info, &global_data),
                source_view(asset_map_info, &asset_map_data),
                previous_signed,
            )?;

            let source_timestamp = i64::try_from(observation.last_funding_update_timestamp)
                .map_err(|_| StateError::InvalidClock)?;

            (
                observation.rate_delta,
                encode_signed_checkpoint(observation.source_checkpoint),
                source_timestamp,
                true,
            )
        }
        _ => return Err(StateError::InvalidAdapterKind.into()),
    };

    drop(market);

    // The state helper applies the authenticated current price and stages both
    // index transitions before committing any field.
    let mut market = context.accounts.market.load_mut()?;

    if already_weighted {
        market.apply_weighted_rate_observation_with_price(
            rate_delta,
            next_checkpoint,
            source_timestamp,
            epoch,
            now,
            underlying_settlement_price,
        )?;
    } else {
        market.apply_rate_observation_with_price(
            rate_delta,
            next_checkpoint,
            source_timestamp,
            epoch,
            now,
            underlying_settlement_price,
        )?;
    }

    Ok(())
}

/// Authenticates and records the current cumulative source checkpoint without
/// realizing any value that accrued before this protocol market existed.
pub fn handle_seed_market_source<'info>(
    context: Context<'info, SeedMarketSource<'info>>,
) -> Result<()> {
    // The typed initializing market and configured venue accounts
    // establish the initial checkpoint without realizing historical accrual.
    let clock = Clock::get()?;
    let now = clock.unix_timestamp;
    let market = context.accounts.market.load()?;

    let epoch = current_epoch(&market, now)?;
    let adapter_kind = market.config.adapter_kind();

    let (checkpoint, source_timestamp) = match adapter_kind {
        VenueAdapterKind(ADAPTER_JUPITER) => {
            if context.remaining_accounts.len() < 1 {
                return Err(StateError::InvalidRemainingAccounts.into());
            }

            let payload = *market.config.adapter.jupiter()?;

            let source_config = JupiterSourceConfig {
                layout: JupiterLayoutVersion::V1,
                custody: payload.custody,
                pool: payload.pool,
                mint: payload.mint,
                limits: ObservationLimits {
                    max_source_delta: payload.max_source_delta,
                    max_source_age_seconds: payload.max_source_age_seconds,
                },
            };

            let custody_info = &context.remaining_accounts[0];

            let custody_data = custody_info.try_borrow_data()?;

            let account = JupiterAccount {
                account: source_view(custody_info, &custody_data),
            };

            let observation = jupiter::seed_observation(&source_config, &account, now)?;

            (
                observation.source_checkpoint,
                observation.source_last_update_timestamp,
            )
        }
        VenueAdapterKind(ADAPTER_PHOENIX) => {
            if context.remaining_accounts.len() < 2 {
                return Err(StateError::InvalidRemainingAccounts.into());
            }

            let payload = *market.config.adapter.phoenix()?;

            let source_config = PhoenixSourceConfig {
                layout: PhoenixLayout::V1,
                global_configuration: payload.global_configuration,
                perp_asset_map: payload.perp_asset_map,
                canonical_quote_mint: payload.canonical_quote_mint,
                asset_slot: payload.asset_slot,
                symbol: payload.symbol,
                asset_id: payload.asset_id,
                market_account: payload.market_account,
                base_lot_decimals: payload.base_lot_decimals,
                tick_size: payload.tick_size,
                funding_interval_seconds: payload.funding_interval_seconds,
                funding_period_seconds: payload.funding_period_seconds,
                max_abs_native_delta: payload.max_abs_native_delta,
                max_funding_age_seconds: payload.max_funding_age_seconds,
                max_mark_age_slots: payload.max_mark_age_slots,
            };

            let global_info = &context.remaining_accounts[0];
            let asset_map_info = &context.remaining_accounts[1];

            if global_info.key() == asset_map_info.key() {
                return Err(StateError::DuplicateSourceAccount.into());
            }

            let global_data = global_info.try_borrow_data()?;

            let asset_map_data = asset_map_info.try_borrow_data()?;

            let source = phoenix::validate_source(
                &source_config,
                &PhoenixSourceContext {
                    protocol_now: now,
                    protocol_slot: clock.slot,
                },
                source_view(global_info, &global_data),
                source_view(asset_map_info, &asset_map_data),
            )?;

            (
                encode_signed_checkpoint(source.cumulative_funding_rate),
                i64::try_from(source.last_funding_update_timestamp)
                    .map_err(|_| StateError::InvalidClock)?,
            )
        }
        _ => return Err(StateError::InvalidAdapterKind.into()),
    };

    drop(market);

    context.accounts.market.load_mut()?.seed_source_checkpoint(
        checkpoint,
        source_timestamp,
        epoch,
        now,
    )?;

    Ok(())
}
