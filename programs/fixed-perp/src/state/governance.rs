use anchor_lang::prelude::*;

use super::{
    AssetState, FixedPoint, MarginGroup, MarketConfig, MarketState, StateError, StateResult,
    MARKET_ACTIVE, MARKET_CLOSE_ONLY, MARKET_PAUSED,
};

pub const GOVERNANCE_CATEGORIES: usize = 8;

/// Authority addresses are normally Squads vault PDAs. Approval thresholds and
/// timelocks belong to Squads; this program applies authorized changes directly.
/// Numeric order is part of the account ABI.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "idl-build", derive(AnchorSerialize, AnchorDeserialize))]
#[cfg_attr(feature = "idl-build", borsh(use_discriminant = true))]
pub enum GovernanceCategory {
    /// Immediate: tighten market and group risk requirements only.
    RiskTightening = 0,
    /// Delayed: relax risk, tune observation policy, raise withdrawal limits,
    /// or reopen markets. Configure a timelock on this authority's Squad.
    RiskRecovery = 1,
    /// Delayed: fees and liquidation incentives, in either direction.
    FeesAndLiquidation = 2,
    /// Immediate: lower withdrawal limits and restrict market operation.
    Emergency = 3,
    /// Immediate: forced deleveraging, subject to position and health checks.
    Deleveraging = 4,
    /// Delayed: resource creation, activation, membership, and closure.
    Resources = 5,
    /// Delayed: collect eligible protocol fees to a selected destination.
    Treasury = 6,
    /// Delayed: rotate authority keys and change subaccount creation fees.
    AuthorityAdministration = 7,
}

impl GovernanceCategory {
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::RiskTightening => 0,
            Self::RiskRecovery => 1,
            Self::FeesAndLiquidation => 2,
            Self::Emergency => 3,
            Self::Deleveraging => 4,
            Self::Resources => 5,
            Self::Treasury => 6,
            Self::AuthorityAdministration => 7,
        }
    }

    pub fn index(self) -> usize {
        usize::from(self.as_u8())
    }
}

impl TryFrom<u8> for GovernanceCategory {
    type Error = StateError;
    fn try_from(value: u8) -> StateResult<Self> {
        match value {
            0 => Ok(Self::RiskTightening),
            1 => Ok(Self::RiskRecovery),
            2 => Ok(Self::FeesAndLiquidation),
            3 => Ok(Self::Emergency),
            4 => Ok(Self::Deleveraging),
            5 => Ok(Self::Resources),
            6 => Ok(Self::Treasury),
            7 => Ok(Self::AuthorityAdministration),
            _ => Err(StateError::InvalidGovernanceCategory),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GlobalGovernancePolicy {
    pub authorities: [Pubkey; GOVERNANCE_CATEGORIES],
    pub subaccount_creation_fee_lamports: u64,
    pub subaccount_creation_fee_destination: Pubkey,
}

#[account(zero_copy)]
#[repr(C)]
#[derive(Debug, Eq, PartialEq)]
pub struct GlobalGovernanceConfig {
    pub authorities: [Pubkey; GOVERNANCE_CATEGORIES],
    /// Next market ID to assign. Zero denotes an empty subaccount slot.
    /// Initialization consumes an ID atomically; closure never recycles it.
    pub next_market_id: u64,
    pub subaccount_creation_fee_lamports: u64,
    pub subaccount_creation_fee_destination: Pubkey,
    pub bump: u8,
    pub _pad: [u8; 7],
}

impl GlobalGovernanceConfig {
    pub const DATA_LEN: usize = core::mem::size_of::<Self>();
    pub const SPACE: usize = 8 + Self::DATA_LEN;

    pub fn initialize_with_policy(
        &mut self,
        bump: u8,
        policy: GlobalGovernancePolicy,
    ) -> StateResult<()> {
        let next = Self {
            authorities: policy.authorities,
            next_market_id: 1,
            subaccount_creation_fee_lamports: policy.subaccount_creation_fee_lamports,
            subaccount_creation_fee_destination: policy.subaccount_creation_fee_destination,
            bump,
            _pad: [0; 7],
        };

        next.validate()?;

        *self = next;

        Ok(())
    }

    pub fn validate(&self) -> StateResult<()> {
        if self.authorities.contains(&Pubkey::default()) {
            return Err(StateError::InvalidGovernanceAuthority);
        }
        if self.subaccount_creation_fee_destination == Pubkey::default() {
            return Err(StateError::InvalidCreationFeeDestination);
        }

        Ok(())
    }

    pub fn authority_for(&self, category: GovernanceCategory, authority: Pubkey) -> bool {
        authority != Pubkey::default() && self.authorities[category.index()] == authority
    }

    pub fn is_creation_fee_destination(&self, destination: Pubkey) -> bool {
        self.subaccount_creation_fee_destination == destination
    }
}

/// None leaves the live field unchanged. Authority rotation is limited to one
/// category per call; fee settings can be updated in the same atomic call.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, Default)]
pub struct UpdateAuthoritiesArgs {
    pub target_category: Option<u8>,
    pub new_authority: Option<Pubkey>,
    pub subaccount_creation_fee_lamports: Option<u64>,
    pub subaccount_creation_fee_destination: Option<Pubkey>,
}

impl UpdateAuthoritiesArgs {
    pub fn apply(&self, config: &mut GlobalGovernanceConfig) -> StateResult<()> {
        if self.target_category.is_some() != self.new_authority.is_some() {
            return Err(StateError::IncompleteAuthorityUpdate);
        }

        let mut next = *config;

        if let (Some(category), Some(authority)) = (self.target_category, self.new_authority) {
            next.authorities[GovernanceCategory::try_from(category)?.index()] = authority;
        }

        if let Some(value) = self.subaccount_creation_fee_lamports {
            next.subaccount_creation_fee_lamports = value;
        }

        if let Some(value) = self.subaccount_creation_fee_destination {
            next.subaccount_creation_fee_destination = value;
        }

        next.validate()?;

        *config = next;

        Ok(())
    }
}

/// Checks direction against live state at execution, not against a value seen
/// when a Squads transaction was proposed. Equality is an allowed no-op.
fn update_directional<T: Ord + Copy>(
    current: &mut T,
    target: Option<T>,
    increase: bool,
) -> StateResult<()> {
    if let Some(value) = target {
        if (increase && value < *current) || (!increase && value > *current) {
            return Err(StateError::RiskUpdateWrongDirection);
        }

        *current = value;
    }

    Ok(())
}

fn update_fixed(current: &mut FixedPoint, target: Option<i128>, increase: bool) -> StateResult<()> {
    let mut raw = current.to_raw_bits();

    update_directional(&mut raw, target, increase)?;

    *current = FixedPoint::from_raw_bits(raw);

    Ok(())
}

/// Market risk fields shared by tightening and recovery. Minimum position size
/// and mark observation window are recovery-only, in either direction.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, Default)]
pub struct UpdateMarketRiskArgs {
    pub initial_margin_factor_raw: Option<i128>,
    pub maintenance_margin_factor_raw: Option<i128>,
    pub closing_margin_relief_bound_factor_raw: Option<i128>,
    pub rate_floor_raw: Option<i128>,
    pub minimum_position_size_atoms: Option<u64>,
    pub maximum_position_size_atoms: Option<u64>,
    pub hard_gross_open_interest_cap_atoms: Option<u128>,
    pub mark_observation_window_seconds: Option<u32>,
    pub margin_time_threshold_seconds: Option<u32>,
    pub order_rate_bound_factor_raw: Option<i128>,
    pub trade_mark_update_bound_factor_raw: Option<i128>,
}

impl UpdateMarketRiskArgs {
    pub fn apply(&self, config: &mut MarketConfig, tightening: bool) -> StateResult<()> {
        let mut next = *config;

        update_fixed(
            &mut next.initial_margin_factor,
            self.initial_margin_factor_raw,
            tightening,
        )?;

        update_fixed(
            &mut next.maintenance_margin_factor,
            self.maintenance_margin_factor_raw,
            tightening,
        )?;

        update_fixed(
            &mut next.closing_margin_relief_bound_factor,
            self.closing_margin_relief_bound_factor_raw,
            !tightening,
        )?;

        update_fixed(&mut next.rate_floor, self.rate_floor_raw, tightening)?;

        if let Some(value) = self.minimum_position_size_atoms {
            if tightening {
                return Err(StateError::RecoveryAuthorityRequired);
            }
            next.minimum_position_size_atoms = value;
        }

        update_directional(
            &mut next.maximum_position_size_atoms,
            self.maximum_position_size_atoms,
            !tightening,
        )?;

        update_directional(
            &mut next.hard_gross_open_interest_cap_atoms,
            self.hard_gross_open_interest_cap_atoms,
            !tightening,
        )?;

        if let Some(value) = self.mark_observation_window_seconds {
            if tightening {
                return Err(StateError::RecoveryAuthorityRequired);
            }

            next.mark_observation_window_seconds = value;
        }

        update_directional(
            &mut next.margin_time_threshold_seconds,
            self.margin_time_threshold_seconds,
            tightening,
        )?;

        update_fixed(
            &mut next.order_rate_bound_factor,
            self.order_rate_bound_factor_raw,
            !tightening,
        )?;

        update_fixed(
            &mut next.trade_mark_update_bound_factor,
            self.trade_mark_update_bound_factor_raw,
            !tightening,
        )?;

        next.validate()?;

        if next.rate_floor != config.rate_floor {
            next.pm_cache_nonce = next
                .pm_cache_nonce
                .checked_add(1)
                .ok_or(StateError::ArithmeticOverflow)?;
        }

        *config = next;

        Ok(())
    }
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, Default)]
pub struct UpdateGroupRiskArgs {
    pub critical_health_ratio_raw: Option<i128>,
    pub risky_order_cancellation_ratio_raw: Option<i128>,
}

impl UpdateGroupRiskArgs {
    pub fn apply(&self, group: &mut MarginGroup, tightening: bool) -> StateResult<()> {
        let mut critical = group.critical_health_ratio;
        let mut cancellation = group.risky_order_cancellation_ratio;

        update_fixed(&mut critical, self.critical_health_ratio_raw, tightening)?;

        update_fixed(
            &mut cancellation,
            self.risky_order_cancellation_ratio_raw,
            tightening,
        )?;

        if critical.to_raw_bits() <= 0 || cancellation.to_raw_bits() < critical.to_raw_bits() {
            return Err(StateError::InvalidGroupHealthRatios);
        }

        group.critical_health_ratio = critical;
        group.risky_order_cancellation_ratio = cancellation;

        Ok(())
    }
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, Default)]
pub struct UpdateFeesAndLiquidationArgs {
    pub taker_fee_rate_raw: Option<i128>,
    pub annualized_settlement_fee_rate_raw: Option<i128>,
    pub liquidation_base_incentive_raw: Option<i128>,
    pub liquidation_incentive_slope_raw: Option<i128>,
    pub liquidation_fee_rate_raw: Option<i128>,
}

impl UpdateFeesAndLiquidationArgs {
    pub fn apply(&self, config: &mut MarketConfig) -> StateResult<()> {
        let mut next = *config;

        if let Some(value) = self.taker_fee_rate_raw {
            next.taker_fee_rate = FixedPoint::from_raw_bits(value);
        }

        if let Some(value) = self.annualized_settlement_fee_rate_raw {
            next.annualized_settlement_fee_rate = FixedPoint::from_raw_bits(value);
        }

        if let Some(value) = self.liquidation_base_incentive_raw {
            next.liquidation_base_incentive = FixedPoint::from_raw_bits(value);
        }

        if let Some(value) = self.liquidation_incentive_slope_raw {
            next.liquidation_incentive_slope = FixedPoint::from_raw_bits(value);
        }

        if let Some(value) = self.liquidation_fee_rate_raw {
            next.liquidation_fee_rate = FixedPoint::from_raw_bits(value);
        }

        next.validate()?;

        *config = next;

        Ok(())
    }
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, Default)]
pub struct UpdateWithdrawalLimitsArgs {
    pub short_window_limit_atoms: Option<u128>,
    pub medium_window_limit_atoms: Option<u128>,
    pub long_window_limit_atoms: Option<u128>,
}

impl UpdateWithdrawalLimitsArgs {
    pub fn apply(&self, asset: &mut AssetState, tightening: bool) -> StateResult<()> {
        let mut short = asset.short_window.limit_atoms;
        let mut medium = asset.medium_window.limit_atoms;
        let mut long = asset.long_window.limit_atoms;

        update_directional(&mut short, self.short_window_limit_atoms, !tightening)?;
        update_directional(&mut medium, self.medium_window_limit_atoms, !tightening)?;
        update_directional(&mut long, self.long_window_limit_atoms, !tightening)?;

        // Preserve the withdrawal counters; a limit update must not reset usage.
        asset.set_limits(short, medium, long);

        Ok(())
    }
}

/// Emergency only restricts operation; reopening always requires recovery.
/// Pending activation and maturity finalization use their dedicated paths.
pub fn update_market_status(
    market: &mut MarketState,
    status: u8,
    emergency: bool,
) -> StateResult<()> {
    if !matches!(
        market.status.0,
        MARKET_ACTIVE | MARKET_CLOSE_ONLY | MARKET_PAUSED
    ) {
        return Err(StateError::InvalidStatus);
    }

    let allowed = if emergency {
        matches!(
            (market.status.0, status),
            (MARKET_ACTIVE, MARKET_CLOSE_ONLY | MARKET_PAUSED) | (MARKET_CLOSE_ONLY, MARKET_PAUSED)
        )
    } else {
        matches!(
            (market.status.0, status),
            (MARKET_PAUSED, MARKET_ACTIVE | MARKET_CLOSE_ONLY) | (MARKET_CLOSE_ONLY, MARKET_ACTIVE)
        )
    };

    if !allowed {
        return Err(StateError::InvalidMarketStatusTransition);
    }

    market.status.0 = status;

    Ok(())
}
