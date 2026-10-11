// CHECKED
use super::{
    FixedPoint, PriceModel, StateError, StateResult, PRICE_MODEL_FIXED, PRICE_MODEL_FLOATING,
    PRICE_MODEL_PEGGED,
};
use anchor_lang::prelude::*;
use bytemuck::{Pod, Zeroable};

pub const ONE_HOUR_SECONDS: u64 = 60 * 60;
pub const ASSET_PENDING: u8 = 0;
pub const ASSET_ACTIVE: u8 = 1;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Pod, Zeroable, InitSpace)]
#[cfg_attr(feature = "idl-build", derive(AnchorSerialize, AnchorDeserialize))]
pub struct WithdrawalWindow {
    pub limit_atoms: u128,
    pub withdrawn_atoms: u128,
    pub epoch: u64,
    pub duration_seconds: u64,
}

fn valid_window_durations(durations: [u64; 3]) -> bool {
    durations[0] > 0 && durations[0] < durations[1] && durations[1] < durations[2]
}

impl WithdrawalWindow {
    fn check_and_update(&mut self, amount: u128, now: i64) -> StateResult<()> {
        if now < 0 || self.duration_seconds == 0 {
            return Err(StateError::InvalidClock);
        }

        let epoch =
            u64::try_from(now).map_err(|_| StateError::InvalidClock)? / self.duration_seconds;

        let withdrawn = if self.epoch == epoch {
            self.withdrawn_atoms
        } else {
            0
        };

        let next = withdrawn
            .checked_add(amount)
            .ok_or(StateError::ArithmeticOverflow)?;

        if next > self.limit_atoms {
            return Err(StateError::WithdrawalLimitExceeded);
        }

        self.epoch = epoch;
        self.withdrawn_atoms = next;

        Ok(())
    }
}

/// Zero-copy state for one protocol asset.  An asset may be purely synthetic or may
/// additionally own a token account.  The account itself is the PDA authority
/// for that token account. The mint is supplied by the instruction and authenticated
/// by the token account.
#[account(zero_copy)]
#[repr(C)]
#[derive(Debug, Default, Eq, PartialEq)]
pub struct AssetState {
    pub asset_id: u32,
    pub bump: u8,
    pub status: u8,
    pub price_model: PriceModel,
    pub decimals: u8,
    pub _pad_0: [u8; 8],
    pub token_account: Pubkey,
    pub feed_id: [u8; 32],
    pub _pad_1: [u8; 8],
    /// Fixed anchor for `Pegged`, exactly one for `Fixed` and zero for floating.
    pub reference_price: FixedPoint,
    pub max_age_seconds: u64,
    pub max_confidence_ratio: FixedPoint,
    pub max_ema_deviation: FixedPoint,
    pub max_peg_deviation: FixedPoint,
    pub short_window: WithdrawalWindow,
    pub medium_window: WithdrawalWindow,
    pub long_window: WithdrawalWindow,
    /// Resource-local governance account bound at asset creation.
    pub governance_key: Pubkey,
    pub _pad_3: [u8; 16],
}

impl AssetState {
    pub const DATA_LEN: usize = core::mem::size_of::<Self>();
    pub const SPACE: usize = 8 + Self::DATA_LEN;

    #[allow(clippy::too_many_arguments)]
    pub fn initialize_in_place(
        &mut self,
        asset_id: u32,
        bump: u8,
        governance_key: Pubkey,
        price_model: PriceModel,
        decimals: u8,
        token_account: Pubkey,
        feed_id: [u8; 32],
        reference_price: FixedPoint,
        max_age_seconds: u64,
        max_confidence_ratio: FixedPoint,
        max_ema_deviation: FixedPoint,
        max_peg_deviation: FixedPoint,
        window_durations: [u64; 3],
        short_window_limit_atoms: u128,
        medium_window_limit_atoms: u128,
        long_window_limit_atoms: u128,
    ) -> StateResult<()> {
        if asset_id == 0
            || governance_key == Pubkey::default()
            || decimals > fixed_perp_math::rate::MAX_DECIMALS
            || max_confidence_ratio.to_raw_bits() < 0
            || max_ema_deviation.to_raw_bits() < 0
            || max_peg_deviation.to_raw_bits() < 0
            || !valid_window_durations(window_durations)
        {
            return Err(StateError::InvalidConfiguration);
        }

        price_model.validate()?;

        let one = FixedPoint::from_raw_bits(1i128 << 64);

        match price_model.0 {
            PRICE_MODEL_FIXED => {
                if token_account != Pubkey::default()
                    || feed_id != [0; 32]
                    || reference_price != one
                    || max_confidence_ratio.to_raw_bits() != 0
                    || max_ema_deviation.to_raw_bits() != 0
                {
                    return Err(StateError::InvalidConfiguration);
                }
            }
            PRICE_MODEL_PEGGED => {
                if feed_id == [0; 32]
                    || reference_price.to_raw_bits() <= 0
                    || max_age_seconds == 0
                    || max_peg_deviation.to_raw_bits() >= one.to_raw_bits()
                {
                    return Err(StateError::InvalidConfiguration);
                }
            }
            PRICE_MODEL_FLOATING => {
                if feed_id == [0; 32]
                    || reference_price.to_raw_bits() != 0
                    || max_age_seconds == 0
                    || max_peg_deviation.to_raw_bits() != 0
                {
                    return Err(StateError::InvalidConfiguration);
                }
            }
            _ => return Err(StateError::InvalidTag),
        }

        self.asset_id = asset_id;
        self.bump = bump;
        self.governance_key = governance_key;
        self.status = ASSET_PENDING;
        self.price_model = price_model;
        self.decimals = decimals;
        self._pad_0 = [0; 8];
        self.token_account = token_account;
        self.feed_id = feed_id;
        self._pad_1 = [0; 8];
        self.reference_price = reference_price;
        self.max_age_seconds = max_age_seconds;
        self.max_confidence_ratio = max_confidence_ratio;
        self.max_ema_deviation = max_ema_deviation;
        self.max_peg_deviation = max_peg_deviation;
        self.short_window = WithdrawalWindow {
            limit_atoms: short_window_limit_atoms,
            duration_seconds: window_durations[0],
            ..WithdrawalWindow::default()
        };
        self.medium_window = WithdrawalWindow {
            limit_atoms: medium_window_limit_atoms,
            duration_seconds: window_durations[1],
            ..WithdrawalWindow::default()
        };
        self.long_window = WithdrawalWindow {
            limit_atoms: long_window_limit_atoms,
            duration_seconds: window_durations[2],
            ..WithdrawalWindow::default()
        };
        self._pad_3 = [0; 16];

        Ok(())
    }

    pub fn activate(
        &mut self,
        activation_window: &mut super::governance::ProposalWindow,
        now: i64,
    ) -> StateResult<()> {
        if self.status != ASSET_PENDING {
            return Err(StateError::InvalidStatus);
        }

        activation_window.consume(now)?;
        self.status = ASSET_ACTIVE;

        Ok(())
    }

    pub fn is_active(&self) -> bool {
        self.status == ASSET_ACTIVE
    }

    pub fn is_pending(&self) -> bool {
        self.status == ASSET_PENDING
    }

    pub fn is_governance_key(&self, key: Pubkey) -> bool {
        self.governance_key == key
    }

    pub fn has_custody(&self) -> bool {
        self.token_account != Pubkey::default()
    }

    pub fn is_valid_custody(&self, token_account: Pubkey) -> bool {
        self.has_custody() && self.token_account == token_account
    }

    pub fn validate_feed(&self, feed_id: [u8; 32]) -> StateResult<()> {
        if self.price_model.0 != PRICE_MODEL_FIXED
            && (self.feed_id == [0; 32] || self.feed_id != feed_id)
        {
            return Err(StateError::InvalidReference);
        }

        Ok(())
    }

    pub fn set_limits(&mut self, short: u128, medium: u128, long: u128) {
        self.short_window.limit_atoms = short;
        self.medium_window.limit_atoms = medium;
        self.long_window.limit_atoms = long;
    }

    /// Updates all withdrawal windows in sequence. Callers must propagate an
    /// error and stop the surrounding operation. An earlier window may already
    /// have been updated when a later window rejects the withdrawal.
    pub fn check_and_commit_withdrawal(&mut self, amount: u128, now: i64) -> StateResult<()> {
        self.short_window.check_and_update(amount, now)?;
        self.medium_window.check_and_update(amount, now)?;
        self.long_window.check_and_update(amount, now)?;
        Ok(())
    }

    /// Returns whether the authenticated central price remains inside the
    /// configured peg band.  Confidence is checked separately by the caller;
    /// a wide confidence interval does not by itself force the whole interval
    /// to fit inside the band.
    pub fn central_price_is_on_peg(&self, price: FixedPoint) -> StateResult<bool> {
        if self.price_model.0 != PRICE_MODEL_PEGGED {
            return Ok(false);
        }

        let anchor = self.reference_price.to_i64f64();
        let deviation = self.max_peg_deviation.to_i64f64();

        let lower = anchor
            .checked_mul(
                fixed::types::I64F64::ONE
                    .checked_sub(deviation)
                    .ok_or(StateError::ArithmeticOverflow)?,
            )
            .ok_or(StateError::ArithmeticOverflow)?;

        let upper = anchor
            .checked_mul(
                fixed::types::I64F64::ONE
                    .checked_add(deviation)
                    .ok_or(StateError::ArithmeticOverflow)?,
            )
            .ok_or(StateError::ArithmeticOverflow)?;

        let value = price.to_i64f64();

        Ok(value >= lower && value <= upper)
    }

    /// Builds a transient observation from this asset's configured source.
    /// Fixed assets need no feed account; all other models require the caller
    /// to provide the authenticated Pyth update account.
    pub fn observation(
        &self,
        feed_account: Option<&AccountInfo<'_>>,
        now: i64,
    ) -> StateResult<super::OracleObservation> {
        if self.price_model.0 == PRICE_MODEL_FIXED {
            super::OracleObservation::fixed(self, now)
        } else {
            let account = feed_account.ok_or(StateError::InvalidReference)?;
            super::OracleObservation::from_pyth_account(account, self, now)
        }
    }
}

/// Delayed authorization to withdraw booked protocol fees from one asset.
/// The executable amount is computed from matured fee ledgers, not stored here.
#[account]
#[derive(Debug, InitSpace)]
pub struct PendingProtocolFeeCollection {
    pub asset: Pubkey,
    pub destination: Pubkey,
    pub rent_recipient: Pubkey,
    pub window: super::ProposalWindow,
}

impl PendingProtocolFeeCollection {
    pub const SPACE: usize = 8 + Self::INIT_SPACE;

    pub fn initialize_in_place(
        &mut self,
        asset: Pubkey,
        destination: Pubkey,
        rent_recipient: Pubkey,
        window: super::ProposalWindow,
    ) -> StateResult<()> {
        if asset == Pubkey::default()
            || destination == Pubkey::default()
            || rent_recipient == Pubkey::default()
        {
            return Err(StateError::InvalidConfiguration);
        }

        self.asset = asset;
        self.destination = destination;
        self.rent_recipient = rent_recipient;
        self.window = window;

        Ok(())
    }

    pub fn is_same_asset(&self, key: Pubkey) -> bool {
        self.asset == key
    }

    pub fn is_same_destination(&self, key: Pubkey) -> bool {
        self.destination == key
    }

    pub fn is_same_rent_recipient(&self, key: Pubkey) -> bool {
        self.rent_recipient == key
    }
}
