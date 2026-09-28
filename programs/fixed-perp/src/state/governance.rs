use anchor_lang::prelude::*;
use bytemuck::{Pod, Zeroable};

use super::{StateError, StateResult};

pub const GOVERNANCE_CATEGORIES: usize = 10;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GlobalGovernancePolicy {
    pub authorities: [Pubkey; GOVERNANCE_CATEGORIES],
    pub increase_delays: [u64; GOVERNANCE_CATEGORIES],
    pub decrease_delays: [u64; GOVERNANCE_CATEGORIES],
    pub creation_delay_seconds: u64,
    pub proposal_expiry_seconds: u64,
    pub subaccount_creation_fee_lamports: u64,
    pub subaccount_creation_fee_destination: Pubkey,
}

/// Global governance authority categories and their stable configuration
/// indices. The numeric order is part of the zero-copy account ABI.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "idl-build", derive(AnchorSerialize, AnchorDeserialize))]
#[cfg_attr(feature = "idl-build", borsh(use_discriminant = true))]
pub enum GovernanceCategory {
    /// Market margin parameters.
    MarketMargin = 0,
    /// Shared margin-group parameters.
    GroupMargin = 1,
    /// Liquidation parameters.
    Liquidation = 2,
    /// Fee and rate parameters.
    FeeRate = 3,
    /// Market risk parameters.
    MarketRisk = 4,
    /// Market status transitions.
    MarketStatus = 5,
    /// AssetState circuit-breaker parameters.
    CircuitBreaker = 6,
    /// AssetState and custody actions.
    AssetCustody = 7,
    /// Resource creation and relationships.
    Creation = 8,
    /// Governance authority and policy.
    Governance = 9,
}

impl GovernanceCategory {
    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    pub const fn index(self) -> usize {
        self.as_u8() as usize
    }
}

impl TryFrom<u8> for GovernanceCategory {
    type Error = StateError;

    fn try_from(value: u8) -> StateResult<Self> {
        match value {
            0 => Ok(Self::MarketMargin),
            1 => Ok(Self::GroupMargin),
            2 => Ok(Self::Liquidation),
            3 => Ok(Self::FeeRate),
            4 => Ok(Self::MarketRisk),
            5 => Ok(Self::MarketStatus),
            6 => Ok(Self::CircuitBreaker),
            7 => Ok(Self::AssetCustody),
            8 => Ok(Self::Creation),
            9 => Ok(Self::Governance),
            _ => Err(StateError::InvalidTag),
        }
    }
}

/// Direction policy for a governed field. The live and target values determine
/// the numeric delay direction; `BothDelayed` opts into the strongest delay.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChangePolicy {
    /// Field has a semantically protective increasing direction.
    IncreaseProtective,
    /// Field has a semantically protective decreasing direction.
    DecreaseProtective,
    /// Neither direction is unambiguously protective, so both use the
    /// strongest configured delay.
    BothDelayed,
}

#[repr(C)]
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    Eq,
    PartialEq,
    Pod,
    Zeroable,
    InitSpace,
    AnchorSerialize,
    AnchorDeserialize,
)]
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
        if expiry_seconds == 0 {
            return Err(StateError::InvalidConfiguration);
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

    /// Validates that `now` is inside this window and clears it atomically.
    pub fn consume(&mut self, now: i64) -> StateResult<()> {
        self.accepts(now)?;
        *self = Self::zeroed();
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
    pub subaccount_creation_fee_lamports: u64,
    pub subaccount_creation_fee_destination: Pubkey,
    pub pending_governance: GlobalGovernanceProposal,
    pub bump: u8,
    pub _pad: [u8; 7],
}

fn validate_governance_policy(
    authorities: &[Pubkey; GOVERNANCE_CATEGORIES],
    increase_delays: &[u64; GOVERNANCE_CATEGORIES],
    decrease_delays: &[u64; GOVERNANCE_CATEGORIES],
    creation_delay_seconds: u64,
    proposal_expiry_seconds: u64,
    subaccount_creation_fee_destination: Pubkey,
) -> StateResult<()> {
    let valid_duration = |seconds: u64| i64::try_from(seconds).is_ok();
    if authorities
        .iter()
        .any(|authority| *authority == Pubkey::default())
        || increase_delays.iter().any(|delay| !valid_duration(*delay))
        || decrease_delays.iter().any(|delay| !valid_duration(*delay))
        || !valid_duration(creation_delay_seconds)
        || proposal_expiry_seconds == 0
        || !valid_duration(proposal_expiry_seconds)
        || subaccount_creation_fee_destination == Pubkey::default()
    {
        return Err(StateError::InvalidConfiguration);
    }
    Ok(())
}

impl GlobalGovernanceConfig {
    pub const DATA_LEN: usize = core::mem::size_of::<Self>();
    pub const SPACE: usize = 8 + Self::DATA_LEN;

    pub fn is_creation_fee_destination(&self, destination: Pubkey) -> bool {
        self.subaccount_creation_fee_destination == destination
    }

    pub fn initialize_in_place(
        &mut self,
        bump: u8,
        authority: Pubkey,
        proposal_expiry_seconds: u64,
    ) -> StateResult<()> {
        validate_governance_policy(
            &[authority; GOVERNANCE_CATEGORIES],
            &[0; GOVERNANCE_CATEGORIES],
            &[0; GOVERNANCE_CATEGORIES],
            0,
            proposal_expiry_seconds,
            authority,
        )?;
        self.authorities = [authority; GOVERNANCE_CATEGORIES];
        self.increase_delays = [0; GOVERNANCE_CATEGORIES];
        self.decrease_delays = [0; GOVERNANCE_CATEGORIES];
        self.creation_delay_seconds = 0;
        self.proposal_expiry_seconds = proposal_expiry_seconds;
        self.subaccount_creation_fee_lamports = 0;
        self.subaccount_creation_fee_destination = authority;
        self.pending_governance = GlobalGovernanceProposal::zeroed();
        self.bump = bump;
        self._pad = [0; 7];
        Ok(())
    }

    pub fn initialize_with_policy(
        &mut self,
        bump: u8,
        policy: GlobalGovernancePolicy,
    ) -> StateResult<()> {
        validate_governance_policy(
            &policy.authorities,
            &policy.increase_delays,
            &policy.decrease_delays,
            policy.creation_delay_seconds,
            policy.proposal_expiry_seconds,
            policy.subaccount_creation_fee_destination,
        )?;
        self.authorities = policy.authorities;
        self.increase_delays = policy.increase_delays;
        self.decrease_delays = policy.decrease_delays;
        self.creation_delay_seconds = policy.creation_delay_seconds;
        self.proposal_expiry_seconds = policy.proposal_expiry_seconds;
        self.subaccount_creation_fee_lamports = policy.subaccount_creation_fee_lamports;
        self.subaccount_creation_fee_destination = policy.subaccount_creation_fee_destination;
        self.pending_governance = GlobalGovernanceProposal::zeroed();
        self.bump = bump;
        self._pad = [0; 7];
        Ok(())
    }

    pub fn validate(&self) -> StateResult<()> {
        validate_governance_policy(
            &self.authorities,
            &self.increase_delays,
            &self.decrease_delays,
            self.creation_delay_seconds,
            self.proposal_expiry_seconds,
            self.subaccount_creation_fee_destination,
        )
    }

    /// Computes one execution window for all changed fields. Numeric direction
    /// is derived from each live/target pair, while `BothDelayed` selects the
    /// stronger configured delay; the maximum applies to the whole proposal.
    pub fn proposal_window_for_values(
        &self,
        category: GovernanceCategory,
        proposed_at: i64,
        changes: &[(i128, i128, ChangePolicy)],
    ) -> StateResult<ProposalWindow> {
        let increase_delay = self
            .increase_delays
            .get(category.index())
            .copied()
            .ok_or(StateError::InvalidConfiguration)?;
        let decrease_delay = self
            .decrease_delays
            .get(category.index())
            .copied()
            .ok_or(StateError::InvalidConfiguration)?;
        let mut delay = 0u64;
        for (current, target, policy) in changes {
            if current == target {
                continue;
            }
            let field_delay = match policy {
                ChangePolicy::IncreaseProtective if target > current => 0,
                ChangePolicy::DecreaseProtective if target < current => 0,
                ChangePolicy::IncreaseProtective => decrease_delay,
                ChangePolicy::DecreaseProtective => increase_delay,
                ChangePolicy::BothDelayed => increase_delay.max(decrease_delay),
            };
            delay = delay.max(field_delay);
        }
        ProposalWindow::create(proposed_at, delay, self.proposal_expiry_seconds)
    }

    pub fn authority_for(&self, category: GovernanceCategory, authority: Pubkey) -> bool {
        self.authorities
            .get(category.index())
            .is_some_and(|expected| *expected != Pubkey::default() && *expected == authority)
    }

    pub fn set_authority(
        &mut self,
        category: GovernanceCategory,
        authority: Pubkey,
    ) -> StateResult<()> {
        if authority == Pubkey::default() {
            return Err(StateError::InvalidAuthority);
        }
        *self
            .authorities
            .get_mut(category.index())
            .ok_or(StateError::InvalidConfiguration)? = authority;
        Ok(())
    }

    pub fn replace_governance_proposal(
        &mut self,
        mut replacement: GlobalGovernanceProposal,
        proposed_at: i64,
    ) -> StateResult<()> {
        if !replacement.is_pending() {
            return Err(StateError::InvalidConfiguration);
        }
        let has_global_update = replacement.creation_delay_seconds.is_present()
            || replacement.proposal_expiry_seconds.is_present()
            || replacement.subaccount_creation_fee_lamports.is_present()
            || replacement.subaccount_creation_fee_destination.is_present();
        let has_category_update = replacement.new_authority.is_present()
            || replacement.increase_delay_seconds.is_present()
            || replacement.decrease_delay_seconds.is_present();
        if !has_global_update && !has_category_update {
            return Err(StateError::InvalidConfiguration);
        }
        if replacement.target_category.is_present() != has_category_update {
            return Err(StateError::InvalidConfiguration);
        }
        if has_category_update {
            GovernanceCategory::try_from(replacement.target_category.value)?;
        }
        if replacement.new_authority.is_present()
            && replacement.new_authority.value == Pubkey::default()
        {
            return Err(StateError::InvalidAuthority);
        }
        if replacement.subaccount_creation_fee_destination.is_present()
            && replacement.subaccount_creation_fee_destination.value == Pubkey::default()
        {
            return Err(StateError::InvalidConfiguration);
        }
        for duration in [
            replacement.increase_delay_seconds,
            replacement.decrease_delay_seconds,
            replacement.creation_delay_seconds,
            replacement.proposal_expiry_seconds,
        ] {
            if duration.is_present() && i64::try_from(duration.value).is_err() {
                return Err(StateError::InvalidConfiguration);
            }
        }
        if replacement.proposal_expiry_seconds.is_present()
            && replacement.proposal_expiry_seconds.value == 0
        {
            return Err(StateError::InvalidConfiguration);
        }
        let strongest_delay = self.increase_delays[GovernanceCategory::Governance.index()]
            .max(self.decrease_delays[GovernanceCategory::Governance.index()]);

        replacement.window =
            ProposalWindow::create(proposed_at, strongest_delay, self.proposal_expiry_seconds)?;

        self.pending_governance = replacement;
        Ok(())
    }

    pub fn cancel_governance_proposal(&mut self) {
        self.pending_governance.clear();
    }

    pub fn execute_governance_proposal(&mut self, now: i64) -> StateResult<()> {
        require_executable(&self.pending_governance, now)?;
        let proposal = self.pending_governance;
        let mut next_authorities = self.authorities;
        let mut next_increase_delays = self.increase_delays;
        let mut next_decrease_delays = self.decrease_delays;
        let mut next_creation_delay = self.creation_delay_seconds;
        let mut next_expiry = self.proposal_expiry_seconds;
        let mut next_fee = self.subaccount_creation_fee_lamports;
        let mut next_destination = self.subaccount_creation_fee_destination;

        if proposal.target_category.is_present() {
            let category = GovernanceCategory::try_from(proposal.target_category.value)?;
            if proposal.new_authority.is_present() {
                next_authorities[category.index()] = proposal.new_authority.value;
            }
            if proposal.increase_delay_seconds.is_present() {
                next_increase_delays[category.index()] = proposal.increase_delay_seconds.value;
            }
            if proposal.decrease_delay_seconds.is_present() {
                next_decrease_delays[category.index()] = proposal.decrease_delay_seconds.value;
            }
        }
        if proposal.creation_delay_seconds.is_present() {
            next_creation_delay = proposal.creation_delay_seconds.value;
        }
        if proposal.proposal_expiry_seconds.is_present() {
            next_expiry = proposal.proposal_expiry_seconds.value;
        }
        if proposal.subaccount_creation_fee_lamports.is_present() {
            next_fee = proposal.subaccount_creation_fee_lamports.value;
        }
        if proposal.subaccount_creation_fee_destination.is_present() {
            next_destination = proposal.subaccount_creation_fee_destination.value;
        }

        validate_governance_policy(
            &next_authorities,
            &next_increase_delays,
            &next_decrease_delays,
            next_creation_delay,
            next_expiry,
            next_destination,
        )?;

        self.authorities = next_authorities;
        self.increase_delays = next_increase_delays;
        self.decrease_delays = next_decrease_delays;
        self.creation_delay_seconds = next_creation_delay;
        self.proposal_expiry_seconds = next_expiry;
        self.subaccount_creation_fee_lamports = next_fee;
        self.subaccount_creation_fee_destination = next_destination;
        self.pending_governance.clear();
        Ok(())
    }
}

fn require_executable<P: TypedProposal>(proposal: &P, now: i64) -> StateResult<()> {
    if !proposal.is_pending() {
        return Err(StateError::NotFound);
    }
    proposal.window().accepts(now)
}

impl MarketMarginProposal {
    /// Applies a ready proposal to a complete staged market configuration.
    pub fn execute_into(&mut self, config: &mut super::MarketConfig, now: i64) -> StateResult<()> {
        require_executable(self, now)?;
        let mut next = *config;
        next.initial_margin_factor = self
            .initial_margin_factor
            .resolve(next.initial_margin_factor);
        next.maintenance_margin_factor = self
            .maintenance_margin_factor
            .resolve(next.maintenance_margin_factor);
        next.closing_margin_relief_bound_factor = self
            .closing_margin_relief_bound_factor
            .resolve(next.closing_margin_relief_bound_factor);
        next.validate()?;
        *config = next;
        self.clear();
        Ok(())
    }
}

impl LiquidationProposal {
    pub fn execute_into(&mut self, config: &mut super::MarketConfig, now: i64) -> StateResult<()> {
        require_executable(self, now)?;
        let mut next = *config;
        next.liquidation_base_incentive =
            self.base_incentive.resolve(next.liquidation_base_incentive);
        next.liquidation_incentive_slope = self
            .incentive_slope
            .resolve(next.liquidation_incentive_slope);
        next.liquidation_fee_rate = self.fee_rate.resolve(next.liquidation_fee_rate);
        next.validate()?;
        *config = next;
        self.clear();
        Ok(())
    }
}

impl FeeRateProposal {
    pub fn execute_into(&mut self, config: &mut super::MarketConfig, now: i64) -> StateResult<()> {
        require_executable(self, now)?;
        let mut next = *config;
        next.taker_fee_rate = self.taker_fee_rate.resolve(next.taker_fee_rate);
        next.annualized_settlement_fee_rate = self
            .annualized_settlement_fee_rate
            .resolve(next.annualized_settlement_fee_rate);
        let previous_rate_floor = next.rate_floor;
        next.rate_floor = self.rate_floor.resolve(next.rate_floor);
        next.validate()?;
        if next.rate_floor != previous_rate_floor {
            next.pm_cache_nonce = next
                .pm_cache_nonce
                .checked_add(1)
                .ok_or(StateError::ArithmeticOverflow)?;
        }
        *config = next;
        self.clear();
        Ok(())
    }
}

impl MarketRiskProposal {
    pub fn execute_into(&mut self, config: &mut super::MarketConfig, now: i64) -> StateResult<()> {
        require_executable(self, now)?;
        let mut next = *config;
        next.minimum_position_size_atoms = self
            .minimum_position_size_atoms
            .resolve(next.minimum_position_size_atoms);
        next.maximum_position_size_atoms = self
            .maximum_position_size_atoms
            .resolve(next.maximum_position_size_atoms);
        next.hard_gross_open_interest_cap_atoms = self
            .hard_gross_open_interest_cap_atoms
            .resolve(next.hard_gross_open_interest_cap_atoms);
        let mark_window = self
            .mark_observation_window_seconds
            .resolve(u64::from(next.mark_observation_window_seconds));
        next.mark_observation_window_seconds =
            u32::try_from(mark_window).map_err(|_| StateError::ArithmeticOverflow)?;
        let margin_threshold = self
            .margin_time_threshold_seconds
            .resolve(u64::from(next.margin_time_threshold_seconds));
        next.margin_time_threshold_seconds =
            u32::try_from(margin_threshold).map_err(|_| StateError::ArithmeticOverflow)?;
        next.order_rate_bound_factor = self
            .order_rate_bound_factor
            .resolve(next.order_rate_bound_factor);
        next.trade_mark_update_bound_factor = self
            .trade_mark_update_bound_factor
            .resolve(next.trade_mark_update_bound_factor);
        next.validate()?;
        *config = next;
        self.clear();
        Ok(())
    }
}

impl MarketStatusProposal {
    pub fn execute_into(&mut self, market: &mut super::MarketState, now: i64) -> StateResult<()> {
        require_executable(self, now)?;
        if !matches!(
            market.status.0,
            super::MARKET_ACTIVE | super::MARKET_CLOSE_ONLY | super::MARKET_PAUSED
        ) {
            // Initial activation and future maturity finalization have their
            // own invariants and cannot be bypassed as a status update.
            return Err(StateError::InvalidStatus);
        }
        let next = self.status.resolve(market.status.0);
        if !matches!(
            next,
            super::MARKET_ACTIVE | super::MARKET_CLOSE_ONLY | super::MARKET_PAUSED
        ) {
            return Err(StateError::InvalidStatus);
        }
        market.status.0 = next;
        self.clear();
        Ok(())
    }
}

impl GroupMarginProposal {
    pub fn execute_into(&mut self, group: &mut super::MarginGroup, now: i64) -> StateResult<()> {
        require_executable(self, now)?;
        let critical = self
            .critical_health_ratio
            .resolve(group.critical_health_ratio);
        let risky = self
            .risky_order_cancellation_ratio
            .resolve(group.risky_order_cancellation_ratio);
        if critical.to_raw_bits() <= 0 || risky.to_raw_bits() < critical.to_raw_bits() {
            return Err(StateError::InvalidConfiguration);
        }
        group.critical_health_ratio = critical;
        group.risky_order_cancellation_ratio = risky;
        self.clear();
        Ok(())
    }
}

impl CircuitBreakerProposal {
    /// Applies circuit-breaker changes to the unified AssetState account.
    pub fn execute_into_asset(
        &mut self,
        asset: &mut super::AssetState,
        now: i64,
    ) -> StateResult<()> {
        require_executable(self, now)?;
        let short = self
            .short_window_limit_atoms
            .resolve(asset.short_window.limit_atoms);
        let medium = self
            .medium_window_limit_atoms
            .resolve(asset.medium_window.limit_atoms);
        let long = self
            .long_window_limit_atoms
            .resolve(asset.long_window.limit_atoms);
        asset.set_limits(short, medium, long);
        self.clear();
        Ok(())
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Pod, Zeroable)]
#[cfg_attr(feature = "idl-build", derive(AnchorSerialize, AnchorDeserialize))]
pub struct OptionalFixedPoint {
    pub present: u8,
    pub _pad: [u8; 7],
    pub value: super::FixedPoint,
}

impl OptionalFixedPoint {
    pub fn some(value: super::FixedPoint) -> Self {
        Self {
            present: 1,
            _pad: [0; 7],
            value,
        }
    }

    pub fn none() -> Self {
        Self::zeroed()
    }

    pub fn is_present(&self) -> bool {
        self.present != 0
    }

    pub fn clear(&mut self) {
        *self = Self::none();
    }

    fn resolve(self, current: super::FixedPoint) -> super::FixedPoint {
        if self.is_present() {
            self.value
        } else {
            current
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Pod, Zeroable)]
#[cfg_attr(feature = "idl-build", derive(AnchorSerialize, AnchorDeserialize))]
pub struct OptionalU64 {
    pub present: u8,
    pub _pad: [u8; 7],
    pub value: u64,
}

/// Fixed-layout optional underlying-atom value. The explicit 15-byte pad
/// places the `u128` at its natural 16-byte alignment without implicit bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Pod, Zeroable)]
#[cfg_attr(feature = "idl-build", derive(AnchorSerialize, AnchorDeserialize))]
pub struct OptionalU128 {
    pub present: u8,
    pub _pad: [u8; 15],
    pub value: u128,
}

impl OptionalU128 {
    pub fn some(value: u128) -> Self {
        Self {
            present: 1,
            _pad: [0; 15],
            value,
        }
    }

    pub fn none() -> Self {
        Self::zeroed()
    }

    pub fn is_present(&self) -> bool {
        self.present != 0
    }

    pub fn clear(&mut self) {
        *self = Self::none();
    }

    fn resolve(self, current: u128) -> u128 {
        if self.is_present() {
            self.value
        } else {
            current
        }
    }
}

impl OptionalU64 {
    pub fn some(value: u64) -> Self {
        Self {
            present: 1,
            _pad: [0; 7],
            value,
        }
    }

    pub fn none() -> Self {
        Self::zeroed()
    }

    pub fn is_present(&self) -> bool {
        self.present != 0
    }

    pub fn clear(&mut self) {
        *self = Self::none();
    }

    fn resolve(self, current: u64) -> u64 {
        if self.is_present() {
            self.value
        } else {
            current
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Pod, Zeroable)]
#[cfg_attr(feature = "idl-build", derive(AnchorSerialize, AnchorDeserialize))]
pub struct OptionalU8 {
    pub present: u8,
    pub value: u8,
    pub _pad: [u8; 6],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Pod, Zeroable)]
#[cfg_attr(feature = "idl-build", derive(AnchorSerialize, AnchorDeserialize))]
pub struct OptionalPubkey {
    pub present: u8,
    pub _pad: [u8; 7],
    pub value: Pubkey,
}

impl OptionalPubkey {
    pub fn some(value: Pubkey) -> Self {
        Self {
            present: 1,
            _pad: [0; 7],
            value,
        }
    }

    pub fn none() -> Self {
        Self::zeroed()
    }

    pub fn is_present(&self) -> bool {
        self.present != 0
    }

    pub fn clear(&mut self) {
        *self = Self::none();
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Pod, Zeroable)]
#[cfg_attr(feature = "idl-build", derive(AnchorSerialize, AnchorDeserialize))]
pub struct GlobalGovernanceProposal {
    pub new_authority: OptionalPubkey,
    pub subaccount_creation_fee_destination: OptionalPubkey,
    pub increase_delay_seconds: OptionalU64,
    pub decrease_delay_seconds: OptionalU64,
    pub creation_delay_seconds: OptionalU64,
    pub proposal_expiry_seconds: OptionalU64,
    pub subaccount_creation_fee_lamports: OptionalU64,
    pub target_category: OptionalU8,
    pub window: ProposalWindow,
}

impl OptionalU8 {
    pub fn some(value: u8) -> Self {
        Self {
            present: 1,
            value,
            _pad: [0; 6],
        }
    }

    pub fn none() -> Self {
        Self::zeroed()
    }

    pub fn is_present(&self) -> bool {
        self.present != 0
    }

    pub fn clear(&mut self) {
        *self = Self::none();
    }

    fn resolve(self, current: u8) -> u8 {
        if self.is_present() {
            self.value
        } else {
            current
        }
    }
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
#[cfg_attr(feature = "idl-build", derive(AnchorSerialize, AnchorDeserialize))]
pub struct LiquidationProposal {
    pub base_incentive: OptionalFixedPoint,
    pub incentive_slope: OptionalFixedPoint,
    pub fee_rate: OptionalFixedPoint,
    pub window: ProposalWindow,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Pod, Zeroable)]
#[cfg_attr(feature = "idl-build", derive(AnchorSerialize, AnchorDeserialize))]
pub struct FeeRateProposal {
    pub taker_fee_rate: OptionalFixedPoint,
    pub annualized_settlement_fee_rate: OptionalFixedPoint,
    pub rate_floor: OptionalFixedPoint,
    pub window: ProposalWindow,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Pod, Zeroable)]
#[cfg_attr(feature = "idl-build", derive(AnchorSerialize, AnchorDeserialize))]
pub struct MarketRiskProposal {
    pub minimum_position_size_atoms: OptionalU64,
    pub maximum_position_size_atoms: OptionalU64,
    pub hard_gross_open_interest_cap_atoms: OptionalU128,
    pub mark_observation_window_seconds: OptionalU64,
    pub margin_time_threshold_seconds: OptionalU64,
    pub order_rate_bound_factor: OptionalFixedPoint,
    pub trade_mark_update_bound_factor: OptionalFixedPoint,
    pub window: ProposalWindow,
    pub _pad: [u8; 8],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Pod, Zeroable)]
#[cfg_attr(feature = "idl-build", derive(AnchorSerialize, AnchorDeserialize))]
pub struct MarketStatusProposal {
    pub status: OptionalU8,
    pub window: ProposalWindow,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Pod, Zeroable)]
#[cfg_attr(feature = "idl-build", derive(AnchorSerialize, AnchorDeserialize))]
pub struct CircuitBreakerProposal {
    pub short_window_limit_atoms: OptionalU128,
    pub medium_window_limit_atoms: OptionalU128,
    pub long_window_limit_atoms: OptionalU128,
    pub window: ProposalWindow,
    pub _pad: [u8; 8],
}

/// Pending proposals scoped to one market. Operational values remain in the
/// market account; this account only owns the category-local pending payloads.
#[account(zero_copy)]
#[repr(C)]
#[derive(Debug, Eq, PartialEq)]
pub struct MarketGovernance {
    pub market_margin: MarketMarginProposal,
    pub liquidation: LiquidationProposal,
    pub fee_rate: FeeRateProposal,
    pub market_risk: MarketRiskProposal,
    pub market_status: MarketStatusProposal,
    pub activation_window: ProposalWindow,
    pub _pad: [u8; 8],
}

impl MarketGovernance {
    pub const DATA_LEN: usize = core::mem::size_of::<Self>();
    pub const SPACE: usize = 8 + Self::DATA_LEN;
}

/// Pending proposals scoped to one margin group.
#[account(zero_copy)]
#[repr(C)]
#[derive(Debug, Eq, PartialEq)]
pub struct MarginGroupGovernance {
    pub group_margin: GroupMarginProposal,
    pub activation_window: ProposalWindow,
}

impl MarginGroupGovernance {
    pub const DATA_LEN: usize = core::mem::size_of::<Self>();
    pub const SPACE: usize = 8 + Self::DATA_LEN;
}

/// Pending circuit-breaker proposals scoped to one AssetState account.
#[account(zero_copy)]
#[repr(C)]
#[derive(Debug, Eq, PartialEq)]
pub struct AssetGovernance {
    pub circuit_breaker: CircuitBreakerProposal,
    pub activation_window: ProposalWindow,
    pub _pad: [u8; 8],
}

macro_rules! impl_activation_window {
    ($name:ty, [$($field:ident),+ $(,)?]) => {
        impl $name {
            /// Initializes the resource activation proposal from a complete
            /// coordinator snapshot and clears any stale typed proposals.
            pub fn initialize_in_place(
                &mut self,
                activation_window: ProposalWindow,
            ) -> StateResult<()> {
                self.replace_activation_window(activation_window)
            }

            /// Replaces the activation proposal snapshot and clears every
            /// category-local pending payload from the prior proposal.
            pub fn replace_activation_window(
                &mut self,
                activation_window: ProposalWindow,
            ) -> StateResult<()> {
                $(self.$field.clear();)+
                self.activation_window = activation_window;
                Ok(())
            }
        }
    };
}

impl_activation_window!(
    MarketGovernance,
    [
        market_margin,
        liquidation,
        fee_rate,
        market_risk,
        market_status
    ]
);
impl_activation_window!(MarginGroupGovernance, [group_margin]);
impl_activation_window!(AssetGovernance, [circuit_breaker]);

impl AssetGovernance {
    pub const DATA_LEN: usize = core::mem::size_of::<Self>();
    pub const SPACE: usize = 8 + Self::DATA_LEN;
}

/// Common lifecycle operations for typed pending proposals. The presence tags
/// on the individual fields are the pending discriminator; no proposal-level
/// boolean is needed.
pub trait TypedProposal {
    fn is_pending(&self) -> bool;
    fn clear(&mut self);
    fn cancel(&mut self) {
        self.clear();
    }
    fn execute(&mut self, now: i64) -> StateResult<()>;
    fn window(&self) -> ProposalWindow;
    fn set_window(&mut self, window: ProposalWindow);
}

macro_rules! impl_typed_proposal {
    ($name:ty, [$($field:ident),+ $(,)?]) => {
        impl $name {
            pub fn is_pending(&self) -> bool {
                $(self.$field.is_present())||+
            }

            pub fn clear(&mut self) {
                *self = Self::default();
            }

            pub fn cancel(&mut self) {
                self.clear();
            }

            pub fn execute(&mut self, now: i64) -> StateResult<()> {
                if !self.is_pending() {
                    return Err(StateError::NotFound);
                }
                self.window.accepts(now)?;
                self.clear();
                Ok(())
            }
        }

        impl TypedProposal for $name {
            fn is_pending(&self) -> bool {
                Self::is_pending(self)
            }

            fn clear(&mut self) {
                Self::clear(self);
            }

            fn execute(&mut self, now: i64) -> StateResult<()> {
                Self::execute(self, now)
            }

            fn window(&self) -> ProposalWindow {
                self.window
            }

            fn set_window(&mut self, window: ProposalWindow) {
                self.window = window;
            }
        }
    };
}

impl_typed_proposal!(
    MarketMarginProposal,
    [
        initial_margin_factor,
        maintenance_margin_factor,
        closing_margin_relief_bound_factor
    ]
);
impl_typed_proposal!(MarketStatusProposal, [status]);
impl_typed_proposal!(
    GlobalGovernanceProposal,
    [
        new_authority,
        subaccount_creation_fee_destination,
        increase_delay_seconds,
        decrease_delay_seconds,
        creation_delay_seconds,
        proposal_expiry_seconds,
        subaccount_creation_fee_lamports,
        target_category
    ]
);
impl_typed_proposal!(
    CircuitBreakerProposal,
    [
        short_window_limit_atoms,
        medium_window_limit_atoms,
        long_window_limit_atoms
    ]
);
impl_typed_proposal!(
    GroupMarginProposal,
    [critical_health_ratio, risky_order_cancellation_ratio]
);
impl_typed_proposal!(
    LiquidationProposal,
    [base_incentive, incentive_slope, fee_rate]
);
impl_typed_proposal!(
    FeeRateProposal,
    [taker_fee_rate, annualized_settlement_fee_rate, rate_floor]
);
impl_typed_proposal!(
    MarketRiskProposal,
    [
        minimum_position_size_atoms,
        maximum_position_size_atoms,
        hard_gross_open_interest_cap_atoms,
        mark_observation_window_seconds,
        margin_time_threshold_seconds,
        order_rate_bound_factor,
        trade_mark_update_bound_factor
    ]
);

impl GlobalGovernanceConfig {
    /// Atomically replace a pending payload and its snapshotted execution
    /// window. Callers must supply the complete replacement value, ensuring
    /// stale fields from the previous proposal cannot survive.
    pub fn replace_typed_proposal_value<P: TypedProposal + Copy>(
        &self,
        pending: &mut P,
        replacement: P,
        category: GovernanceCategory,
        proposed_at: i64,
        changes: &[(i128, i128, ChangePolicy)],
    ) -> StateResult<()> {
        if !replacement.is_pending() {
            return Err(StateError::NotFound);
        }
        let window = self.proposal_window_for_values(category, proposed_at, changes)?;
        *pending = replacement;
        pending.set_window(window);
        Ok(())
    }
}
