use core::convert::TryFrom;

use fixed_perp_math::rate::RateDelta;
use solana_pubkey::Pubkey;

use crate::{AccountView, AdapterError};

pub const PHOENIX_PROGRAM_ID: Pubkey = Pubkey::new_from_array([
    206, 112, 214, 251, 86, 142, 67, 235, 252, 150, 74, 144, 21, 108, 59, 233, 86, 59, 8, 23, 6,
    124, 171, 105, 78, 188, 131, 239, 221, 179, 126, 134,
]);

pub const PHOENIX_GLOBAL_CONFIGURATION: Pubkey = Pubkey::new_from_array([
    29, 175, 47, 92, 51, 210, 108, 155, 85, 255, 152, 249, 55, 122, 220, 47, 198, 18, 2, 159, 201,
    35, 13, 89, 37, 168, 225, 164, 90, 78, 71, 202,
]);

pub const PHOENIX_PERP_ASSET_MAP: Pubkey = Pubkey::new_from_array([
    26, 117, 53, 209, 220, 9, 238, 232, 249, 78, 141, 62, 120, 170, 231, 36, 82, 172, 237, 208,
    208, 236, 155, 218, 229, 208, 63, 208, 51, 37, 72, 209,
]);

pub const GLOBAL_CONFIGURATION_DISCRIMINATOR: [u8; 8] = [37, 146, 212, 210, 47, 136, 111, 20];
pub const PERP_ASSET_MAP_DISCRIMINATOR: [u8; 8] = [142, 54, 115, 223, 124, 239, 108, 38];
pub const GLOBAL_CONFIGURATION_ACCOUNT_LEN: usize = 2_560;
pub const PERP_ASSET_MAP_ACCOUNT_LEN: usize = 1_622_064;
pub const MAX_NUMBER_OF_PERP_ASSETS: u32 = 1_024;
pub const PERP_ASSET_MAP_HEADER_LEN: usize = 48;
pub const PERP_ASSET_ENTRY_STRIDE: usize = 1_584;
pub const PERP_ASSET_METADATA_LEN: usize = 1_568;
pub const PHOENIX_QUOTE_LOT_DECIMALS: u8 = 6;

const GLOBAL_CANONICAL_MINT_OFFSET: usize = 296;
const GLOBAL_ASSET_MAP_OFFSET: usize = 360;
const GLOBAL_ACCOUNT_KEY_OFFSET: usize = 8;
const MAP_SLOTS_USED_OFFSET: usize = 32;
const MAP_CAPACITY_OFFSET: usize = 40;
const MAP_NUM_ASSETS_OFFSET: usize = 24;
const ASSET_SYMBOL_LEN: usize = 16;
const ASSET_VALUE_OFFSET: usize = ASSET_SYMBOL_LEN;
const ASSET_MARK_SLOT_OFFSET: usize = 16;
const ASSET_MARK_TICKS_OFFSET: usize = 24;
const ASSET_STATIC_MARKET_OFFSET: usize = 904;
const ASSET_TICK_SIZE_OFFSET: usize = ASSET_STATIC_MARKET_OFFSET + 32;
const ASSET_ASSET_ID_LOWER_OFFSET: usize = ASSET_STATIC_MARKET_OFFSET + 40;
const ASSET_BASE_LOT_DECIMALS_OFFSET: usize = ASSET_STATIC_MARKET_OFFSET + 42;
const ASSET_ASSET_ID_UPPER_OFFSET: usize = ASSET_STATIC_MARKET_OFFSET + 44;
const ASSET_FUNDING_OFFSET: usize = 1_280;
const ASSET_CUMULATIVE_FUNDING_RATE_OFFSET: usize = ASSET_FUNDING_OFFSET + 32;
const ASSET_START_INTERVAL_TIMESTAMP_OFFSET: usize = ASSET_FUNDING_OFFSET + 40;
const ASSET_LAST_FUNDING_UPDATE_TIMESTAMP_OFFSET: usize = ASSET_FUNDING_OFFSET + 48;
const ASSET_FUNDING_INTERVAL_SECONDS_OFFSET: usize = ASSET_FUNDING_OFFSET + 56;
const ASSET_FUNDING_PERIOD_SECONDS_OFFSET: usize = ASSET_FUNDING_OFFSET + 64;
const ASSET_MAX_FUNDING_RATE_OFFSET: usize = ASSET_FUNDING_OFFSET + 72;
const ASSET_TOMBSTONE_OFFSET: usize = 1_450;
const ASSET_RISK_ACTION_PRICE_VALIDITY_RULES_OFFSET: usize = 352;
const ASSET_MARK_PRICE_LAST_VALIDATED_SLOT_OFFSET: usize = 816;
const RISK_ACTION_PRICE_VALIDITY_RULES_LEN: usize = 256;
const MARK_PRICE_LAST_VALIDATED_SLOT_COUNT: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PhoenixLayout {
    V1,
}

/// Protocol-owned, per-market Phoenix source configuration.
///
/// This is trusted configuration, not serialized instruction data and not a
/// caller-controlled account-selection mechanism. The pure adapter accepts it
/// by reference so tests and protocol-owned market configuration can select
/// any supported Rise market without hard-coding SOL. The instruction-layer
/// refresh path must load this configuration from protocol state (and reject
/// or ignore a caller-supplied replacement), then bind the supplied accounts
/// to the stored global/map/slot/symbol/market metadata.
///
/// Consistently changing all market-specific fields to another valid source is
/// a governance reconfiguration. Changing only the source accounts is account
/// substitution and must not redirect this configuration to another slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PhoenixSourceConfig {
    pub layout: PhoenixLayout,
    pub global_configuration: Pubkey,
    pub perp_asset_map: Pubkey,
    pub canonical_quote_mint: Pubkey,
    pub asset_slot: u16,
    pub symbol: [u8; 16],
    pub asset_id: u16,
    pub market_account: Pubkey,
    pub base_lot_decimals: u8,
    pub tick_size: u64,
    pub funding_interval_seconds: u64,
    pub funding_period_seconds: u64,
    pub max_abs_native_delta: u128,
    pub max_funding_age_seconds: u64,
    pub max_mark_age_slots: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PhoenixSourceContext {
    pub protocol_now: i64,
    pub protocol_slot: u64,
}

/// Source fields decoded from one fixed Phoenix asset-map entry.
///
/// `risk_action_price_validity_rules` is retained as the exact 256 raw bytes
/// exposed by the pinned Rise decoder. Rise does not define an on-chain rule
/// value invariant in this source pin, so the adapter does not reinterpret or
/// reject individual bytes. The four validation slots are retained verbatim;
/// no slot is selected or assigned semantics by this pure slice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ValidatedPhoenixSource {
    pub cumulative_funding_rate: i64,
    pub mark_ticks: i64,
    pub mark_slot: u64,
    pub risk_action_price_validity_rules: [u8; RISK_ACTION_PRICE_VALIDITY_RULES_LEN],
    pub mark_price_last_validated_slots: [u64; MARK_PRICE_LAST_VALIDATED_SLOT_COUNT],
    pub start_interval_timestamp: u64,
    pub last_funding_update_timestamp: u64,
    pub funding_interval_seconds: u64,
    pub funding_period_seconds: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PhoenixRateObservation {
    pub source_checkpoint: i64,
    pub source_native_delta: i128,
    pub rate_delta: RateDelta,
    pub mark_ticks: i64,
    pub mark_slot: u64,
    pub last_funding_update_timestamp: u64,
}

/// Validate the fixed-offset, source-backed Phoenix accounts for one market.
///
/// `config` is trusted protocol-owned market configuration; account bytes and
/// the clock context are the untrusted inputs. The instruction-layer refresh
/// path must load `config` from protocol state and source
/// `PhoenixSourceContext` from the Solana Clock sysvar rather than instruction
/// data.
pub fn validate_source(
    config: &PhoenixSourceConfig,
    context: &PhoenixSourceContext,
    global: AccountView<'_>,
    asset_map: AccountView<'_>,
) -> Result<ValidatedPhoenixSource, AdapterError> {
    if config.layout != PhoenixLayout::V1 {
        return Err(AdapterError::InvalidMapLayout);
    }
    if config.global_configuration != PHOENIX_GLOBAL_CONFIGURATION
        || config.perp_asset_map != PHOENIX_PERP_ASSET_MAP
        || global.key != config.global_configuration
        || asset_map.key != config.perp_asset_map
    {
        return Err(AdapterError::InvalidAccountKey);
    }
    if global.owner != PHOENIX_PROGRAM_ID || asset_map.owner != PHOENIX_PROGRAM_ID {
        return Err(AdapterError::InvalidAccountOwner);
    }
    if global.data.len() != GLOBAL_CONFIGURATION_ACCOUNT_LEN
        || asset_map.data.len() != PERP_ASSET_MAP_ACCOUNT_LEN
    {
        return Err(AdapterError::InvalidAccountLength);
    }
    require_discriminator(global.data, GLOBAL_CONFIGURATION_DISCRIMINATOR)?;
    require_discriminator(asset_map.data, PERP_ASSET_MAP_DISCRIMINATOR)?;

    if read_pubkey(global.data, GLOBAL_ACCOUNT_KEY_OFFSET)? != global.key {
        return Err(AdapterError::InvalidAccountKey);
    }
    if read_pubkey(global.data, GLOBAL_ASSET_MAP_OFFSET)? != asset_map.key {
        return Err(AdapterError::InvalidMapRelationship);
    }
    if read_pubkey(global.data, GLOBAL_CANONICAL_MINT_OFFSET)? != config.canonical_quote_mint {
        return Err(AdapterError::InvalidCanonicalQuoteMint);
    }

    let slots_used = read_u32(asset_map.data, MAP_SLOTS_USED_OFFSET)?;
    let num_assets = read_u16(asset_map.data, MAP_NUM_ASSETS_OFFSET)?;
    let capacity = read_u64(asset_map.data, MAP_CAPACITY_OFFSET)?;
    if slots_used > MAX_NUMBER_OF_PERP_ASSETS
        || u32::from(num_assets) > slots_used
        || capacity != u64::from(MAX_NUMBER_OF_PERP_ASSETS)
    {
        return Err(AdapterError::InvalidMapLayout);
    }
    if u32::from(config.asset_slot) >= slots_used {
        return Err(AdapterError::InvalidAssetSlot);
    }

    let entry_offset = PERP_ASSET_MAP_HEADER_LEN
        .checked_add(
            usize::from(config.asset_slot)
                .checked_mul(PERP_ASSET_ENTRY_STRIDE)
                .ok_or(AdapterError::ArithmeticOverflow)?,
        )
        .ok_or(AdapterError::ArithmeticOverflow)?;
    let value_offset = entry_offset
        .checked_add(ASSET_VALUE_OFFSET)
        .ok_or(AdapterError::ArithmeticOverflow)?;
    let symbol = read_array::<16>(asset_map.data, entry_offset)?;
    if symbol != config.symbol {
        return Err(AdapterError::InvalidSymbol);
    }
    if asset_map.data[value_offset + ASSET_TOMBSTONE_OFFSET] != 0 {
        return Err(AdapterError::InvalidAssetSlot);
    }

    let market_account = read_pubkey(asset_map.data, value_offset + ASSET_STATIC_MARKET_OFFSET)?;
    let tick_size = read_u64(asset_map.data, value_offset + ASSET_TICK_SIZE_OFFSET)?;
    let asset_id = u32::from(read_u16(
        asset_map.data,
        value_offset + ASSET_ASSET_ID_LOWER_OFFSET,
    )?) | (u32::from(read_u16(
        asset_map.data,
        value_offset + ASSET_ASSET_ID_UPPER_OFFSET,
    )?) << 16);
    let base_lot_decimals = read_i8(
        asset_map.data,
        value_offset + ASSET_BASE_LOT_DECIMALS_OFFSET,
    )?;
    if market_account != config.market_account {
        return Err(AdapterError::InvalidMarket);
    }
    if asset_id != u32::from(config.asset_id) {
        return Err(AdapterError::InvalidAssetId);
    }
    let configured_base_lot_decimals =
        i8::try_from(config.base_lot_decimals).map_err(|_| AdapterError::InvalidLotMetadata)?;
    if tick_size != config.tick_size || base_lot_decimals != configured_base_lot_decimals {
        return Err(AdapterError::InvalidLotMetadata);
    }

    if config.funding_interval_seconds == 0 || config.funding_period_seconds == 0 {
        return Err(AdapterError::InvalidInterval);
    }
    let cumulative_funding_rate = read_i64(
        asset_map.data,
        value_offset + ASSET_CUMULATIVE_FUNDING_RATE_OFFSET,
    )?;
    let start_interval_timestamp = read_u64(
        asset_map.data,
        value_offset + ASSET_START_INTERVAL_TIMESTAMP_OFFSET,
    )?;
    let last_funding_update_timestamp = read_u64(
        asset_map.data,
        value_offset + ASSET_LAST_FUNDING_UPDATE_TIMESTAMP_OFFSET,
    )?;
    let funding_interval_seconds = read_u64(
        asset_map.data,
        value_offset + ASSET_FUNDING_INTERVAL_SECONDS_OFFSET,
    )?;
    let funding_period_seconds = read_u64(
        asset_map.data,
        value_offset + ASSET_FUNDING_PERIOD_SECONDS_OFFSET,
    )?;
    let _max_funding_rate = read_i64(asset_map.data, value_offset + ASSET_MAX_FUNDING_RATE_OFFSET)?;
    if start_interval_timestamp > last_funding_update_timestamp {
        return Err(AdapterError::InvalidSourceTimestamp);
    }
    if funding_interval_seconds == 0
        || funding_period_seconds == 0
        || funding_interval_seconds != config.funding_interval_seconds
        || funding_period_seconds != config.funding_period_seconds
    {
        return Err(AdapterError::InvalidInterval);
    }

    if context.protocol_now < 0 {
        return Err(AdapterError::InvalidClock);
    }
    let protocol_now =
        u64::try_from(context.protocol_now).map_err(|_| AdapterError::InvalidClock)?;
    if last_funding_update_timestamp > protocol_now {
        return Err(AdapterError::FutureFundingTimestamp);
    }
    if protocol_now - last_funding_update_timestamp > config.max_funding_age_seconds {
        return Err(AdapterError::StaleFunding);
    }

    let risk_action_price_validity_rules = read_array::<RISK_ACTION_PRICE_VALIDITY_RULES_LEN>(
        asset_map.data,
        value_offset + ASSET_RISK_ACTION_PRICE_VALIDITY_RULES_OFFSET,
    )?;
    let mut mark_price_last_validated_slots = [0; MARK_PRICE_LAST_VALIDATED_SLOT_COUNT];
    for (index, slot) in mark_price_last_validated_slots.iter_mut().enumerate() {
        *slot = read_u64(
            asset_map.data,
            value_offset + ASSET_MARK_PRICE_LAST_VALIDATED_SLOT_OFFSET + index * 8,
        )?;
    }

    let mark_slot = read_u64(asset_map.data, value_offset + ASSET_MARK_SLOT_OFFSET)?;
    let mark_ticks_u64 = read_u64(asset_map.data, value_offset + ASSET_MARK_TICKS_OFFSET)?;
    if mark_slot > context.protocol_slot {
        return Err(AdapterError::FutureMarkSlot);
    }
    if context.protocol_slot - mark_slot > config.max_mark_age_slots {
        return Err(AdapterError::StaleMark);
    }
    let mark_ticks = i64::try_from(mark_ticks_u64).map_err(|_| AdapterError::InvalidMark)?;
    if mark_ticks <= 0 || tick_size == 0 {
        return Err(AdapterError::InvalidMark);
    }

    Ok(ValidatedPhoenixSource {
        cumulative_funding_rate,
        mark_ticks,
        mark_slot,
        risk_action_price_validity_rules,
        mark_price_last_validated_slots,
        start_interval_timestamp,
        last_funding_update_timestamp,
        funding_interval_seconds,
        funding_period_seconds,
    })
}

pub fn normalize_source_delta(
    native_delta: i128,
    base_lot_decimals: u8,
) -> Result<RateDelta, AdapterError> {
    // Rise stores cumulative funding in signed quote lots per base lot. Its
    // quote lots have six decimals, while base_lot_decimals is the number of
    // base lots per whole underlying unit. Preserve the venue's price-weighted
    // economics by converting only those lot units.
    let mut native = native_delta.unsigned_abs();
    let mut q64_scale = 1u128 << 64;
    let mut base_scale = 10u128
        .checked_pow(u32::from(base_lot_decimals))
        .ok_or(AdapterError::NormalizationOverflow)?;
    let mut denominator = 10u128.pow(u32::from(PHOENIX_QUOTE_LOT_DECIMALS));
    for numerator in [&mut native, &mut q64_scale, &mut base_scale] {
        let divisor = gcd(*numerator, denominator);
        *numerator /= divisor;
        denominator /= divisor;
    }
    let magnitude = native
        .checked_mul(q64_scale)
        .and_then(|value| value.checked_mul(base_scale))
        .and_then(|value| value.checked_div(denominator))
        .ok_or(AdapterError::NormalizationOverflow)?;
    let magnitude = i128::try_from(magnitude).map_err(|_| AdapterError::NormalizationOverflow)?;
    let raw_bits = if native_delta < 0 {
        magnitude
            .checked_neg()
            .ok_or(AdapterError::NormalizationOverflow)?
    } else {
        magnitude
    };
    Ok(RateDelta::from_i64f64(fixed::types::I64F64::from_bits(
        raw_bits,
    )))
}

fn gcd(mut left: u128, mut right: u128) -> u128 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left
}

pub fn observe(
    config: &PhoenixSourceConfig,
    context: &PhoenixSourceContext,
    global: AccountView<'_>,
    asset_map: AccountView<'_>,
    previous_native_checkpoint: i64,
) -> Result<PhoenixRateObservation, AdapterError> {
    let source = validate_source(config, context, global, asset_map)?;
    let source_native_delta = i128::from(source.cumulative_funding_rate)
        .checked_sub(i128::from(previous_native_checkpoint))
        .ok_or(AdapterError::ArithmeticOverflow)?;
    if !native_delta_within_bound(source_native_delta, config.max_abs_native_delta) {
        return Err(AdapterError::SourceDeltaTooLarge);
    }
    let rate_delta = normalize_source_delta(source_native_delta, config.base_lot_decimals)?;
    Ok(PhoenixRateObservation {
        source_checkpoint: source.cumulative_funding_rate,
        source_native_delta,
        rate_delta,
        mark_ticks: source.mark_ticks,
        mark_slot: source.mark_slot,
        last_funding_update_timestamp: source.last_funding_update_timestamp,
    })
}

pub fn native_delta_within_bound(native_delta: i128, max_abs_native_delta: u128) -> bool {
    native_delta.unsigned_abs() <= max_abs_native_delta
}

fn require_discriminator(data: &[u8], expected: [u8; 8]) -> Result<(), AdapterError> {
    if data.len() < expected.len() || data[..expected.len()] != expected {
        return Err(AdapterError::InvalidDiscriminator);
    }
    Ok(())
}

fn read_array<const N: usize>(data: &[u8], offset: usize) -> Result<[u8; N], AdapterError> {
    let end = offset
        .checked_add(N)
        .ok_or(AdapterError::ArithmeticOverflow)?;
    let bytes = data
        .get(offset..end)
        .ok_or(AdapterError::InvalidAccountLength)?;
    let mut value = [0u8; N];
    value.copy_from_slice(bytes);
    Ok(value)
}

fn read_pubkey(data: &[u8], offset: usize) -> Result<Pubkey, AdapterError> {
    Ok(Pubkey::new_from_array(read_array(data, offset)?))
}

fn read_u16(data: &[u8], offset: usize) -> Result<u16, AdapterError> {
    Ok(u16::from_le_bytes(read_array(data, offset)?))
}

fn read_u32(data: &[u8], offset: usize) -> Result<u32, AdapterError> {
    Ok(u32::from_le_bytes(read_array(data, offset)?))
}

fn read_u64(data: &[u8], offset: usize) -> Result<u64, AdapterError> {
    Ok(u64::from_le_bytes(read_array(data, offset)?))
}

fn read_i8(data: &[u8], offset: usize) -> Result<i8, AdapterError> {
    Ok(i8::from_le_bytes(read_array(data, offset)?))
}

fn read_i64(data: &[u8], offset: usize) -> Result<i64, AdapterError> {
    Ok(i64::from_le_bytes(read_array(data, offset)?))
}
