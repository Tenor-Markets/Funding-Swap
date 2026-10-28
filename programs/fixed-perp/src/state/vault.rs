// CHECKED
use super::{FixedPoint, OracleConfig, OracleObservation, StateError, StateResult};
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
        if now < 0 {
            return Err(StateError::InvalidClock);
        }
        if self.duration_seconds == 0 {
            return Err(StateError::InvalidWithdrawalWindowDurations);
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

/// The singleton USDC custody and oracle policy. All protocol cash is USDC atoms.
#[account(zero_copy)]
#[repr(C)]
#[derive(Debug, Default, Eq, PartialEq)]
pub struct AssetState {
    pub bump: u8,
    pub status: u8,
    pub _pad_0: [u8; 14],
    pub token_account: Pubkey,
    pub oracle: OracleConfig,
    pub max_peg_deviation: FixedPoint,
    pub short_window: WithdrawalWindow,
    pub medium_window: WithdrawalWindow,
    pub long_window: WithdrawalWindow,
}

pub const USDC_MINT: Pubkey =
    Pubkey::from_str_const("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v");
/// Pyth USDC/USD: eaa020c61cc479712813461ce153894a96a6c00b21ed0cfc2798d1f9a9e9c94a.
pub const USDC_USD_FEED_ID: [u8; 32] = [
    234, 160, 32, 198, 28, 196, 121, 113, 40, 19, 70, 28, 225, 83, 137, 74, 150, 166, 192, 11, 33,
    237, 12, 252, 39, 152, 209, 249, 169, 233, 201, 74,
];
/// Phoenix quote cash is mapped one-for-one into the protocol USDC ledger.
pub const PHOENIX_QUOTE_MINT: Pubkey =
    Pubkey::from_str_const("PhUsd11YkbjSaWjFncfAAmatntsjx3MgDR9B6g1ks3A");
pub const USDC_DECIMALS: u8 = 6;
pub const USDC_SEED: &[u8] = b"usdc";

pub fn usdc_asset_address() -> Pubkey {
    Pubkey::find_program_address(&[USDC_SEED], &crate::ID).0
}

impl AssetState {
    pub const DATA_LEN: usize = core::mem::size_of::<Self>();
    pub const SPACE: usize = 8 + Self::DATA_LEN;

    #[allow(clippy::too_many_arguments)]
    pub fn initialize_in_place(
        &mut self,
        bump: u8,
        token_account: Pubkey,
        oracle: OracleConfig,
        max_peg_deviation: FixedPoint,
        window_durations: [u64; 3],
        short_window_limit_atoms: u128,
        medium_window_limit_atoms: u128,
        long_window_limit_atoms: u128,
    ) -> StateResult<()> {
        oracle.validate()?;
        if oracle.feed_id != USDC_USD_FEED_ID {
            return Err(StateError::InvalidOracleFeed);
        }
        if token_account == Pubkey::default() {
            return Err(StateError::InvalidTargetCustody);
        }
        if max_peg_deviation.to_raw_bits() < 0 || max_peg_deviation.to_raw_bits() >= (1i128 << 64) {
            return Err(StateError::InvalidOraclePolicy);
        }
        if !valid_window_durations(window_durations) {
            return Err(StateError::InvalidWithdrawalWindowDurations);
        }
        *self = Self {
            bump,
            status: ASSET_PENDING,
            _pad_0: [0; 14],
            token_account,
            oracle,
            max_peg_deviation,
            short_window: WithdrawalWindow {
                limit_atoms: short_window_limit_atoms,
                duration_seconds: window_durations[0],
                withdrawn_atoms: 0,
                epoch: 0,
            },
            medium_window: WithdrawalWindow {
                limit_atoms: medium_window_limit_atoms,
                duration_seconds: window_durations[1],
                withdrawn_atoms: 0,
                epoch: 0,
            },
            long_window: WithdrawalWindow {
                limit_atoms: long_window_limit_atoms,
                duration_seconds: window_durations[2],
                withdrawn_atoms: 0,
                epoch: 0,
            },
        };
        Ok(())
    }

    /// USDC is counted at par only after its authenticated price passes the peg gate.
    pub fn validate_usdc_price(&self, observation: OracleObservation) -> StateResult<()> {
        let deviation = observation
            .normalized_price
            .to_raw_bits()
            .checked_sub(1i128 << 64)
            .ok_or(StateError::ArithmeticOverflow)?
            .unsigned_abs();

        if deviation > self.max_peg_deviation.to_raw_bits().unsigned_abs() {
            return Err(StateError::UsdcOutsidePegBand);
        }
        Ok(())
    }

    pub fn activate(&mut self) {
        self.status = ASSET_ACTIVE;
    }

    pub fn is_active(&self) -> bool {
        self.status == ASSET_ACTIVE
    }

    pub fn is_pending(&self) -> bool {
        self.status == ASSET_PENDING
    }

    pub fn is_valid_custody(&self, token_account: Pubkey) -> bool {
        self.token_account == token_account
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
}
