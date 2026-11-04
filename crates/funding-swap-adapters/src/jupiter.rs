use core::convert::TryFrom;

use fixed::types::I64F64;
use funding_swap_math::{rate::RateDelta, FixedPoint};
use solana_pubkey::Pubkey;

use crate::{
    validate_observation_timestamps, AccountView, AdapterError, ObservationContext,
    ObservationLimits,
};

pub const JUPITER_PROGRAM_ID: Pubkey = Pubkey::new_from_array([
    5, 177, 243, 202, 241, 148, 98, 239, 135, 96, 240, 171, 222, 117, 205, 61, 158, 227, 27, 58,
    50, 198, 32, 232, 148, 18, 46, 156, 155, 129, 69, 250,
]);

pub const JUPITER_CUSTODY_DISCRIMINATOR: [u8; 8] = [1, 184, 48, 81, 93, 131, 63, 145];
pub const JUPITER_CUSTODY_ACCOUNT_LEN: usize = 2_000;
pub const JUPITER_RATE_SCALE: u128 = 1_000_000_000;
const JUPITER_DBPS_SCALE: u128 = 100_000;
const JUPITER_BPS_SCALE: u128 = 10_000;
const JUPITER_UTILIZATION_SCALE: u128 = 1_000_000_000;
const JUPITER_SECONDS_PER_HOUR: u128 = 3_600;
const JUPITER_HOURS_PER_YEAR: u128 = 24 * 365;

const POOL_OFFSET: usize = 8;
const MINT_OFFSET: usize = 40;
const DECIMALS_OFFSET: usize = 104;
const OWNED_OFFSET: usize = 222;
const LOCKED_OFFSET: usize = 230;
const FUNDING_RATE_STATE_OFFSET: usize = 262;
const JUMP_RATE_STATE_OFFSET: usize = 352;
const DEBT_OFFSET: usize = 1_004;
const BORROW_LEND_INTERESTS_ACCRUED_OFFSET: usize = 1_020;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JupiterLayoutVersion {
    V1,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JupiterSourceConfig {
    pub layout: JupiterLayoutVersion,
    pub custody: Pubkey,
    pub pool: Pubkey,
    pub mint: Pubkey,
    pub limits: ObservationLimits,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JupiterAccount<'a> {
    pub account: AccountView<'a>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JupiterAssets {
    pub owned: u64,
    pub locked: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FundingRateState {
    pub cumulative_interest_rate: u128,
    pub last_update: i64,
    pub hourly_funding_dbps: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JumpRateState {
    pub min_rate_bps: u64,
    pub max_rate_bps: u64,
    pub target_rate_bps: u64,
    pub target_utilization_rate: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JupiterCustody {
    pub pool: Pubkey,
    pub mint: Pubkey,
    pub decimals: u8,
    pub assets: JupiterAssets,
    pub funding_rate_state: FundingRateState,
    pub jump_rate_state: JumpRateState,
    pub debt: u128,
    pub borrow_lend_interests_accured: u128,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ValidatedJupiterSource {
    pub program: Pubkey,
    pub custody: JupiterCustody,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JupiterRateObservation {
    pub source_checkpoint: u128,
    pub source_delta: u128,
    pub source_last_update_timestamp: i64,
    pub rate_delta: RateDelta,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JupiterSeedObservation {
    pub source_checkpoint: u128,
    pub source_last_update_timestamp: i64,
}

pub fn validate_source(
    config: &JupiterSourceConfig,
    account: &JupiterAccount<'_>,
) -> Result<ValidatedJupiterSource, AdapterError> {
    if account.account.key != config.custody {
        return Err(AdapterError::InvalidAccountKey);
    }

    if account.account.owner != JUPITER_PROGRAM_ID {
        return Err(AdapterError::InvalidAccountOwner);
    }

    let custody = decode_custody(config.layout, account.account.data)?;

    if custody.pool != config.pool {
        return Err(AdapterError::InvalidPool);
    }

    if custody.mint != config.mint {
        return Err(AdapterError::InvalidMint);
    }

    Ok(ValidatedJupiterSource {
        program: JUPITER_PROGRAM_ID,
        custody,
    })
}

pub fn observe(
    config: &JupiterSourceConfig,
    account: &JupiterAccount<'_>,
    context: &ObservationContext,
) -> Result<JupiterRateObservation, AdapterError> {
    let source = validate_source(config, account)?;
    validate_observation_timestamps(
        *context,
        source.custody.funding_rate_state.last_update,
        config.limits,
    )?;
    let source_checkpoint = effective_checkpoint(source.custody, context.protocol_now)?;
    let source_delta = source_checkpoint
        .checked_sub(context.previous_source_checkpoint)
        .ok_or(AdapterError::DecreasingCheckpoint)?;
    if source_delta > config.limits.max_source_delta {
        return Err(AdapterError::SourceDeltaTooLarge);
    }

    Ok(JupiterRateObservation {
        source_checkpoint,
        source_delta,
        source_last_update_timestamp: source.custody.funding_rate_state.last_update,
        rate_delta: normalize_source_delta(source_delta)?,
    })
}

/// Authenticates a fresh source and returns its current cumulative checkpoint
/// without imposing a delta bound against pre-market history.
pub fn seed_observation(
    config: &JupiterSourceConfig,
    account: &JupiterAccount<'_>,
    protocol_now: i64,
) -> Result<JupiterSeedObservation, AdapterError> {
    let source = validate_source(config, account)?;

    validate_observation_timestamps(
        ObservationContext {
            previous_source_checkpoint: 0,
            previous_protocol_checkpoint_at: protocol_now,
            protocol_now,
        },
        source.custody.funding_rate_state.last_update,
        config.limits,
    )?;

    Ok(JupiterSeedObservation {
        source_checkpoint: effective_checkpoint(source.custody, protocol_now)?,
        source_last_update_timestamp: source.custody.funding_rate_state.last_update,
    })
}

pub fn effective_checkpoint(
    custody: JupiterCustody,
    protocol_now: i64,
) -> Result<u128, AdapterError> {
    let state = custody.funding_rate_state;

    if protocol_now <= state.last_update {
        return Ok(state.cumulative_interest_rate);
    }
    if protocol_now < 0 {
        return Err(AdapterError::InvalidClock);
    }
    if state.last_update < 0 {
        return Err(AdapterError::InvalidSourceAnchor);
    }

    let elapsed = u128::try_from(
        protocol_now
            .checked_sub(state.last_update)
            .ok_or(AdapterError::ArithmeticOverflow)?,
    )
    .map_err(|_| AdapterError::ArithmeticOverflow)?;
    let hourly_rate = current_hourly_rate(custody)?;
    let numerator = elapsed
        .checked_mul(hourly_rate)
        .ok_or(AdapterError::ArithmeticOverflow)?;
    let virtual_increment = checked_ceil_div(numerator, JUPITER_SECONDS_PER_HOUR)?;
    state
        .cumulative_interest_rate
        .checked_add(virtual_increment)
        .ok_or(AdapterError::ArithmeticOverflow)
}

pub fn normalize_source_delta(source_delta: u128) -> Result<RateDelta, AdapterError> {
    let whole = source_delta / JUPITER_RATE_SCALE;
    let remainder = source_delta % JUPITER_RATE_SCALE;
    let raw_bits = whole
        .checked_mul(1u128 << 64)
        .and_then(|whole_bits| {
            remainder
                .checked_mul(1u128 << 64)
                .and_then(|remainder_bits| remainder_bits.checked_div(JUPITER_RATE_SCALE))
                .and_then(|remainder_bits| whole_bits.checked_add(remainder_bits))
        })
        .ok_or(AdapterError::NormalizationOverflow)?;
    let raw_bits = i128::try_from(raw_bits).map_err(|_| AdapterError::NormalizationOverflow)?;
    Ok(RateDelta::from_i64f64(I64F64::from_bits(raw_bits)))
}

/// Applies the canonical underlying/settlement price to Jupiter's
/// dimensionless funding delta exactly once, preserving Q64.64 scaling.
pub fn normalize_source_delta_price_weighted(
    source_delta: u128,
    underlying_settlement_price: FixedPoint,
) -> Result<RateDelta, AdapterError> {
    let dimensionless = normalize_source_delta(source_delta)?.to_i64f64();
    let weighted = dimensionless
        .checked_mul(underlying_settlement_price.to_i64f64())
        .ok_or(AdapterError::NormalizationOverflow)?;
    Ok(RateDelta::from_i64f64(weighted))
}

fn current_hourly_rate(custody: JupiterCustody) -> Result<u128, AdapterError> {
    // Match Jupiter's getCurrentFundingRate operation order: a custody with no
    // raw assets owned has no current funding rate, even if debt would make the
    // adjusted utilization inputs nonzero.
    if custody.assets.owned == 0 {
        return Ok(0);
    }

    let debt = if custody.debt > custody.borrow_lend_interests_accured {
        custody
            .debt
            .checked_sub(custody.borrow_lend_interests_accured)
            .ok_or(AdapterError::ArithmeticOverflow)?
    } else {
        0
    };
    let debt = checked_ceil_div(debt, JUPITER_RATE_SCALE)?;
    let owned = u128::from(custody.assets.owned)
        .checked_add(debt)
        .ok_or(AdapterError::ArithmeticOverflow)?;
    let locked = u128::from(custody.assets.locked)
        .checked_add(debt)
        .ok_or(AdapterError::ArithmeticOverflow)?;
    if owned == 0 {
        return Ok(0);
    }
    if locked > owned {
        return Err(AdapterError::InvalidUtilization);
    }

    if custody.funding_rate_state.hourly_funding_dbps != 0 {
        // Linear borrow rate mechanism.
        let hourly_funding_rate = u128::from(custody.funding_rate_state.hourly_funding_dbps)
            .checked_mul(JUPITER_RATE_SCALE)
            .ok_or(AdapterError::ArithmeticOverflow)?
            / JUPITER_DBPS_SCALE;
        return checked_ceil_div(
            locked
                .checked_mul(hourly_funding_rate)
                .ok_or(AdapterError::ArithmeticOverflow)?,
            owned,
        );
    }

    // Jump-rate borrow mechanism.
    let jump = custody.jump_rate_state;
    let min_rate = u128::from(jump.min_rate_bps);
    let max_rate = u128::from(jump.max_rate_bps);
    let target_rate = u128::from(jump.target_rate_bps);
    let target_utilization = u128::from(jump.target_utilization_rate);
    if min_rate > target_rate
        || target_rate > max_rate
        || target_utilization == 0
        || target_utilization > JUPITER_UTILIZATION_SCALE
    {
        return Err(AdapterError::InvalidRateConfig);
    }

    let utilization = locked
        .checked_mul(JUPITER_UTILIZATION_SCALE)
        .ok_or(AdapterError::ArithmeticOverflow)?
        / owned;
    let yearly_rate_bps = if utilization <= target_utilization {
        let slope = target_rate
            .checked_sub(min_rate)
            .ok_or(AdapterError::InvalidRateConfig)?;
        let interpolated = checked_ceil_div(
            slope
                .checked_mul(utilization)
                .ok_or(AdapterError::ArithmeticOverflow)?,
            target_utilization,
        )?;
        min_rate
            .checked_add(interpolated)
            .ok_or(AdapterError::ArithmeticOverflow)?
    } else {
        let denominator = JUPITER_UTILIZATION_SCALE
            .checked_sub(target_utilization)
            .ok_or(AdapterError::InvalidRateConfig)?;
        if denominator == 0 {
            return Err(AdapterError::InvalidRateConfig);
        }
        let rate_diff = max_rate
            .checked_sub(target_rate)
            .ok_or(AdapterError::InvalidRateConfig)?;
        let utilization_diff = utilization
            .checked_sub(target_utilization)
            .ok_or(AdapterError::ArithmeticOverflow)?;
        target_rate
            .checked_add(checked_ceil_div(
                rate_diff
                    .checked_mul(utilization_diff)
                    .ok_or(AdapterError::ArithmeticOverflow)?,
                denominator,
            )?)
            .ok_or(AdapterError::ArithmeticOverflow)?
    };
    let yearly_rate = yearly_rate_bps
        .checked_mul(JUPITER_RATE_SCALE)
        .ok_or(AdapterError::ArithmeticOverflow)?
        / JUPITER_BPS_SCALE;
    Ok(yearly_rate / JUPITER_HOURS_PER_YEAR)
}

fn decode_custody(
    _layout: JupiterLayoutVersion,
    data: &[u8],
) -> Result<JupiterCustody, AdapterError> {
    require_layout(data)?;

    Ok(JupiterCustody {
        pool: read_pubkey(data, POOL_OFFSET)?,
        mint: read_pubkey(data, MINT_OFFSET)?,
        decimals: read_u8(data, DECIMALS_OFFSET)?,
        assets: JupiterAssets {
            owned: read_u64(data, OWNED_OFFSET)?,
            locked: read_u64(data, LOCKED_OFFSET)?,
        },
        funding_rate_state: FundingRateState {
            cumulative_interest_rate: read_u128(data, FUNDING_RATE_STATE_OFFSET)?,
            last_update: read_i64(data, FUNDING_RATE_STATE_OFFSET + 16)?,
            hourly_funding_dbps: read_u64(data, FUNDING_RATE_STATE_OFFSET + 24)?,
        },
        jump_rate_state: JumpRateState {
            min_rate_bps: read_u64(data, JUMP_RATE_STATE_OFFSET)?,
            max_rate_bps: read_u64(data, JUMP_RATE_STATE_OFFSET + 8)?,
            target_rate_bps: read_u64(data, JUMP_RATE_STATE_OFFSET + 16)?,
            target_utilization_rate: read_u64(data, JUMP_RATE_STATE_OFFSET + 24)?,
        },
        debt: read_u128(data, DEBT_OFFSET)?,
        borrow_lend_interests_accured: read_u128(data, BORROW_LEND_INTERESTS_ACCRUED_OFFSET)?,
    })
}

fn require_layout(data: &[u8]) -> Result<(), AdapterError> {
    if data.len() != JUPITER_CUSTODY_ACCOUNT_LEN {
        return Err(AdapterError::InvalidAccountLength);
    }

    if data.get(..8) != Some(JUPITER_CUSTODY_DISCRIMINATOR.as_slice()) {
        return Err(AdapterError::InvalidDiscriminator);
    }

    Ok(())
}

fn read_array<const N: usize>(data: &[u8], offset: usize) -> Result<[u8; N], AdapterError> {
    data.get(offset..offset + N)
        .ok_or(AdapterError::InvalidAccountLength)?
        .try_into()
        .map_err(|_| AdapterError::InvalidAccountLength)
}

fn read_pubkey(data: &[u8], offset: usize) -> Result<Pubkey, AdapterError> {
    Ok(Pubkey::new_from_array(read_array(data, offset)?))
}

fn read_u8(data: &[u8], offset: usize) -> Result<u8, AdapterError> {
    data.get(offset)
        .copied()
        .ok_or(AdapterError::InvalidAccountLength)
}

fn read_u64(data: &[u8], offset: usize) -> Result<u64, AdapterError> {
    Ok(u64::from_le_bytes(read_array(data, offset)?))
}

fn read_i64(data: &[u8], offset: usize) -> Result<i64, AdapterError> {
    Ok(i64::from_le_bytes(read_array(data, offset)?))
}

fn read_u128(data: &[u8], offset: usize) -> Result<u128, AdapterError> {
    Ok(u128::from_le_bytes(read_array(data, offset)?))
}

fn checked_ceil_div(value: u128, divisor: u128) -> Result<u128, AdapterError> {
    if divisor == 0 {
        return Err(AdapterError::ArithmeticOverflow);
    }
    if value == 0 {
        return Ok(0);
    }
    value
        .checked_sub(1)
        .and_then(|adjusted| adjusted.checked_div(divisor))
        .and_then(|quotient| quotient.checked_add(1))
        .ok_or(AdapterError::ArithmeticOverflow)
}
