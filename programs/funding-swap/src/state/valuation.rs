// CHECKED
use anchor_lang::prelude::*;
use bytemuck::{Pod, Zeroable};

use super::{FixedPoint, StateError, StateResult};

/// Authentication policy for one Pyth USD price feed.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Pod, Zeroable)]
#[cfg_attr(feature = "idl-build", derive(AnchorSerialize, AnchorDeserialize))]
pub struct OracleConfig {
    pub feed_id: [u8; 32],
    pub max_age_seconds: u64,
    pub _pad: [u8; 8],
    pub max_confidence_ratio: FixedPoint,
    pub max_ema_deviation: FixedPoint,
}

impl OracleConfig {
    pub fn validate(&self) -> StateResult<()> {
        if self.feed_id == [0; 32]
            || self.max_age_seconds == 0
            || self.max_confidence_ratio.to_raw_bits() < 0
            || self.max_ema_deviation.to_raw_bits() < 0
        {
            return Err(StateError::InvalidOraclePolicy);
        }
        Ok(())
    }
}

/// The authenticated, transient observation used by health and settlement.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct OracleObservation {
    /// Central oracle price after applying the decimal exponent, in Q64.64.
    pub normalized_price: FixedPoint,
    /// Lower and upper confidence endpoints after decimal scaling, in Q64.64.
    pub lower: FixedPoint,
    pub upper: FixedPoint,
    /// EMA after decimal scaling, in the same units as normalized_price.
    pub ema: FixedPoint,
    pub publish_time: i64,
    /// Confidence width after decimal scaling, in Q64.64.
    pub confidence: FixedPoint,
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

        if age > max_age_seconds {
            return Err(StateError::StaleObservation);
        }

        if self.normalized_price.to_raw_bits() <= 0
            || self.lower.to_raw_bits() <= 0
            || self.upper.to_raw_bits() < self.normalized_price.to_raw_bits()
            || self.lower.to_raw_bits() > self.normalized_price.to_raw_bits()
            || self.confidence.to_raw_bits() < 0
            || self.ema.to_raw_bits() <= 0
        {
            return Err(StateError::InvalidOracleObservation);
        }

        let scale = 1u128 << 64;

        let confidence = u128::try_from(self.confidence.to_raw_bits())
            .map_err(|_| StateError::ArithmeticOverflow)?;

        let price = u128::try_from(self.normalized_price.to_raw_bits())
            .map_err(|_| StateError::ArithmeticOverflow)?;

        let max_confidence = u128::try_from(max_confidence_ratio.to_raw_bits())
            .map_err(|_| StateError::ArithmeticOverflow)?;

        let confidence_bound = price
            .checked_mul(max_confidence)
            .ok_or(StateError::ArithmeticOverflow)?;

        let confidence_value = confidence
            .checked_mul(scale)
            .ok_or(StateError::ArithmeticOverflow)?;

        if confidence_value > confidence_bound {
            return Err(StateError::OracleConfidenceTooWide);
        }

        let delta = self
            .normalized_price
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
            return Err(StateError::OracleEmaDeviationExceeded);
        }

        Ok(())
    }

    /// Reads and authenticates one Pyth Receiver price-update account.  The
    /// account owner, full guardian verification, feed identity, freshness,
    /// confidence, and spot-vs-EMA bound are all checked before any value is
    /// returned to a caller.
    pub fn from_pyth_account(
        account: &AccountInfo<'_>,
        config: &OracleConfig,
        now: i64,
    ) -> StateResult<Self> {
        use anchor_lang::AccountDeserialize;
        use pyth_solana_receiver_sdk::{price_update::PriceUpdateV2, ID as PYTH_RECEIVER_ID};

        if account.owner != &PYTH_RECEIVER_ID {
            return Err(StateError::InvalidAccountOwner);
        }

        let data = account
            .try_borrow_data()
            .map_err(|_| StateError::AccountDataBorrowFailed)?;

        let mut bytes: &[u8] = &data;

        let update = PriceUpdateV2::try_deserialize(&mut bytes)
            .map_err(|_| StateError::InvalidOracleAccountData)?;

        if update.verification_level
            != pyth_solana_receiver_sdk::price_update::VerificationLevel::Full
        {
            return Err(StateError::OracleNotFullyVerified);
        }

        let message = update.price_message;

        if config.feed_id != message.feed_id {
            return Err(StateError::InvalidOracleFeed);
        }

        let normalized_price = scale_pyth_value(i128::from(message.price), message.exponent)?;
        let confidence = scale_pyth_value(i128::from(message.conf), message.exponent)?;
        let ema = scale_pyth_value(i128::from(message.ema_price), message.exponent)?;

        let lower = FixedPoint::from_raw_bits(
            normalized_price
                .to_raw_bits()
                .checked_sub(confidence.to_raw_bits())
                .ok_or(StateError::ArithmeticOverflow)?,
        );

        let upper = FixedPoint::from_raw_bits(
            normalized_price
                .to_raw_bits()
                .checked_add(confidence.to_raw_bits())
                .ok_or(StateError::ArithmeticOverflow)?,
        );

        let observation = Self {
            normalized_price,
            lower,
            upper,
            ema,
            publish_time: message.publish_time,
            confidence,
        };

        observation.validate(
            now,
            config.max_age_seconds,
            config.max_confidence_ratio,
            config.max_ema_deviation,
        )?;

        Ok(observation)
    }
}

fn scale_pyth_value(mantissa: i128, exponent: i32) -> StateResult<FixedPoint> {
    if mantissa < 0 {
        return Err(StateError::InvalidOraclePrice);
    }

    let value =
        fixed::types::I64F64::checked_from_num(mantissa).ok_or(StateError::ArithmeticOverflow)?;
    let raw = u128::try_from(value.to_bits()).map_err(|_| StateError::ArithmeticOverflow)?;

    if raw == 0 {
        return Ok(FixedPoint::default());
    }

    let factor = funding_swap_math::pow10(exponent.unsigned_abs());
    let scaled = if exponent >= 0 {
        raw.checked_mul(factor.ok_or(StateError::ArithmeticOverflow)?)
            .ok_or(StateError::ArithmeticOverflow)?
    } else {
        // 10^39 exceeds every positive Q64.64 raw value, so larger negative
        // exponents underflow to zero, just as repeated division does.
        factor.map_or(0, |factor| raw / factor)
    };

    Ok(FixedPoint::from_raw_bits(
        i128::try_from(scaled).map_err(|_| StateError::ArithmeticOverflow)?,
    ))
}

/// Sign-directed valuation factors.
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
            return Err(StateError::InvalidValuationFactors);
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
