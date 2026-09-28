use fixed::types::I64F64;

use crate::error::MathError;

#[repr(C, align(8))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
#[cfg_attr(
    feature = "anchor-idl",
    derive(anchor_lang::AnchorSerialize, anchor_lang::AnchorDeserialize)
)]
pub struct FixedPoint {
    pub value: [u8; 16],
}

impl FixedPoint {
    #[inline]
    pub const fn from_raw_le_bytes(value: [u8; 16]) -> Self {
        Self { value }
    }

    #[inline]
    pub const fn to_raw_le_bytes(self) -> [u8; 16] {
        self.value
    }

    #[inline]
    pub const fn from_raw_bits(value: i128) -> Self {
        Self {
            value: value.to_le_bytes(),
        }
    }

    #[inline]
    pub const fn to_raw_bits(self) -> i128 {
        i128::from_le_bytes(self.value)
    }

    #[inline]
    pub const fn from_i64f64(value: I64F64) -> Self {
        Self::from_raw_bits(value.to_bits())
    }

    #[inline]
    pub const fn to_i64f64(self) -> I64F64 {
        I64F64::from_bits(self.to_raw_bits())
    }

    /// Returns the signed floor of this Q64.64 value as whole atoms,
    /// rounding toward negative infinity.
    ///
    /// Positive values round down and negative values round further down.
    /// The floor is taken after `checked_floor`, so the raw bits are an exact
    /// integer and the arithmetic shift by the fractional width is exact.
    pub fn floor_atoms(self) -> Result<i128, MathError> {
        let floored = self
            .to_i64f64()
            .checked_floor()
            .ok_or(MathError::Overflow)?;
        Ok(floored.to_bits() >> I64F64::FRAC_NBITS)
    }

    /// Returns the conservative ceiling of this Q64.64 value as whole atoms,
    /// rounding toward positive infinity.
    ///
    /// Used wherever a requirement must never be understated.
    pub fn ceil_atoms(self) -> Result<i128, MathError> {
        let ceiled = self.to_i64f64().checked_ceil().ok_or(MathError::Overflow)?;
        Ok(ceiled.to_bits() >> I64F64::FRAC_NBITS)
    }

    /// Returns the checked Q64.64 product of this value and a signed atomic
    /// integer. Multiplying raw Q64.64 bits by an integer keeps the fractional
    /// scale, so the result stays in Q64.64 without a rescale.
    pub fn checked_mul_int(self, atoms: i128) -> Result<Self, MathError> {
        let product = atoms
            .checked_mul(self.to_raw_bits())
            .ok_or(MathError::Overflow)?;
        Ok(Self::from_raw_bits(product))
    }

    /// Normalizes this whole-unit price over a signed atomic balance in one
    /// decimal step: `floor(self * atoms / 10^decimals)`, returned as whole
    /// value atoms.
    ///
    /// The oracle-native whole-unit price is applied to the atomic balance
    /// first and the decimal scale is divided out once; prices are never
    /// converted to per-atom values first. The explicit signed floor keeps
    /// positive assets valued conservatively low and negative liabilities
    /// valued conservatively high. `decimals` must satisfy `10^decimals`
    /// within the exact integer range.
    pub fn scaled_value_atoms(self, atoms: i128, decimals: u8) -> Result<i128, MathError> {
        let scale = pow10(decimals).ok_or(MathError::Overflow)?;
        let mut numerator = self.to_raw_bits().unsigned_abs();
        let mut amount = atoms.unsigned_abs();
        let mut denominator = (1u128 << I64F64::FRAC_NBITS)
            .checked_mul(scale)
            .ok_or(MathError::Overflow)?;
        let g = gcd_u128(numerator, denominator);
        numerator /= g;
        denominator /= g;
        let g = gcd_u128(amount, denominator);
        amount /= g;
        denominator /= g;
        let product = numerator.checked_mul(amount).ok_or(MathError::Overflow)?;
        let quotient = product / denominator;
        let remainder = product % denominator;
        let negative = (self.to_raw_bits() < 0) ^ (atoms < 0);
        let rounded = if negative && remainder != 0 {
            quotient.checked_add(1).ok_or(MathError::Overflow)?
        } else {
            quotient
        };
        let value = i128::try_from(rounded).map_err(|_| MathError::Overflow)?;
        if negative {
            value.checked_neg().ok_or(MathError::Overflow)
        } else {
            Ok(value)
        }
    }
}

/// Returns `10^decimals` when it fits the exact integer range, else `None`.
fn pow10(decimals: u8) -> Option<u128> {
    if decimals > 18 {
        return None;
    }
    let mut value = 1u128;
    for _ in 0..decimals {
        value = value.checked_mul(10)?;
    }
    Some(value)
}

fn gcd_u128(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        let r = a % b;
        a = b;
        b = r;
    }
    a
}
