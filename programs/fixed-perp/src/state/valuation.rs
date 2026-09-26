use anchor_lang::prelude::*;
use bytemuck::{Pod, Zeroable};

use super::{FixedPoint, StateError, StateResult};

/// Canonical Q64.64 price plus an absolute confidence bound and freshness.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Pod, Zeroable)]
#[cfg_attr(feature = "idl-build", derive(AnchorSerialize, AnchorDeserialize))]
pub struct PriceObservation {
    pub canonical_price: FixedPoint,
    pub confidence: FixedPoint,
    pub last_update_timestamp: i64,
    pub max_age_seconds: u64,
    pub _pad_0: [u8; 8],
}

impl PriceObservation {
    pub fn validate(&self, now: i64) -> StateResult<()> {
        if now < 0 || self.last_update_timestamp < 0 || self.last_update_timestamp > now {
            return Err(StateError::InvalidClock);
        }
        if self.canonical_price.to_raw_bits() <= 0
            || self.confidence.to_raw_bits() < 0
            || self.confidence.to_raw_bits() >= self.canonical_price.to_raw_bits()
            || self.max_age_seconds == 0
            || now
                .checked_sub(self.last_update_timestamp)
                .ok_or(StateError::InvalidClock)?
                > i64::try_from(self.max_age_seconds).map_err(|_| StateError::ArithmeticOverflow)?
        {
            return Err(if self.max_age_seconds == 0 {
                StateError::InvalidConfiguration
            } else {
                StateError::StaleObservation
            });
        }
        Ok(())
    }

    /// Selects the confidence endpoint in the conservative direction.
    pub fn conservative_price(&self, signed_contribution: i128) -> StateResult<FixedPoint> {
        let price = self.canonical_price.to_raw_bits();
        let confidence = self.confidence.to_raw_bits();
        let adjusted = if signed_contribution >= 0 {
            price.checked_sub(confidence)
        } else {
            price.checked_add(confidence)
        }
        .ok_or(StateError::ArithmeticOverflow)?;
        Ok(FixedPoint::from_raw_bits(adjusted))
    }
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
