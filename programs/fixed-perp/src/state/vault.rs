use super::{StateError, StateResult};
use anchor_lang::prelude::*;
use bytemuck::{Pod, Zeroable};

pub const COLLATERAL_VAULT_RESERVED: usize = 24;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Pod, Zeroable, InitSpace)]
#[cfg_attr(feature = "idl-build", derive(AnchorSerialize, AnchorDeserialize))]
pub struct WithdrawalWindow {
    pub limit_atoms: u128,
    pub withdrawn_atoms: u128,
    pub epoch: u64,
    pub _pad: [u8; 8],
}

impl WithdrawalWindow {
    fn check_and_update(&mut self, amount: u128, now: i64, duration: u64) -> StateResult<()> {
        if now < 0 || duration == 0 {
            return Err(StateError::InvalidClock);
        }
        let epoch = u64::try_from(now).map_err(|_| StateError::InvalidClock)? / duration;
        let withdrawn = if self.epoch == epoch {
            self.withdrawn_atoms
        } else {
            0
        };
        let next = withdrawn
            .checked_add(amount)
            .ok_or(StateError::ArithmeticOverflow)?;
        if next > self.limit_atoms {
            return Err(StateError::Conflict);
        }
        self.epoch = epoch;
        self.withdrawn_atoms = next;
        Ok(())
    }
}

/// Protocol binding for one mint-specific SPL collateral vault.
#[account(zero_copy)]
#[repr(C)]
#[derive(Debug, Default, InitSpace)]
pub struct CollateralVault {
    pub mint: Pubkey,
    pub token_account: Pubkey,
    pub bump: u8,
    pub withdrawal_paused: u8,
    pub _pad_0: [u8; 14],
    pub one_hour: WithdrawalWindow,
    pub twelve_hours: WithdrawalWindow,
    pub twenty_four_hours: WithdrawalWindow,
    pub reserved: [u8; COLLATERAL_VAULT_RESERVED],
    pub _pad_1: [u8; 8],
}

impl CollateralVault {
    pub fn initialize_in_place(
        &mut self,
        mint: Pubkey,
        token_account: Pubkey,
        bump: u8,
    ) -> StateResult<()> {
        if mint == Pubkey::default() || token_account == Pubkey::default() {
            return Err(StateError::InvalidConfiguration);
        }
        self.mint = mint;
        self.token_account = token_account;
        self.bump = bump;
        self.withdrawal_paused = 0;
        self._pad_0 = [0; 14];
        self.reserved = [0; COLLATERAL_VAULT_RESERVED];
        self._pad_1 = [0; 8];
        Ok(())
    }

    pub fn set_limits(&mut self, one_hour: u128, twelve_hours: u128, twenty_four_hours: u128) {
        self.one_hour.limit_atoms = one_hour;
        self.twelve_hours.limit_atoms = twelve_hours;
        self.twenty_four_hours.limit_atoms = twenty_four_hours;
    }

    pub fn check_and_commit_withdrawal(&mut self, amount: u128, now: i64) -> StateResult<()> {
        if self.withdrawal_paused != 0 {
            return Err(StateError::Conflict);
        }
        let mut one = self.one_hour;
        let mut twelve = self.twelve_hours;
        let mut day = self.twenty_four_hours;
        one.check_and_update(amount, now, 60 * 60)?;
        twelve.check_and_update(amount, now, 12 * 60 * 60)?;
        day.check_and_update(amount, now, 24 * 60 * 60)?;
        self.one_hour = one;
        self.twelve_hours = twelve;
        self.twenty_four_hours = day;
        Ok(())
    }
}
