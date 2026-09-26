use anchor_lang::prelude::*;
use bytemuck::{Pod, Zeroable};

use super::{StateError, StateResult};

pub const GOVERNANCE_CATEGORIES: usize = 10;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Pod, Zeroable)]
#[cfg_attr(feature = "idl-build", derive(AnchorSerialize, AnchorDeserialize))]
pub struct ProposalWindow {
    pub proposed_at: i64,
    pub execute_after: i64,
    pub execute_before: i64,
}

impl ProposalWindow {
    pub fn create(proposed_at: i64, delay_seconds: u64, expiry_seconds: u64) -> StateResult<Self> {
        if proposed_at < 0 {
            return Err(StateError::InvalidClock);
        }
        let delay = i64::try_from(delay_seconds).map_err(|_| StateError::ArithmeticOverflow)?;
        let expiry = i64::try_from(expiry_seconds).map_err(|_| StateError::ArithmeticOverflow)?;
        let execute_after = proposed_at
            .checked_add(delay)
            .ok_or(StateError::ArithmeticOverflow)?;
        let execute_before = execute_after
            .checked_add(expiry)
            .ok_or(StateError::ArithmeticOverflow)?;
        Ok(Self {
            proposed_at,
            execute_after,
            execute_before,
        })
    }

    pub fn accepts(&self, now: i64) -> StateResult<()> {
        if now < 0 {
            return Err(StateError::InvalidClock);
        }
        if now < self.execute_after || now > self.execute_before {
            return Err(StateError::Conflict);
        }
        Ok(())
    }
}

#[account(zero_copy)]
#[repr(C)]
#[derive(Debug, Eq, PartialEq)]
pub struct GlobalGovernanceConfig {
    pub authorities: [Pubkey; GOVERNANCE_CATEGORIES],
    pub increase_delays: [u64; GOVERNANCE_CATEGORIES],
    pub decrease_delays: [u64; GOVERNANCE_CATEGORIES],
    pub creation_delay_seconds: u64,
    pub proposal_expiry_seconds: u64,
    pub bump: u8,
    pub _pad: [u8; 7],
}

impl GlobalGovernanceConfig {
    pub const DATA_LEN: usize = core::mem::size_of::<Self>();
    pub const SPACE: usize = 8 + Self::DATA_LEN;

    pub fn initialize_in_place(
        &mut self,
        bump: u8,
        authority: Pubkey,
        proposal_expiry_seconds: u64,
    ) -> StateResult<()> {
        if authority == Pubkey::default() || proposal_expiry_seconds == 0 {
            return Err(StateError::InvalidConfiguration);
        }
        self.authorities = [authority; GOVERNANCE_CATEGORIES];
        self.creation_delay_seconds = 0;
        self.proposal_expiry_seconds = proposal_expiry_seconds;
        self.bump = bump;
        self._pad = [0; 7];
        Ok(())
    }

    pub fn validate(&self) -> StateResult<()> {
        if self.proposal_expiry_seconds == 0 {
            return Err(StateError::InvalidConfiguration);
        }
        Ok(())
    }

    pub fn proposal_window(
        &self,
        category: usize,
        proposed_at: i64,
        protective: bool,
    ) -> StateResult<ProposalWindow> {
        let delay = if protective {
            self.decrease_delays
                .get(category)
                .copied()
                .ok_or(StateError::InvalidConfiguration)?
        } else {
            self.increase_delays
                .get(category)
                .copied()
                .ok_or(StateError::InvalidConfiguration)?
        };
        ProposalWindow::create(proposed_at, delay, self.proposal_expiry_seconds)
    }

    pub fn replace_proposal(
        &self,
        pending: &mut ResourceGovernance,
        category: usize,
        proposed_at: i64,
        protective: bool,
    ) -> StateResult<()> {
        pending.pending = self.proposal_window(category, proposed_at, protective)?;
        pending.pending_tag = 1;
        Ok(())
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Pod, Zeroable)]
#[cfg_attr(feature = "idl-build", derive(AnchorSerialize, AnchorDeserialize))]
pub struct ResourceGovernance {
    pub authority: Pubkey,
    pub pending: ProposalWindow,
    pub pending_tag: u8,
    pub _pad: [u8; 7],
}

impl ResourceGovernance {
    pub fn clear(&mut self) {
        self.pending = ProposalWindow::zeroed();
        self.pending_tag = 0;
    }
    pub fn is_pending(&self) -> bool {
        self.pending_tag != 0
    }
    pub fn execute(&mut self, now: i64) -> StateResult<()> {
        if !self.is_pending() {
            return Err(StateError::NotFound);
        }
        self.pending.accepts(now)?;
        self.clear();
        Ok(())
    }
    pub fn cancel(&mut self) {
        self.clear();
    }
}

pub type MarketGovernance = ResourceGovernance;
pub type MarginGroupGovernance = ResourceGovernance;
pub type VaultGovernance = ResourceGovernance;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Pod, Zeroable)]
#[cfg_attr(feature = "idl-build", derive(AnchorSerialize, AnchorDeserialize))]
pub struct OptionalFixedPoint {
    pub present: u8,
    pub _pad: [u8; 7],
    pub value: super::FixedPoint,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Pod, Zeroable)]
#[cfg_attr(feature = "idl-build", derive(AnchorSerialize, AnchorDeserialize))]
pub struct OptionalU64 {
    pub present: u8,
    pub _pad: [u8; 7],
    pub value: u64,
}

/// Typed pending fields used by resource-local governance. Presence tags are
/// explicit so zero-copy state never relies on Rust bool or an untyped mask.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Pod, Zeroable)]
#[cfg_attr(feature = "idl-build", derive(AnchorSerialize, AnchorDeserialize))]
pub struct MarketMarginProposal {
    pub initial_margin_factor: OptionalFixedPoint,
    pub maintenance_margin_factor: OptionalFixedPoint,
    pub closing_margin_relief_bound_factor: OptionalFixedPoint,
    pub window: ProposalWindow,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Pod, Zeroable)]
#[cfg_attr(feature = "idl-build", derive(AnchorSerialize, AnchorDeserialize))]
pub struct GroupMarginProposal {
    pub critical_health_ratio: OptionalFixedPoint,
    pub risky_order_cancellation_ratio: OptionalFixedPoint,
    pub window: ProposalWindow,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Pod, Zeroable)]
pub struct LiquidationProposal {
    pub base_incentive: OptionalFixedPoint,
    pub incentive_slope: OptionalFixedPoint,
    pub fee_rate: OptionalFixedPoint,
    pub window: ProposalWindow,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Pod, Zeroable)]
pub struct FeeRateProposal {
    pub taker_fee_rate: OptionalFixedPoint,
    pub annualized_settlement_fee_rate: OptionalFixedPoint,
    pub rate_floor: OptionalFixedPoint,
    pub window: ProposalWindow,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Pod, Zeroable)]
pub struct MarketRiskProposal {
    pub minimum_position_size_atoms: OptionalU64,
    pub maximum_position_size_atoms: OptionalU64,
    pub hard_gross_open_interest_cap_atoms: OptionalU64,
    pub mark_observation_window_seconds: OptionalU64,
    pub margin_time_threshold_seconds: OptionalU64,
    pub order_rate_bound_factor: OptionalFixedPoint,
    pub trade_mark_update_bound_factor: OptionalFixedPoint,
    pub window: ProposalWindow,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Pod, Zeroable)]
pub struct MarketStatusProposal {
    pub status: u8,
    pub _pad: [u8; 7],
    pub window: ProposalWindow,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Pod, Zeroable)]
pub struct CircuitBreakerProposal {
    pub one_hour_limit_atoms: OptionalU64,
    pub twelve_hour_limit_atoms: OptionalU64,
    pub twenty_four_hour_limit_atoms: OptionalU64,
    pub withdrawal_paused: u8,
    pub _pad: [u8; 7],
    pub window: ProposalWindow,
}
