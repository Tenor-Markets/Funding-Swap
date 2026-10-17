use core::convert::TryFrom;

use fixed::types::I64F64;

use crate::{error::MathError, fixed_point::FixedPoint};

pub const MAX_TICK_STEP: u8 = 15;
pub const MAX_DECIMALS: u8 = 18;
pub const SECONDS_PER_YEAR: i64 = 365 * 24 * 60 * 60;

// Nearest Q64.64 representation of 1.00005 tick base.
const TICK_BASE: I64F64 = I64F64::from_bits(18_447_666_410_913_237_094);

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rate(FixedPoint);

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RateDelta(FixedPoint);

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RateIndex(FixedPoint);

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FeeRate(FixedPoint);

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FeeIndex(FixedPoint);

macro_rules! signed_wrapper {
    ($name:ident) => {
        impl $name {
            #[inline]
            pub const fn from_i64f64(value: I64F64) -> Self {
                Self(FixedPoint::from_i64f64(value))
            }

            #[inline]
            pub const fn to_i64f64(self) -> I64F64 {
                self.0.to_i64f64()
            }
        }
    };
}

macro_rules! fee_wrapper {
    ($name:ident) => {
        impl $name {
            #[inline]
            pub fn try_from_i64f64(value: I64F64) -> Result<Self, MathError> {
                if value.to_bits() < 0 {
                    return Err(MathError::NegativeFee);
                }

                Ok(Self(FixedPoint::from_i64f64(value)))
            }

            #[inline]
            pub const fn to_i64f64(self) -> I64F64 {
                self.0.to_i64f64()
            }
        }
    };
}

signed_wrapper!(Rate);
signed_wrapper!(RateDelta);
signed_wrapper!(RateIndex);
fee_wrapper!(FeeRate);
fee_wrapper!(FeeIndex);

impl Rate {
    /// Converts a signed tick to `sign(tick) * (1.00005^(abs(tick) * tick_step) - 1)`.
    pub fn from_tick(tick: i16, tick_step: u8) -> Result<Self, MathError> {
        if tick_step == 0 || tick_step > MAX_TICK_STEP {
            return Err(MathError::InvalidTickStep);
        }
        if tick == 0 {
            return Ok(Self::default());
        }

        // The validated bounds cap the exponent at 491_520.
        let mut exponent = u32::from(tick.unsigned_abs()) * u32::from(tick_step);
        let mut multiplier = I64F64::ONE;
        let mut power = TICK_BASE;
        while exponent != 0 {
            if exponent & 1 != 0 {
                multiplier = multiplier.checked_mul(power).ok_or(MathError::Overflow)?;
            }
            exponent >>= 1;
            if exponent != 0 {
                power = power.checked_mul(power).ok_or(MathError::Overflow)?;
            }
        }

        let magnitude = multiplier
            .checked_sub(I64F64::ONE)
            .ok_or(MathError::Overflow)?;
        let rate = if tick < 0 {
            magnitude.checked_neg().ok_or(MathError::Overflow)?
        } else {
            magnitude
        };
        Ok(Self::from_i64f64(rate))
    }

    /// Returns the underlying Q64.64 value for storage or further checked math.
    pub const fn to_fixed_point(self) -> FixedPoint {
        self.0
    }

    /// Calculates the fixed-leg cash change using Q64.64 math and signed ceiling.
    pub fn fixed_payment_atoms(
        self,
        signed_size: i64,
        time_to_maturity: i64,
        seconds_per_year: i64,
    ) -> Result<i128, MathError> {
        if time_to_maturity < 0 || seconds_per_year <= 0 {
            return Err(MathError::InvalidDuration);
        }
        if signed_size == 0 || time_to_maturity == 0 || self.0.to_raw_bits() == 0 {
            return Ok(0);
        }

        let duration = I64F64::checked_from_num(time_to_maturity).ok_or(MathError::Overflow)?;
        let year = I64F64::checked_from_num(seconds_per_year).ok_or(MathError::Overflow)?;
        let time_fraction = duration.checked_div(year).ok_or(MathError::Overflow)?;
        let annualized_rate = self
            .to_i64f64()
            .checked_mul(time_fraction)
            .ok_or(MathError::Overflow)?;
        let size = I64F64::checked_from_num(signed_size).ok_or(MathError::Overflow)?;
        let unrounded = size
            .checked_mul(annualized_rate)
            .ok_or(MathError::Overflow)?;
        let payment = unrounded.checked_ceil().ok_or(MathError::Overflow)?;
        let atoms = payment.to_bits() >> I64F64::FRAC_NBITS;

        atoms.checked_neg().ok_or(MathError::Overflow)
    }

    /// Converts an annualized, scaled price-weighted quote into settlement
    /// atoms. The underlying size is multiplied before decimal and tick-scale
    /// normalization, and one signed floor is applied to the full rational.
    pub fn fixed_payment_atoms_scaled(
        self,
        signed_underlying_atoms: i128,
        time_to_maturity: i64,
        seconds_per_year: i64,
        underlying_decimals: u8,
        settlement_decimals: u8,
        tick_scale: u8,
    ) -> Result<i128, MathError> {
        if time_to_maturity < 0 || seconds_per_year <= 0 {
            return Err(MathError::InvalidDuration);
        }
        let payment = scaled_payment(ScaledPaymentParams {
            raw_value_q64_bits: self.0.to_raw_bits(),
            signed_underlying_atoms,
            time_to_maturity_seconds: Some(time_to_maturity),
            seconds_per_year,
            underlying_decimals,
            settlement_decimals,
            tick_scale,
            rounding: Rounding::Floor,
        })?;
        payment.checked_neg().ok_or(MathError::Overflow)
    }
}

impl RateDelta {
    /// Multiplies by a position magnitude and floors the result to whole atoms.
    #[inline]
    pub fn floating_payment_atoms(self, position_magnitude: u64) -> Result<i128, MathError> {
        let position = I64F64::checked_from_num(position_magnitude).ok_or(MathError::Overflow)?;
        let product = position
            .checked_mul(self.to_i64f64())
            .ok_or(MathError::Overflow)?;
        let floored = product.checked_floor().ok_or(MathError::Overflow)?;

        Ok(floored.to_bits() >> I64F64::FRAC_NBITS)
    }

    /// Converts a cumulative, already time-weighted index delta into
    /// settlement atoms using the market's implicit decimal scale.
    pub fn floating_payment_atoms_scaled(
        self,
        signed_underlying_atoms: i128,
        underlying_decimals: u8,
        settlement_decimals: u8,
        tick_scale: u8,
    ) -> Result<i128, MathError> {
        scaled_payment(ScaledPaymentParams {
            raw_value_q64_bits: self.0.to_raw_bits(),
            signed_underlying_atoms,
            time_to_maturity_seconds: None,
            seconds_per_year: 1,
            underlying_decimals,
            settlement_decimals,
            tick_scale,
            rounding: Rounding::Floor,
        })
    }
}

impl FeeRate {
    #[inline]
    pub fn settlement_fee_atoms(self, position_magnitude: u64) -> Result<u128, MathError> {
        let position = I64F64::checked_from_num(position_magnitude).ok_or(MathError::Overflow)?;
        let product = position
            .checked_mul(self.to_i64f64())
            .ok_or(MathError::Overflow)?;
        let ceiled = product.checked_ceil().ok_or(MathError::Overflow)?;
        let atoms = ceiled.to_bits() >> I64F64::FRAC_NBITS;
        u128::try_from(atoms).map_err(|_| MathError::Overflow)
    }

    /// Converts a nonnegative cumulative fee index delta into settlement
    /// atoms. Fees use conservative ceiling rounding.
    pub fn settlement_fee_atoms_scaled(
        self,
        underlying_atoms: u128,
        underlying_decimals: u8,
        settlement_decimals: u8,
        tick_scale: u8,
    ) -> Result<u128, MathError> {
        let atoms = i128::try_from(underlying_atoms).map_err(|_| MathError::Overflow)?;
        let value = scaled_payment(ScaledPaymentParams {
            raw_value_q64_bits: self.0.to_raw_bits(),
            signed_underlying_atoms: atoms,
            time_to_maturity_seconds: None,
            seconds_per_year: 1,
            underlying_decimals,
            settlement_decimals,
            tick_scale,
            rounding: Rounding::Ceil,
        })?;
        u128::try_from(value).map_err(|_| MathError::Overflow)
    }
}

#[derive(Clone, Copy)]
enum Rounding {
    Floor,
    Ceil,
}

#[derive(Clone, Copy)]
struct ScaledPaymentParams {
    raw_value_q64_bits: i128,
    signed_underlying_atoms: i128,
    time_to_maturity_seconds: Option<i64>,
    seconds_per_year: i64,
    underlying_decimals: u8,
    settlement_decimals: u8,
    tick_scale: u8,
    rounding: Rounding,
}

fn pow10(decimals: u8) -> Result<u128, MathError> {
    if decimals > MAX_DECIMALS {
        return Err(MathError::Overflow);
    }
    crate::pow10(u32::from(decimals)).ok_or(MathError::Overflow)
}

fn gcd(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        let r = a % b;
        a = b;
        b = r;
    }
    a
}

/// Reduce numerator/denominator factors pairwise before checked multiplication.
fn reduce_factors(numerators: &mut [u128], denominators: &mut [u128]) {
    for numerator in numerators.iter_mut() {
        for denominator in denominators.iter_mut() {
            let divisor = gcd(*numerator, *denominator);
            if divisor > 1 {
                *numerator /= divisor;
                *denominator /= divisor;
            }
            if *numerator == 1 {
                break;
            }
        }
    }
}

fn checked_product(values: &[u128]) -> Result<u128, MathError> {
    values.iter().try_fold(1u128, |acc, value| {
        acc.checked_mul(*value).ok_or(MathError::Overflow)
    })
}

fn signed_abs(value: i128) -> u128 {
    value.unsigned_abs()
}

fn scaled_payment(params: ScaledPaymentParams) -> Result<i128, MathError> {
    if params.seconds_per_year <= 0 {
        return Err(MathError::InvalidDuration);
    }
    let settlement_scale = pow10(params.settlement_decimals)?;
    let underlying_scale = pow10(params.underlying_decimals)?;
    let tick_scale_factor = pow10(params.tick_scale)?;
    let mut numerators = [
        signed_abs(params.raw_value_q64_bits),
        signed_abs(params.signed_underlying_atoms),
        settlement_scale,
        1,
    ];
    let mut denominators = [1u128 << 64, underlying_scale, tick_scale_factor, 1];
    if let Some(time) = params.time_to_maturity_seconds {
        numerators[3] = u128::try_from(time).map_err(|_| MathError::InvalidDuration)?;
        denominators[3] =
            u128::try_from(params.seconds_per_year).map_err(|_| MathError::Overflow)?;
    }
    reduce_factors(&mut numerators, &mut denominators);
    let numerator = checked_product(&numerators)?;
    let denominator = checked_product(&denominators)?;
    if denominator == 0 {
        return Err(MathError::Overflow);
    }
    let negative = (params.raw_value_q64_bits < 0) ^ (params.signed_underlying_atoms < 0);
    let quotient = numerator / denominator;
    let remainder = numerator % denominator;
    let rounded = match params.rounding {
        Rounding::Floor if negative && remainder != 0 => {
            quotient.checked_add(1).ok_or(MathError::Overflow)?
        }
        Rounding::Ceil if !negative && remainder != 0 => {
            quotient.checked_add(1).ok_or(MathError::Overflow)?
        }
        _ => quotient,
    };
    let magnitude = i128::try_from(rounded).map_err(|_| MathError::Overflow)?;
    if negative {
        magnitude.checked_neg().ok_or(MathError::Overflow)
    } else {
        Ok(magnitude)
    }
}

impl RateIndex {
    #[inline]
    pub fn checked_add(self, delta: RateDelta) -> Result<Self, MathError> {
        let value = self
            .to_i64f64()
            .checked_add(delta.to_i64f64())
            .ok_or(MathError::Overflow)?;
        Ok(Self::from_i64f64(value))
    }

    #[inline]
    pub fn checked_sub(self, earlier: Self) -> Result<RateDelta, MathError> {
        let diff = self
            .to_i64f64()
            .checked_sub(earlier.to_i64f64())
            .ok_or(MathError::Overflow)?;
        Ok(RateDelta::from_i64f64(diff))
    }
}

impl FeeIndex {
    #[inline]
    pub fn checked_add(self, rate: FeeRate) -> Result<Self, MathError> {
        let value = self
            .to_i64f64()
            .checked_add(rate.to_i64f64())
            .ok_or(MathError::Overflow)?;
        Self::try_from_i64f64(value)
    }

    #[inline]
    pub fn checked_sub(self, earlier: Self) -> Result<FeeRate, MathError> {
        let diff = self
            .to_i64f64()
            .checked_sub(earlier.to_i64f64())
            .ok_or(MathError::Overflow)?;
        FeeRate::try_from_i64f64(diff)
    }
}
