// CHECKED
use anchor_lang::prelude::*;
use bytemuck::{Pod, Zeroable};

use super::vault::AssetState;
use super::{FixedPoint, StateError, StateResult};

/// Price-source policy stored in an [`AssetState`].
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Pod, Zeroable)]
#[cfg_attr(feature = "idl-build", derive(AnchorSerialize, AnchorDeserialize))]
pub struct PriceModel(pub u8);

pub const PRICE_MODEL_FIXED: u8 = 0;
pub const PRICE_MODEL_PEGGED: u8 = 1;
pub const PRICE_MODEL_FLOATING: u8 = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PriceEndpoint {
    Lower,
    Upper,
}

impl PriceEndpoint {
    pub fn for_signed_value(value: i128) -> Self {
        if value < 0 {
            Self::Upper
        } else {
            Self::Lower
        }
    }
}

impl PriceModel {
    pub const FIXED: Self = Self(PRICE_MODEL_FIXED);
    pub const PEGGED: Self = Self(PRICE_MODEL_PEGGED);
    pub const FLOATING: Self = Self(PRICE_MODEL_FLOATING);

    pub fn validate(self) -> StateResult<()> {
        match self.0 {
            PRICE_MODEL_FIXED | PRICE_MODEL_PEGGED | PRICE_MODEL_FLOATING => Ok(()),
            _ => Err(StateError::InvalidTag),
        }
    }
}

/// Exact positive rational used when a matured market converts settlement
/// units into collateral units.  Keeping the numerator and denominator (and
/// reducing them before storage) avoids quantising a cross-mint oracle ratio.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Pod, Zeroable)]
#[cfg_attr(feature = "idl-build", derive(AnchorSerialize, AnchorDeserialize))]
pub struct SettlementRatio {
    pub numerator: u128,
    pub denominator: u128,
}

impl SettlementRatio {
    pub fn new(numerator: u128, denominator: u128) -> StateResult<Self> {
        if numerator == 0 || denominator == 0 {
            return Err(StateError::InvalidConfiguration);
        }

        let divisor = gcd(numerator, denominator);

        Ok(Self {
            numerator: numerator / divisor,
            denominator: denominator / divisor,
        })
    }

    /// Applies the exact ratio once. Positive claims are floored; negative
    /// obligations are ceiled so the protocol never understates either side.
    pub fn apply(self, settlement_balance: i128) -> StateResult<i128> {
        let magnitude = settlement_balance.unsigned_abs();

        let product = magnitude
            .checked_mul(self.numerator)
            .ok_or(StateError::ArithmeticOverflow)?;

        let quotient = product / self.denominator;

        let remainder = product % self.denominator;

        let rounded = if settlement_balance < 0 && remainder != 0 {
            quotient
                .checked_add(1)
                .ok_or(StateError::ArithmeticOverflow)?
        } else {
            quotient
        };

        if settlement_balance < 0 {
            let value = i128::try_from(rounded).map_err(|_| StateError::ArithmeticOverflow)?;
            value.checked_neg().ok_or(StateError::ArithmeticOverflow)
        } else {
            i128::try_from(rounded).map_err(|_| StateError::ArithmeticOverflow)
        }
    }

    /// Inverts [`Self::apply`]. Collateral atoms become settlement atoms.
    /// Positive amounts floor and negative amounts move away from zero.
    pub fn apply_inverse(self, collateral_balance: i128) -> StateResult<i128> {
        let magnitude = collateral_balance.unsigned_abs();
        let product = magnitude
            .checked_mul(self.denominator)
            .ok_or(StateError::ArithmeticOverflow)?;
        let quotient = product / self.numerator;
        let remainder = product % self.numerator;
        let rounded = if collateral_balance < 0 && remainder != 0 {
            quotient
                .checked_add(1)
                .ok_or(StateError::ArithmeticOverflow)?
        } else {
            quotient
        };
        if collateral_balance < 0 {
            let value = i128::try_from(rounded).map_err(|_| StateError::ArithmeticOverflow)?;
            value.checked_neg().ok_or(StateError::ArithmeticOverflow)
        } else {
            i128::try_from(rounded).map_err(|_| StateError::ArithmeticOverflow)
        }
    }

    /// Constructs collateral-atoms per settlement-atom from positive oracle
    /// mantissas and decimal exponents.  No intermediate floating-point ratio
    /// is formed.
    pub fn from_price_mantissas(
        settlement_price: u64,
        settlement_exponent: i32,
        collateral_price: u64,
        collateral_exponent: i32,
        collateral_decimals: u8,
        settlement_decimals: u8,
    ) -> StateResult<Self> {
        if settlement_price == 0 || collateral_price == 0 {
            return Err(StateError::InvalidConfiguration);
        }

        let mut numerator = u128::from(settlement_price);
        let mut denominator = u128::from(collateral_price);

        // Price values are mantissa * 10^exponent.  Include token atom
        // decimals in the same rational before reducing it.
        scale_signed_power(&mut numerator, &mut denominator, settlement_exponent)?;

        scale_signed_power(
            &mut numerator,
            &mut denominator,
            i32::from(collateral_decimals),
        )?;

        scale_signed_power(&mut numerator, &mut denominator, -collateral_exponent)?;

        scale_signed_power(
            &mut numerator,
            &mut denominator,
            -i32::from(settlement_decimals),
        )?;

        Self::new(numerator, denominator)
    }

    pub fn from_fixed_prices(
        settlement_price: FixedPoint,
        collateral_price: FixedPoint,
        collateral_decimals: u8,
        settlement_decimals: u8,
    ) -> StateResult<Self> {
        let settlement_raw = settlement_price.to_raw_bits();
        let collateral_raw = collateral_price.to_raw_bits();

        if settlement_raw <= 0 || collateral_raw <= 0 {
            return Err(StateError::InvalidConfiguration);
        }

        let mut numerator =
            u128::try_from(settlement_raw).map_err(|_| StateError::InvalidConfiguration)?;

        let mut denominator =
            u128::try_from(collateral_raw).map_err(|_| StateError::InvalidConfiguration)?;

        // The exponent from fixed prices are always 0, the price itself is 1.
        scale_signed_power(
            &mut numerator,
            &mut denominator,
            i32::from(collateral_decimals),
        )?;
        scale_signed_power(
            &mut numerator,
            &mut denominator,
            -i32::from(settlement_decimals),
        )?;

        Self::new(numerator, denominator)
    }

    /// Builds the ratio from transient observations.  When both observations
    /// came directly from Pyth, preserve their mantissa/exponent fields and
    /// avoid an intermediate Q64.64 division.  Fixed or on-peg observations
    /// intentionally fall back to their exact configured Q64.64 anchors.
    pub fn from_observations(
        settlement_price: FixedPoint,
        settlement_observation: OracleObservation,
        collateral_price: FixedPoint,
        collateral_observation: OracleObservation,
        collateral_decimals: u8,
        settlement_decimals: u8,
    ) -> StateResult<Self> {
        if settlement_observation.price_mantissa > 0
            && collateral_observation.price_mantissa > 0
            && settlement_price == settlement_observation.price
            && collateral_price == collateral_observation.price
        {
            Self::from_price_mantissas(
                u64::try_from(settlement_observation.price_mantissa)
                    .map_err(|_| StateError::ArithmeticOverflow)?,
                settlement_observation.exponent,
                u64::try_from(collateral_observation.price_mantissa)
                    .map_err(|_| StateError::ArithmeticOverflow)?,
                collateral_observation.exponent,
                collateral_decimals,
                settlement_decimals,
            )
        } else {
            Self::from_fixed_prices(
                settlement_price,
                collateral_price,
                collateral_decimals,
                settlement_decimals,
            )
        }
    }
}

fn gcd(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        let remainder = a % b;
        a = b;
        b = remainder;
    }
    a
}

fn scale_signed_power(
    numerator: &mut u128,
    denominator: &mut u128,
    exponent: i32,
) -> StateResult<()> {
    if exponent >= 0 {
        let factor = ten_pow(u32::try_from(exponent).map_err(|_| StateError::ArithmeticOverflow)?)?;

        *numerator = numerator
            .checked_mul(factor)
            .ok_or(StateError::ArithmeticOverflow)?;
    } else {
        let factor = ten_pow(exponent.unsigned_abs())?;

        *denominator = denominator
            .checked_mul(factor)
            .ok_or(StateError::ArithmeticOverflow)?;
    }

    let divisor = gcd(*numerator, *denominator);

    *numerator /= divisor;

    *denominator /= divisor;

    Ok(())
}

fn ten_pow(exponent: u32) -> StateResult<u128> {
    if exponent > 38 {
        return Err(StateError::ArithmeticOverflow);
    }

    let mut value = 1u128;

    for _ in 0..exponent {
        value = value
            .checked_mul(10)
            .ok_or(StateError::ArithmeticOverflow)?;
    }
    Ok(value)
}

/// The authenticated, transient observation used by health and settlement.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct OracleObservation {
    pub price: FixedPoint,
    pub lower: FixedPoint,
    pub upper: FixedPoint,
    pub ema: FixedPoint,
    pub publish_time: i64,
    pub confidence: FixedPoint,
    /// Original Pyth price fields, retained only in this transient value so
    /// finalization can form an exact ratio without first quantizing to Q64.64.
    pub price_mantissa: i128,
    pub exponent: i32,
}

impl OracleObservation {
    pub fn validate(
        &self,
        now: i64,
        max_age_seconds: u64,
        max_confidence_ratio: FixedPoint,
        max_ema_deviation: FixedPoint,
    ) -> StateResult<()> {
        if now < 0 || self.publish_time < 0 || self.publish_time > now {
            return Err(StateError::InvalidClock);
        }

        let age = u64::try_from(
            now.checked_sub(self.publish_time)
                .ok_or(StateError::InvalidClock)?,
        )
        .map_err(|_| StateError::InvalidClock)?;

        if age > max_age_seconds
            || self.price.to_raw_bits() <= 0
            || self.lower.to_raw_bits() <= 0
            || self.upper.to_raw_bits() < self.price.to_raw_bits()
            || self.lower.to_raw_bits() > self.price.to_raw_bits()
            || self.confidence.to_raw_bits() < 0
            || self.ema.to_raw_bits() <= 0
        {
            return Err(StateError::InvalidConfiguration);
        }

        let scale = 1u128 << 64;
        let confidence = u128::try_from(self.confidence.to_raw_bits())
            .map_err(|_| StateError::ArithmeticOverflow)?;
        let price =
            u128::try_from(self.price.to_raw_bits()).map_err(|_| StateError::ArithmeticOverflow)?;
        let max_confidence = u128::try_from(max_confidence_ratio.to_raw_bits())
            .map_err(|_| StateError::ArithmeticOverflow)?;
        let confidence_bound = price
            .checked_mul(max_confidence)
            .ok_or(StateError::ArithmeticOverflow)?;
        let confidence_value = confidence
            .checked_mul(scale)
            .ok_or(StateError::ArithmeticOverflow)?;
        if confidence_value > confidence_bound {
            return Err(StateError::Conflict);
        }

        let delta = self
            .price
            .to_raw_bits()
            .checked_sub(self.ema.to_raw_bits())
            .ok_or(StateError::ArithmeticOverflow)?
            .unsigned_abs();
        let ema =
            u128::try_from(self.ema.to_raw_bits()).map_err(|_| StateError::ArithmeticOverflow)?;
        let max_ema = u128::try_from(max_ema_deviation.to_raw_bits())
            .map_err(|_| StateError::ArithmeticOverflow)?;
        let ema_bound = ema
            .checked_mul(max_ema)
            .ok_or(StateError::ArithmeticOverflow)?;
        let ema_value = delta
            .checked_mul(scale)
            .ok_or(StateError::ArithmeticOverflow)?;
        if ema_value > ema_bound {
            return Err(StateError::Conflict);
        }
        Ok(())
    }

    pub fn conservative_price(&self, endpoint: PriceEndpoint) -> FixedPoint {
        match endpoint {
            PriceEndpoint::Lower => self.lower,
            PriceEndpoint::Upper => self.upper,
        }
    }

    /// Applies the configured peg policy before selecting the conservative
    /// endpoint.  Pegged assets use the anchor while the authenticated central
    /// spot remains inside the band; otherwise the sign-directed endpoint is
    /// retained for health calculations.
    pub fn conservative_price_for(
        &self,
        asset: &AssetState,
        endpoint: PriceEndpoint,
    ) -> StateResult<FixedPoint> {
        if asset.price_model.0 == PRICE_MODEL_FIXED {
            return Ok(asset.reference_price);
        }
        if asset.central_price_is_on_peg(self.price)? {
            Ok(asset.reference_price)
        } else {
            Ok(self.conservative_price(endpoint))
        }
    }

    /// Reads and authenticates one Pyth Receiver price-update account.  The
    /// account owner, full guardian verification, feed identity, freshness,
    /// confidence, and spot-vs-EMA bound are all checked before any value is
    /// returned to a caller.
    pub fn from_pyth_account(
        account: &AccountInfo<'_>,
        asset: &AssetState,
        now: i64,
    ) -> StateResult<Self> {
        use anchor_lang::AccountDeserialize;
        use pyth_solana_receiver_sdk::{price_update::PriceUpdateV2, ID as PYTH_RECEIVER_ID};

        if account.owner != &PYTH_RECEIVER_ID {
            return Err(StateError::InvalidReference);
        }

        let data = account
            .try_borrow_data()
            .map_err(|_| StateError::InvalidReference)?;

        let mut bytes: &[u8] = &data;

        let update =
            PriceUpdateV2::try_deserialize(&mut bytes).map_err(|_| StateError::InvalidReference)?;

        if update.verification_level
            != pyth_solana_receiver_sdk::price_update::VerificationLevel::Full
        {
            return Err(StateError::Conflict);
        }

        let message = update.price_message;

        asset.validate_feed(message.feed_id)?;

        let price = scale_pyth_value(message.price as i128, message.exponent)?;
        let confidence = scale_pyth_value(message.conf as i128, message.exponent)?;
        let ema = scale_pyth_value(message.ema_price as i128, message.exponent)?;

        let lower = FixedPoint::from_raw_bits(
            price
                .to_raw_bits()
                .checked_sub(confidence.to_raw_bits())
                .ok_or(StateError::ArithmeticOverflow)?,
        );

        let upper = FixedPoint::from_raw_bits(
            price
                .to_raw_bits()
                .checked_add(confidence.to_raw_bits())
                .ok_or(StateError::ArithmeticOverflow)?,
        );

        let observation = Self {
            price,
            lower,
            upper,
            ema,
            publish_time: message.publish_time,
            confidence,
            price_mantissa: i128::from(message.price), // Negative prices are rejcted with the scale_pyth_value call for message.price
            exponent: message.exponent,
        };

        observation.validate(
            now,
            asset.max_age_seconds,
            asset.max_confidence_ratio,
            asset.max_ema_deviation,
        )?;

        Ok(observation)
    }

    pub fn fixed(asset: &AssetState, now: i64) -> StateResult<Self> {
        let one = asset.reference_price;

        Ok(Self {
            price: one,
            lower: one,
            upper: one,
            ema: one,
            publish_time: now,
            confidence: FixedPoint::from_raw_bits(0),
            price_mantissa: 0,
            exponent: 0,
        })
    }

    /// For a pegged asset, use the fixed anchor only while the central spot is
    /// inside the configured band. Outside the band, retain oracle endpoints
    /// for conservative health valuation.
    pub fn effective_price(asset: &AssetState, observation: Self) -> FixedPoint {
        if asset.price_model.0 == PRICE_MODEL_FIXED
            || asset
                .central_price_is_on_peg(observation.price)
                .unwrap_or(false)
        {
            asset.reference_price
        } else {
            observation.price
        }
    }
}

fn scale_pyth_value(mantissa: i128, exponent: i32) -> StateResult<FixedPoint> {
    if mantissa < 0 {
        return Err(StateError::InvalidConfiguration);
    }

    let mut value =
        fixed::types::I64F64::checked_from_num(mantissa).ok_or(StateError::ArithmeticOverflow)?;

    if exponent >= 0 {
        for _ in 0..u32::try_from(exponent).map_err(|_| StateError::ArithmeticOverflow)? {
            value = value
                .checked_mul(fixed::types::I64F64::from_num(10))
                .ok_or(StateError::ArithmeticOverflow)?;
        }
    } else {
        for _ in 0..exponent.unsigned_abs() {
            value = value
                .checked_div(fixed::types::I64F64::from_num(10))
                .ok_or(StateError::ArithmeticOverflow)?;
        }
    }

    Ok(FixedPoint::from_i64f64(value))
}

/// Governance-controlled sign-directed valuation factors.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Pod, Zeroable)]
#[cfg_attr(feature = "idl-build", derive(AnchorSerialize, AnchorDeserialize))]
pub struct ValuationFactors {
    pub discount_factor: FixedPoint,
    pub liability_factor: FixedPoint,
}

impl ValuationFactors {
    pub fn validate(&self) -> StateResult<()> {
        let one = 1i128 << 64;

        if self.discount_factor.to_raw_bits() <= 0
            || self.discount_factor.to_raw_bits() > one
            || self.liability_factor.to_raw_bits() < one
        {
            return Err(StateError::InvalidConfiguration);
        }
        Ok(())
    }

    pub fn apply(&self, value: i128) -> StateResult<i128> {
        let factor = if value >= 0 {
            self.discount_factor
        } else {
            self.liability_factor
        };

        factor
            .checked_mul_int(value)
            .map_err(|_| StateError::ArithmeticOverflow)?
            .floor_atoms()
            .map_err(|_| StateError::ArithmeticOverflow)
    }
}
