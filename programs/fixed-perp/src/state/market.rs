use anchor_lang::prelude::*;
use bytemuck::{Pod, Zeroable};
use fixed::types::I64F64;
use fixed_perp_math::{rate::RateDelta, Rate};

use super::user::OrderRiskScale;
use super::{
    AssetState, FixedPoint, OracleObservation, OrderBook, OrderId, OrderRef, PriceEndpoint,
    SettlementRatio, Side, StateError, StateResult, Subaccount, TimeInForce, ValuationFactors,
    BOOK_CAPACITY,
};

pub use fixed_perp_math::rate::SECONDS_PER_YEAR;

fn decimal_scale(decimals: u8) -> StateResult<I64F64> {
    let mut scale = I64F64::ONE;

    for _ in 0..decimals {
        scale = scale
            .checked_mul(I64F64::from_num(10))
            .ok_or(StateError::ArithmeticOverflow)?;
    }
    Ok(scale)
}

/// Signed division that rounds away from zero when the remainder is nonzero
/// and the operands have the same sign. This is Boros `rawDivCeil`.
fn ceil_div(numerator: i128, denominator: i128) -> Option<i128> {
    if denominator == 0 {
        return None;
    }

    let quotient = numerator.checked_div(denominator)?;
    let remainder = numerator.checked_rem(denominator)?;

    if remainder != 0 && numerator.signum() == denominator.signum() {
        quotient.checked_add(1)
    } else {
        Some(quotient)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlaceOrderIntent {
    pub size: i64,
    pub limit_tick: i16,
    pub time_in_force: TimeInForce,
    pub expiry_timestamp: i64,
    pub maximum_payment_atoms: u128,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PlaceOrderResult {
    pub filled_size: i64,
    pub remaining_size: i64,
    pub fixed_payment_atoms: i128,
    pub taker_fee_atoms: u128,
    pub resting_order: Option<OrderId>,
}

/// One market plus the transient authenticated observation for its underlying
/// AssetState.  The observation is caller-owned and never persisted.
#[derive(Clone, Copy)]
pub struct PortfolioMarketObservation<'a> {
    pub market: &'a MarketState,
    pub underlying_asset: &'a AssetState,
    pub underlying: OracleObservation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BatchOrder {
    pub size: u64,
    pub limit_tick: i16,
    pub expiry_timestamp: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlaceOrderBatchIntent<'a> {
    pub side: Side,
    pub time_in_force: TimeInForce,
    pub maximum_payment_atoms: u128,
    pub orders: &'a [BatchOrder],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ModifyOrderIntent {
    pub order_id: OrderId,
    pub new_size: i64,
}

pub struct MakerSubaccount<'a> {
    pub key: Pubkey,
    pub subaccount: &'a Subaccount,
}

#[derive(Default)]
struct MatchTotals {
    filled_size: u64,
    fixed_payment_atoms: i128,
    taker_fee_atoms: u128,
    last_matched_tick: Option<i16>,
}

struct RestingOrder {
    size: i64,
    tick: i16,
    time_in_force: TimeInForce,
    expiry_timestamp: i64,
}

pub const MARGIN_GROUP_RESERVED: usize = 24;
pub const JUPITER_PAYLOAD_RESERVED: usize = 16;
pub const PHOENIX_PAYLOAD_RESERVED: usize = 8;
pub const VENUE_CONFIG_RESERVED: usize = 16;
pub const VENUE_ADAPTER_PAYLOAD_WORDS: usize = core::mem::size_of::<PhoenixAdapterPayload>() / 16;
pub const MARKET_CONFIG_RESERVED: usize = 32;
pub const MARKET_STATE_RESERVED: usize = 24;

pub const ADAPTER_JUPITER: u8 = 1;
pub const ADAPTER_PHOENIX: u8 = 2;
pub const GROUP_PENDING: u8 = 0;
pub const GROUP_ACTIVE: u8 = 1;
pub const MARKET_PENDING: u8 = 0;
pub const MARKET_ACTIVE: u8 = 1;
pub const MARKET_CLOSE_ONLY: u8 = 2;
pub const MARKET_PAUSED: u8 = 3;
pub const MARKET_MATURED: u8 = 4;

macro_rules! tag_type {
    ($name:ident) => {
        #[repr(transparent)]
        #[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Pod, Zeroable)]
        #[cfg_attr(feature = "idl-build", derive(AnchorSerialize, AnchorDeserialize))]
        pub struct $name(pub u8);
    };
}

tag_type!(MarketStatus);
tag_type!(VenueAdapterKind);
tag_type!(VenueLayout);

#[account(zero_copy)]
#[repr(C)]
#[derive(Debug, Eq, PartialEq)]
pub struct MarginGroup {
    pub group_id: u16,
    pub bump: u8,
    pub status: u8,
    pub _pad_0: [u8; 4],
    pub collateral_state: Pubkey,
    pub settlement_state: Pubkey,
    pub governance_key: Pubkey,
    pub critical_health_ratio: FixedPoint,
    pub risky_order_cancellation_ratio: FixedPoint,
    pub market_count: u32,
    pub subaccount_count: u32,
    /// Group-level health haircuts.
    pub collateral_factors: ValuationFactors,
    pub settlement_factors: ValuationFactors,
    pub reserved: [u8; MARGIN_GROUP_RESERVED],
}

impl MarginGroup {
    pub fn is_pending(&self) -> bool {
        self.status == GROUP_PENDING
    }

    pub fn is_active(&self) -> bool {
        self.status == GROUP_ACTIVE
    }

    pub fn is_same_collateral_state(&self, state: Pubkey) -> bool {
        self.collateral_state == state
    }

    pub fn is_governance_key(&self, key: Pubkey) -> bool {
        self.governance_key == key
    }

    /// Records one newly created subaccount after the instruction validates its group binding.
    pub fn add_subaccount(&mut self) -> StateResult<()> {
        self.subaccount_count = self
            .subaccount_count
            .checked_add(1)
            .ok_or(StateError::ArithmeticOverflow)?;

        Ok(())
    }

    /// Removes one closed empty subaccount from this group's live count.
    pub fn remove_subaccount(&mut self) -> StateResult<()> {
        self.subaccount_count = self
            .subaccount_count
            .checked_sub(1)
            .ok_or(StateError::Conflict)?;

        Ok(())
    }

    /// Records one membership account created for this group.
    pub fn add_market(&mut self) -> StateResult<()> {
        self.market_count = self
            .market_count
            .checked_add(1)
            .ok_or(StateError::ArithmeticOverflow)?;

        Ok(())
    }

    pub const DATA_LEN: usize = core::mem::size_of::<Self>();
    pub const SPACE: usize = 8 + Self::DATA_LEN;
}

/// Discriminator-only proof that a market was approved into a margin group.
///
/// Existence of this account proves membership; it carries no other state.
/// Membership is derived from the group and market keys by account validation
/// outside this state module, which never derives addresses itself.
#[account]
#[derive(Debug, Default)]
pub struct MarketGroupMembership {}

/// Rent-bearing delayed proposal for one group/market membership. The active
/// membership remains the discriminator-only PDA above.
#[account]
#[derive(Debug, InitSpace)]
pub struct PendingMarketGroupMembership {
    pub margin_group: Pubkey,
    pub market: Pubkey,
    pub rent_recipient: Pubkey,
    pub window: super::ProposalWindow,
}

impl PendingMarketGroupMembership {
    pub const SPACE: usize = 8 + Self::INIT_SPACE;

    /// Initializes a delayed market/group membership proposal.
    pub fn initialize_in_place(
        &mut self,
        margin_group: Pubkey,
        market: Pubkey,
        rent_recipient: Pubkey,
        window: super::ProposalWindow,
    ) -> StateResult<()> {
        if margin_group == Pubkey::default()
            || market == Pubkey::default()
            || rent_recipient == Pubkey::default()
        {
            return Err(StateError::InvalidConfiguration);
        }

        self.margin_group = margin_group;
        self.market = market;
        self.rent_recipient = rent_recipient;
        self.window = window;

        Ok(())
    }

    pub fn is_same_margin_group(&self, key: Pubkey) -> bool {
        self.margin_group == key
    }

    pub fn is_same_market(&self, key: Pubkey) -> bool {
        self.market == key
    }

    pub fn is_same_rent_recipient(&self, key: Pubkey) -> bool {
        self.rent_recipient == key
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Pod, Zeroable)]
#[cfg_attr(feature = "idl-build", derive(AnchorSerialize, AnchorDeserialize))]
pub struct JupiterAdapterPayload {
    pub max_source_delta: u128,
    pub custody: Pubkey,
    pub pool: Pubkey,
    pub mint: Pubkey,
    pub max_source_age_seconds: u64,
    pub layout: VenueLayout,
    pub _pad_0: [u8; 7],
    pub reserved: [u8; JUPITER_PAYLOAD_RESERVED],
    pub _pad_1: [u8; 16],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Pod, Zeroable)]
#[cfg_attr(feature = "idl-build", derive(AnchorSerialize, AnchorDeserialize))]
pub struct PhoenixAdapterPayload {
    pub max_abs_native_delta: u128,
    pub global_configuration: Pubkey,
    pub perp_asset_map: Pubkey,
    pub canonical_quote_mint: Pubkey,
    pub market_account: Pubkey,
    pub tick_size: u64,
    pub funding_interval_seconds: u64,
    pub funding_period_seconds: u64,
    pub max_funding_age_seconds: u64,
    pub max_mark_age_slots: u64,
    pub symbol: [u8; 16],
    pub asset_slot: u16,
    pub asset_id: u16,
    pub base_lot_decimals: u8,
    pub layout: VenueLayout,
    pub _pad_0: [u8; 10],
    pub reserved: [u8; PHOENIX_PAYLOAD_RESERVED],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Pod, Zeroable)]
#[cfg_attr(feature = "idl-build", derive(AnchorSerialize, AnchorDeserialize))]
pub struct VenueAdapterConfig {
    pub kind: VenueAdapterKind,
    pub _pad_0: [u8; 15],
    // One fixed-capacity tagged payload sized for the larger active adapter.
    pub payload: [u128; VENUE_ADAPTER_PAYLOAD_WORDS],
    pub reserved: [u8; VENUE_CONFIG_RESERVED],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Pod, Zeroable)]
#[cfg_attr(feature = "idl-build", derive(AnchorSerialize, AnchorDeserialize))]
pub struct MarketConfig {
    /// Maximum gross open interest in underlying atoms.
    pub hard_gross_open_interest_cap_atoms: u128,
    pub start_timestamp: i64,
    pub maturity_timestamp: i64,
    /// Minimum and maximum position sizes in underlying atoms.
    pub minimum_position_size_atoms: u64,
    pub maximum_position_size_atoms: u64,
    pub collateral_state: Pubkey,
    pub settlement_state: Pubkey,
    pub underlying_state: Pubkey,
    pub _pad_0: [u8; 16],
    pub adapter: VenueAdapterConfig,
    pub rate_floor: FixedPoint,
    pub initial_margin_factor: FixedPoint,
    pub maintenance_margin_factor: FixedPoint,
    pub order_rate_bound_factor: FixedPoint,
    pub closing_margin_relief_bound_factor: FixedPoint,
    pub trade_mark_update_bound_factor: FixedPoint,
    pub taker_fee_rate: FixedPoint,
    pub annualized_settlement_fee_rate: FixedPoint,
    pub liquidation_base_incentive: FixedPoint,
    pub liquidation_incentive_slope: FixedPoint,
    pub liquidation_fee_rate: FixedPoint,
    pub market_id: u32,
    pub mark_observation_window_seconds: u32,
    pub margin_time_threshold_seconds: u32,
    pub pm_cache_nonce: u32,
    pub _pad_1: [u8; 4],
    /// Implicit decimal exponent for all market rate/index mantissas.
    pub tick_scale: u8,
    /// AssetState decimal metadata for position/order/OI underlying atoms.
    pub underlying_decimals: u8,
    /// AssetState decimal metadata for settlement balances and payments.
    pub settlement_decimals: u8,
    pub _pad_units: u8,
    pub reserved: [u8; MARKET_CONFIG_RESERVED],
    pub _pad_2: [u8; 12],
    pub _pad_3: [u8; 12],
}

#[account(zero_copy(unsafe))]
#[repr(C)]
#[derive(Debug, Eq, PartialEq)]
pub struct MarketState {
    pub config: MarketConfig,
    pub order_book: OrderBook,
    pub floating_index: FixedPoint,
    pub fee_index: FixedPoint,
    pub source_native_checkpoint: u128,
    pub source_last_update_timestamp: i64,
    pub protocol_checkpoint_timestamp: i64,
    pub epoch_origin_timestamp: i64,
    pub checkpoint_epoch: u64,
    pub previous_mark_rate: FixedPoint,
    pub target_mark_rate: FixedPoint,
    pub last_eligible_trade_timestamp: i64,
    pub materialized_gross_open_interest: u128,
    pub pending_maker_oi_reserve: u128,
    pub accrued_protocol_fee_atoms: u128,
    pub position_factors: ValuationFactors,
    pub bump: u8,
    pub status: MarketStatus,
    pub source_seeded: u8,
    pub _pad_0: [u8; 1],
    pub subaccount_count: u32,
    /// Authenticated collateral-per-settlement ratio frozen at maturity.
    pub frozen_settlement_ratio: SettlementRatio,
    /// Timestamp at which the settlement ratio was frozen.
    pub finalized_at_timestamp: i64,
    pub reserved: [u8; MARKET_STATE_RESERVED],
    /// Resource-local governance account bound at market creation.
    pub governance_key: Pubkey,
}

impl MarketState {
    pub fn is_governance_key(&self, key: Pubkey) -> bool {
        self.governance_key == key
    }

    /// Reads the authenticated collateral-per-settlement ratio frozen at
    /// maturity.
    pub fn frozen_settlement_ratio(&self) -> Option<SettlementRatio> {
        let ratio = self.frozen_settlement_ratio;

        (ratio.numerator != 0 && ratio.denominator != 0).then_some(ratio)
    }

    pub fn finalized_at(&self) -> Option<i64> {
        (self.finalized_at_timestamp > 0).then_some(self.finalized_at_timestamp)
    }

    pub fn freeze_settlement_ratio(
        &mut self,
        ratio: SettlementRatio,
        finalized_at: i64,
    ) -> StateResult<()> {
        if ratio.numerator == 0
            || ratio.denominator == 0
            || !matches!(self.status.0, MARKET_ACTIVE | MARKET_CLOSE_ONLY)
            || self.source_seeded != 1
            || finalized_at < self.config.maturity_timestamp
        {
            return Err(StateError::InvalidStatus);
        }

        self.frozen_settlement_ratio = ratio;
        self.finalized_at_timestamp = finalized_at;
        self.status = MarketStatus(MARKET_MATURED);

        Ok(())
    }

    /// Records a subaccount entering this market for the first time.
    pub(crate) fn add_subaccount(&mut self) -> StateResult<()> {
        self.subaccount_count = self
            .subaccount_count
            .checked_add(1)
            .ok_or(StateError::ArithmeticOverflow)?;

        Ok(())
    }

    /// Records an occupied subaccount slot being released from this market.
    pub(crate) fn remove_subaccount(&mut self) -> StateResult<()> {
        self.subaccount_count = self
            .subaccount_count
            .checked_sub(1)
            .ok_or(StateError::Conflict)?;

        Ok(())
    }

    /// Returns whether the market is still awaiting activation.
    pub fn is_pending(&self) -> bool {
        self.status.0 == MARKET_PENDING
    }

    /// Returns whether the market permits unrestricted order entry and matching.
    pub fn is_active(&self) -> bool {
        self.status.0 == MARKET_ACTIVE
    }

    /// Returns whether the market permits only exposure-reducing orders and fills.
    pub fn is_close_only(&self) -> bool {
        self.status.0 == MARKET_CLOSE_ONLY
    }

    /// Returns whether maturity finalization has completed for this market.
    pub fn is_matured(&self) -> bool {
        self.status.0 == MARKET_MATURED
    }

    /// Collateral atoms the protocol may remove for this market's booked fees.
    ///
    /// Every slot must already be closed, so its settlement has been folded
    /// into collateral claims and this fee was not credited to a user. A
    /// matured market converts the fee at the same frozen ratio, floored.
    /// The same asset is already denominated in collateral atoms.
    pub fn accrued_fee_collateral_atoms(&self) -> StateResult<u128> {
        if !self.is_matured() || self.subaccount_count != 0 {
            return Err(StateError::InvalidStatus);
        }

        if self.materialized_gross_open_interest != 0 || self.pending_maker_oi_reserve != 0 {
            return Err(StateError::Conflict);
        }

        let fees = self.accrued_protocol_fee_atoms;

        if fees == 0 {
            return Ok(0);
        }

        if self.config.collateral_state == self.config.settlement_state {
            return Ok(fees);
        }

        let fees = i128::try_from(fees).map_err(|_| StateError::ArithmeticOverflow)?;

        let collateral = self
            .frozen_settlement_ratio()
            .ok_or(StateError::InvalidStatus)?
            .apply(fees)?;

        u128::try_from(collateral).map_err(|_| StateError::ArithmeticOverflow)
    }

    /// Zeros booked fees on the supplied matured markets and returns the
    /// collateral atoms those fees entitle the protocol to withdraw.
    /// Nothing is changed if any market is ineligible or the total is zero.
    pub fn take_matured_protocol_fees(
        asset: Pubkey,
        markets: &mut [&mut Self],
    ) -> StateResult<u64> {
        if markets.is_empty() {
            return Err(StateError::InvalidConfiguration);
        }

        let mut total = 0u128;

        for (index, market) in markets.iter().enumerate() {
            if market.config.collateral_state != asset {
                return Err(StateError::InvalidKey);
            }

            let market_id = market.config.market_id;

            if markets[..index]
                .iter()
                .any(|earlier| earlier.config.market_id == market_id)
            {
                return Err(StateError::InvalidRemainingAccounts);
            }

            total = total
                .checked_add(market.accrued_fee_collateral_atoms()?)
                .ok_or(StateError::ArithmeticOverflow)?;
        }

        if total == 0 {
            return Err(StateError::InvalidConfiguration);
        }

        let amount = u64::try_from(total).map_err(|_| StateError::ArithmeticOverflow)?;

        for market in markets.iter_mut() {
            market.accrued_protocol_fee_atoms = 0;
        }

        Ok(amount)
    }

    /// Transfers `size_to_liquidator` of the victim's position to the liquidator.
    ///
    /// This is the Boros liquidation swap: the victim is reduce-only, the
    /// liquidator does not need a prior position, the incentive is
    /// `min(k, health_ratio)` times the maintenance-margin drop, and the
    /// liquidation fee is charged only to the liquidator. The liquidator pays
    /// or receives the fixed cash flow of the size they take; the victim
    /// receives the opposite flow and pays the incentive. `health_ratio` must
    /// already be the victim's portfolio ratio in `[0, 1)`.
    pub fn liquidate(
        &mut self,
        victim: &mut Subaccount,
        liquidator: &mut Subaccount,
        size_to_liquidator: i64,
        health_ratio: FixedPoint,
        now: i64,
    ) -> StateResult<(i128, u128, i128)> {
        if !self.is_active() && !self.is_close_only() {
            return Err(StateError::InvalidStatus);
        }
        let ratio = health_ratio.to_i64f64();
        if ratio < I64F64::ZERO || ratio >= I64F64::ONE {
            return Err(StateError::InvalidStatus);
        }
        let time_to_maturity = self
            .config
            .maturity_timestamp
            .checked_sub(now)
            .ok_or(StateError::InvalidClock)?;
        if time_to_maturity <= 0 {
            return Err(StateError::InvalidStatus);
        }

        let victim_slot = victim
            .lookup_market(self.config.market_id)
            .ok_or(StateError::NotFound)?;
        let old_victim = victim.markets[victim_slot].position_size;
        if size_to_liquidator == 0
            || old_victim == 0
            || size_to_liquidator.signum() != old_victim.signum()
            || size_to_liquidator.unsigned_abs() > old_victim.unsigned_abs()
        {
            return Err(StateError::Conflict);
        }
        let new_victim = old_victim
            .checked_sub(size_to_liquidator)
            .ok_or(StateError::ArithmeticOverflow)?;
        let old_liquidator = liquidator
            .lookup_market(self.config.market_id)
            .map(|slot| liquidator.markets[slot].position_size)
            .unwrap_or(0);
        let new_liquidator = old_liquidator
            .checked_add(size_to_liquidator)
            .ok_or(StateError::ArithmeticOverflow)?;

        let mark = self.current_mark_rate(now)?;
        let no_orders = super::OrderRiskAggregate::zeroed();
        let margin_before = self.maintenance_margin(
            self.risk_principal(old_victim, &no_orders, mark)?,
            time_to_maturity,
        )?;
        let margin_after = self.maintenance_margin(
            self.risk_principal(new_victim, &no_orders, mark)?,
            time_to_maturity,
        )?;
        // Boros subtracts the two maintenance margins as uints, so an increase
        // reverts rather than paying a zero incentive.
        let margin_drop = margin_before
            .checked_sub(margin_after)
            .ok_or(StateError::ArithmeticOverflow)?;
        if margin_drop < 0 {
            return Err(StateError::ArithmeticOverflow);
        }
        let complement = I64F64::ONE
            .checked_sub(ratio)
            .ok_or(StateError::ArithmeticOverflow)?;
        let k = self
            .config
            .liquidation_base_incentive
            .to_i64f64()
            .checked_add(
                self.config
                    .liquidation_incentive_slope
                    .to_i64f64()
                    .checked_mul(complement)
                    .ok_or(StateError::ArithmeticOverflow)?,
            )
            .ok_or(StateError::ArithmeticOverflow)?;
        let factor = if k < ratio { k } else { ratio };
        if factor < I64F64::ZERO {
            return Err(StateError::InvalidConfiguration);
        }
        let incentive = FixedPoint::from_i64f64(factor)
            .checked_mul_int(margin_drop)
            .map_err(|_| StateError::ArithmeticOverflow)?
            .floor_atoms()
            .map_err(|_| StateError::ArithmeticOverflow)?;
        // This is the cash flow of the account that receives
        // `size_to_liquidator`. A long pays fixed, so the value is negative
        // for a positive size at a positive mark. The victim takes the
        // opposite flow and pays the incentive; the fee stays with the
        // liquidator. The two fixed legs are exact negations, so user cash
        // plus the booked fee is zero.
        let fixed_cash = Rate::from_i64f64(mark.to_i64f64())
            .fixed_payment_atoms_scaled(
                i128::from(size_to_liquidator),
                time_to_maturity,
                SECONDS_PER_YEAR,
                self.config.underlying_decimals,
                self.config.settlement_decimals,
                self.config.tick_scale,
            )
            .map_err(|_| StateError::ArithmeticOverflow)?;
        let fee =
            self.liquidation_fee_atoms(size_to_liquidator.unsigned_abs(), time_to_maturity)?;
        let fee_i128 = i128::try_from(fee).map_err(|_| StateError::ArithmeticOverflow)?;
        let victim_cash = fixed_cash
            .checked_neg()
            .and_then(|value| value.checked_sub(incentive))
            .ok_or(StateError::ArithmeticOverflow)?;
        let liquidator_cash = fixed_cash
            .checked_add(incentive)
            .and_then(|value| value.checked_sub(fee_i128))
            .ok_or(StateError::ArithmeticOverflow)?;
        let next_open_interest = self
            .materialized_gross_open_interest
            .checked_sub(u128::from(old_victim.unsigned_abs()))
            .and_then(|value| value.checked_add(u128::from(new_victim.unsigned_abs())))
            .and_then(|value| value.checked_sub(u128::from(old_liquidator.unsigned_abs())))
            .and_then(|value| value.checked_add(u128::from(new_liquidator.unsigned_abs())))
            .ok_or(StateError::Conflict)?;
        let next_fees = self
            .accrued_protocol_fee_atoms
            .checked_add(fee)
            .ok_or(StateError::ArithmeticOverflow)?;
        let next_victim_settlement = victim.markets[victim_slot]
            .settlement_balance_atoms
            .checked_add(victim_cash)
            .ok_or(StateError::ArithmeticOverflow)?;
        let next_liquidator_settlement = match liquidator.lookup_market(self.config.market_id) {
            Some(slot) => liquidator.markets[slot]
                .settlement_balance_atoms
                .checked_add(liquidator_cash)
                .ok_or(StateError::ArithmeticOverflow)?,
            None => {
                if !liquidator.markets.iter().any(|slot| slot.market_id == 0) {
                    return Err(StateError::CapacityExhausted);
                }
                self.subaccount_count
                    .checked_add(1)
                    .ok_or(StateError::ArithmeticOverflow)?;
                liquidator_cash
            }
        };

        // Orders are cancelled only once the transfer is known to fit. A
        // rejected size, a full liquidator, or a balance that cannot take the
        // cash leaves the book and both subaccounts unchanged.
        self.close_all_orders(victim, victim_slot)?;
        let liquidator_slot = liquidator.ensure_market(self)?;
        victim.markets[victim_slot].position_size = new_victim;
        victim.markets[victim_slot].settlement_balance_atoms = next_victim_settlement;
        liquidator.markets[liquidator_slot].position_size = new_liquidator;
        liquidator.markets[liquidator_slot].settlement_balance_atoms = next_liquidator_settlement;
        self.materialized_gross_open_interest = next_open_interest;
        self.accrued_protocol_fee_atoms = next_fees;
        Ok((incentive, fee, liquidator_cash))
    }

    /// Reduces two opposite positions. `size_to_winner` is the winner's trade.
    ///
    /// This is Boros `forceDeleverage`: pause does not block it, maturity does,
    /// both trades are reduce-only, only the loser's orders are cancelled, and
    /// there is no fee. `loser_value` is the loser's portfolio value in
    /// settlement atoms. A negative value shifts `alpha` of that shortfall from
    /// the winner to the loser; a nonnegative value trades at the mark. `alpha`
    /// is a fraction in `[0, 1]`.
    pub fn deleverage(
        &mut self,
        winner: &mut Subaccount,
        loser: &mut Subaccount,
        size_to_winner: i64,
        loser_value: i128,
        alpha: FixedPoint,
        now: i64,
    ) -> StateResult<i128> {
        let status = self.status.0;
        if status != MARKET_ACTIVE && status != MARKET_CLOSE_ONLY && status != MARKET_PAUSED {
            return Err(StateError::InvalidStatus);
        }
        let time_to_maturity = self
            .config
            .maturity_timestamp
            .checked_sub(now)
            .ok_or(StateError::InvalidClock)?;
        if time_to_maturity <= 0 {
            return Err(StateError::InvalidStatus);
        }
        let alpha_bits = alpha.to_raw_bits();
        if alpha_bits < 0 || alpha_bits > (1i128 << 64) {
            return Err(StateError::InvalidConfiguration);
        }

        let winner_slot = winner
            .lookup_market(self.config.market_id)
            .ok_or(StateError::NotFound)?;
        let loser_slot = loser
            .lookup_market(self.config.market_id)
            .ok_or(StateError::NotFound)?;
        let old_winner = winner.markets[winner_slot].position_size;
        let old_loser = loser.markets[loser_slot].position_size;
        let loser_trade = size_to_winner
            .checked_neg()
            .ok_or(StateError::ArithmeticOverflow)?;
        if !Self::trade_reduces(old_winner, size_to_winner)
            || !Self::trade_reduces(old_loser, loser_trade)
        {
            return Err(StateError::Conflict);
        }
        let new_winner = old_winner
            .checked_add(size_to_winner)
            .ok_or(StateError::ArithmeticOverflow)?;
        let new_loser = old_loser
            .checked_add(loser_trade)
            .ok_or(StateError::ArithmeticOverflow)?;

        let mark = self.current_mark_rate(now)?;
        let fixed_cash = Rate::from_i64f64(mark.to_i64f64())
            .fixed_payment_atoms_scaled(
                i128::from(size_to_winner),
                time_to_maturity,
                SECONDS_PER_YEAR,
                self.config.underlying_decimals,
                self.config.settlement_decimals,
                self.config.tick_scale,
            )
            .map_err(|_| StateError::ArithmeticOverflow)?;
        let loss = Self::attributed_loss(loser_value, alpha, size_to_winner, old_loser)?;
        let winner_cash = fixed_cash
            .checked_add(loss)
            .ok_or(StateError::ArithmeticOverflow)?;
        let loser_cash = winner_cash
            .checked_neg()
            .ok_or(StateError::ArithmeticOverflow)?;
        let next_winner_settlement = winner.markets[winner_slot]
            .settlement_balance_atoms
            .checked_add(winner_cash)
            .ok_or(StateError::ArithmeticOverflow)?;
        let next_loser_settlement = loser.markets[loser_slot]
            .settlement_balance_atoms
            .checked_add(loser_cash)
            .ok_or(StateError::ArithmeticOverflow)?;
        let next_open_interest = self
            .materialized_gross_open_interest
            .checked_sub(u128::from(old_winner.unsigned_abs()))
            .and_then(|value| value.checked_add(u128::from(new_winner.unsigned_abs())))
            .and_then(|value| value.checked_sub(u128::from(old_loser.unsigned_abs())))
            .and_then(|value| value.checked_add(u128::from(new_loser.unsigned_abs())))
            .ok_or(StateError::Conflict)?;

        // The loser's book is cancelled only after the transfer is known to fit.
        self.close_all_orders(loser, loser_slot)?;
        winner.markets[winner_slot].position_size = new_winner;
        winner.markets[winner_slot].settlement_balance_atoms = next_winner_settlement;
        loser.markets[loser_slot].position_size = new_loser;
        loser.markets[loser_slot].settlement_balance_atoms = next_loser_settlement;
        self.materialized_gross_open_interest = next_open_interest;
        Ok(loss)
    }

    /// `true` when `trade` is nonzero, no larger than `position`, and not the
    /// same sign. This is Boros `_isReducedOnly`.
    fn trade_reduces(position: i64, trade: i64) -> bool {
        trade != 0
            && trade.unsigned_abs() <= position.unsigned_abs()
            && trade.signum() * position.signum() <= 0
    }

    /// Settlement-atom shortfall moved onto the winner. Zero while the loser
    /// still has value. Otherwise `floor(loser_value * ceil(alpha * size / position))`,
    /// which is the cash form of Boros' annualized bankruptcy rate.
    fn attributed_loss(
        loser_value: i128,
        alpha: FixedPoint,
        size_to_winner: i64,
        loser_position: i64,
    ) -> StateResult<i128> {
        if loser_value >= 0 {
            return Ok(0);
        }
        let numerator = alpha
            .to_raw_bits()
            .checked_mul(i128::from(size_to_winner))
            .ok_or(StateError::ArithmeticOverflow)?;
        let factor_bits = ceil_div(numerator, i128::from(loser_position))
            .filter(|factor| *factor >= 0)
            .ok_or(StateError::ArithmeticOverflow)?;
        FixedPoint::from_raw_bits(factor_bits)
            .checked_mul_int(loser_value)
            .map_err(|_| StateError::ArithmeticOverflow)?
            .floor_atoms()
            .map_err(|_| StateError::ArithmeticOverflow)
    }

    fn liquidation_fee_atoms(&self, size: u64, time_to_maturity: i64) -> StateResult<u128> {
        let rate = I64F64::from_bits(self.config.liquidation_fee_rate.to_raw_bits());
        let elapsed =
            I64F64::checked_from_num(time_to_maturity).ok_or(StateError::ArithmeticOverflow)?;
        let year =
            I64F64::checked_from_num(SECONDS_PER_YEAR).ok_or(StateError::ArithmeticOverflow)?;
        let market_scale = decimal_scale(self.config.tick_scale)?;
        let settlement_scale = decimal_scale(self.config.settlement_decimals)?;
        let maturity_scaled_rate = rate
            .checked_mul(elapsed)
            .and_then(|value| value.checked_mul(market_scale))
            .and_then(|value| value.checked_mul(settlement_scale))
            .and_then(|value| value.checked_div(year))
            .ok_or(StateError::ArithmeticOverflow)?;
        fixed_perp_math::rate::FeeRate::try_from_i64f64(maturity_scaled_rate)
            .map_err(|_| StateError::InvalidConfiguration)?
            .settlement_fee_atoms_scaled(
                u128::from(size),
                self.config.underlying_decimals,
                0,
                self.config.tick_scale,
            )
            .map_err(|_| StateError::ArithmeticOverflow)
    }

    /// Applies a side to a bounded unsigned amount for an internal fill.
    fn signed_amount(amount: u64, side: Side) -> StateResult<i64> {
        // A value greater than i64::MAX is not expected to be passed, but we err as opposed to unwrapping.
        let amount = i64::try_from(amount).map_err(|_| StateError::ArithmeticOverflow)?;

        match side {
            Side::Long => Ok(amount),
            Side::Short => amount.checked_neg().ok_or(StateError::ArithmeticOverflow),
        }
    }

    /// Converts a signed cash change into its nonnegative taker debit.
    fn cash_debit_atoms(cash_delta: i128) -> u128 {
        if cash_delta < 0 {
            cash_delta.unsigned_abs()
        } else {
            0
        }
    }

    /// Returns whether the current best maker price is within the taker's limit.
    fn tick_is_matchable(taker_side: Side, limit_tick: i16, maker_tick: i16) -> bool {
        match taker_side {
            Side::Long => maker_tick <= limit_tick,
            Side::Short => maker_tick >= limit_tick,
        }
    }

    /// Finds the largest amount from one node that remains within the aggregate payment cap.
    fn affordable_fill_size(
        &self,
        candidate: u64,
        maker_side: Side,
        tick: i16,
        time_to_maturity: i64,
        maximum_payment_atoms: u128,
        totals: &MatchTotals,
    ) -> StateResult<u64> {
        // Affordability is monotonic over size, so retain the largest passing
        // prefix of this node rather than testing every possible fill amount.
        let mut low = 1u64;
        let mut high = candidate;
        let mut accepted = 0u64;

        while low <= high {
            let middle = low + (high - low) / 2;
            let fill = Self::signed_amount(middle, maker_side)?;
            let payment = self.fixed_payment_atoms(fill, tick, time_to_maturity)?;

            // The cap applies to the instruction's aggregate payment and fee,
            // not independently to the candidate fill at this tick.
            let fixed = totals
                .fixed_payment_atoms
                .checked_add(payment)
                .ok_or(StateError::ArithmeticOverflow)?;

            let fee = self.taker_fee_atoms(
                totals
                    .filled_size
                    .checked_add(middle)
                    .ok_or(StateError::ArithmeticOverflow)?,
                time_to_maturity,
            )?;

            let cash = fixed
                .checked_neg()
                .and_then(|value| value.checked_sub(i128::try_from(fee).ok()?))
                .ok_or(StateError::ArithmeticOverflow)?;

            if Self::cash_debit_atoms(cash) <= maximum_payment_atoms {
                // This size fits; search the larger half while retaining it as
                // the best known fill in case every larger size exceeds the cap.
                accepted = middle;

                low = middle
                    .checked_add(1)
                    .ok_or(StateError::ArithmeticOverflow)?;
            } else {
                high = middle - 1;
            }
        }

        Ok(accepted)
    }

    /// Converts a book tick into its Q64.64 rate.
    fn rate_at_tick(&self, tick: i16) -> StateResult<Rate> {
        Rate::from_tick(tick, self.order_book.tick_step)
            .map_err(|_| StateError::InvalidConfiguration)
    }

    /// Checks a rate against one symmetric mark-relative distance factor.
    fn rate_within_bound(
        &self,
        rate: FixedPoint,
        mark: FixedPoint,
        factor: FixedPoint,
    ) -> StateResult<bool> {
        let mark_abs = mark
            .to_raw_bits()
            .checked_abs()
            .ok_or(StateError::ArithmeticOverflow)?;

        let base = mark_abs.max(self.config.rate_floor.to_raw_bits());

        let distance = fixed::types::I64F64::from_bits(base)
            .checked_mul(fixed::types::I64F64::from_bits(factor.to_raw_bits()))
            .ok_or(StateError::ArithmeticOverflow)?
            .to_bits();

        let difference = rate
            .to_raw_bits()
            .checked_sub(mark.to_raw_bits())
            .and_then(i128::checked_abs)
            .ok_or(StateError::ArithmeticOverflow)?;

        Ok(difference <= distance)
    }

    /// Advances the linearly interpolated mark target after an eligible matched tick.
    fn update_mark_after_fill(&mut self, now: i64, tick: i16) -> StateResult<()> {
        let current = self.current_mark_rate(now)?;
        let rate = self.rate_at_tick(tick)?.to_fixed_point();

        if !self.rate_within_bound(rate, current, self.config.trade_mark_update_bound_factor)? {
            return Err(StateError::Conflict);
        }

        self.previous_mark_rate = current;
        self.target_mark_rate = rate;
        self.last_eligible_trade_timestamp = now;

        Ok(())
    }

    /// Checks that one fill strictly reduces a position without crossing through zero.
    fn fill_is_closing(position: i64, fill: i64) -> bool {
        position.signum() == -fill.signum() && fill.unsigned_abs() <= position.unsigned_abs()
    }

    /// Projects a market slot through every pending fill without mutating it.
    fn projected_position(&self, subaccount: &Subaccount, slot_index: usize) -> StateResult<i64> {
        let mut position = subaccount.markets[slot_index].position_size;

        for reference in subaccount.iter_order_refs() {
            if usize::from(reference.slot_index) == slot_index {
                position = position
                    .checked_add(
                        self.order_book.nodes[usize::from(reference.node_index)]
                            .pending_fill_size(),
                    )
                    .ok_or(StateError::ArithmeticOverflow)?;
            }
        }

        Ok(position)
    }

    /// Enforces close-only reduction using the taker and supplied maker positions.
    fn check_close_only_fill(
        &self,
        taker: &Subaccount,
        taker_market_slot: usize,
        taker_fill: i64,
        maker_subaccount: Pubkey,
        maker_fill: i64,
        close_only_makers: &[MakerSubaccount<'_>],
    ) -> StateResult<()> {
        if taker.markets[taker_market_slot].close_only_exempt == 0
            && !Self::fill_is_closing(taker.markets[taker_market_slot].position_size, taker_fill)
        {
            return Err(StateError::Conflict);
        }

        let maker_index = close_only_makers
            .binary_search_by_key(&maker_subaccount, |maker| maker.key)
            .map_err(|_| StateError::NotFound)?;

        let maker = close_only_makers[maker_index].subaccount;

        let slot_index = maker
            .lookup_market(self.config.market_id)
            .ok_or(StateError::NotFound)?;

        if maker.markets[slot_index].close_only_exempt == 0
            && !Self::fill_is_closing(self.projected_position(maker, slot_index)?, maker_fill)
        {
            return Err(StateError::Conflict);
        }

        Ok(())
    }

    /// Validates the taker's final position size after all requested fills are applied.
    fn validate_final_position_size(
        &self,
        taker: &Subaccount,
        slot_index: usize,
    ) -> StateResult<()> {
        let size = taker.markets[slot_index].position_size.unsigned_abs();

        if size != 0
            && (size < self.config.minimum_position_size_atoms
                || size > self.config.maximum_position_size_atoms)
        {
            return Err(StateError::Conflict);
        }

        Ok(())
    }

    pub const DATA_LEN: usize = core::mem::size_of::<Self>();
    pub const SPACE: usize = 8 + Self::DATA_LEN;
}

impl VenueAdapterConfig {
    /// Views a Jupiter payload after confirming the active adapter tag.
    pub fn jupiter(&self) -> StateResult<&JupiterAdapterPayload> {
        if self.kind.0 != ADAPTER_JUPITER {
            return Err(StateError::InvalidTag);
        }
        bytemuck::try_from_bytes(
            &bytemuck::bytes_of(&self.payload)[..core::mem::size_of::<JupiterAdapterPayload>()],
        )
        .map_err(|_| StateError::InvalidConfiguration)
    }

    /// Mutably views a Jupiter payload after confirming the active adapter tag.
    pub fn jupiter_mut(&mut self) -> StateResult<&mut JupiterAdapterPayload> {
        if self.kind.0 != ADAPTER_JUPITER {
            return Err(StateError::InvalidTag);
        }
        bytemuck::try_from_bytes_mut(
            &mut bytemuck::bytes_of_mut(&mut self.payload)
                [..core::mem::size_of::<JupiterAdapterPayload>()],
        )
        .map_err(|_| StateError::InvalidConfiguration)
    }

    /// Views a Phoenix payload after confirming the active adapter tag.
    pub fn phoenix(&self) -> StateResult<&PhoenixAdapterPayload> {
        if self.kind.0 != ADAPTER_PHOENIX {
            return Err(StateError::InvalidTag);
        }
        bytemuck::try_from_bytes(bytemuck::bytes_of(&self.payload))
            .map_err(|_| StateError::InvalidConfiguration)
    }

    /// Mutably views a Phoenix payload after confirming the active adapter tag.
    pub fn phoenix_mut(&mut self) -> StateResult<&mut PhoenixAdapterPayload> {
        if self.kind.0 != ADAPTER_PHOENIX {
            return Err(StateError::InvalidTag);
        }
        bytemuck::try_from_bytes_mut(bytemuck::bytes_of_mut(&mut self.payload))
            .map_err(|_| StateError::InvalidConfiguration)
    }

    /// Clears opaque bytes before storing a validated tagged adapter configuration.
    fn zero_opaque_bytes(&mut self) -> StateResult<()> {
        self._pad_0 = [0; 15];
        self.reserved = [0; VENUE_CONFIG_RESERVED];
        match self.kind.0 {
            ADAPTER_JUPITER => {
                let payload = self.jupiter_mut()?;
                payload._pad_0 = [0; 7];
                payload.reserved = [0; JUPITER_PAYLOAD_RESERVED];
                payload._pad_1 = [0; 16];
                bytemuck::bytes_of_mut(&mut self.payload)
                    [core::mem::size_of::<JupiterAdapterPayload>()..]
                    .fill(0);
            }
            ADAPTER_PHOENIX => {
                let payload = self.phoenix_mut()?;
                payload._pad_0 = [0; 10];
                payload.reserved = [0; PHOENIX_PAYLOAD_RESERVED];
            }
            _ => return Err(StateError::InvalidTag),
        }
        Ok(())
    }

    /// Validates the active adapter payload and its required source relationships.
    pub fn validate(&self) -> StateResult<()> {
        match self.kind.0 {
            ADAPTER_JUPITER => {
                let payload = self.jupiter()?;
                if payload.layout.0 != 1
                    || payload.custody == Pubkey::default()
                    || payload.pool == Pubkey::default()
                    || payload.mint == Pubkey::default()
                {
                    return Err(StateError::InvalidConfiguration);
                }
            }
            ADAPTER_PHOENIX => {
                let payload = self.phoenix()?;
                if payload.layout.0 != 1
                    || payload.global_configuration == Pubkey::default()
                    || payload.perp_asset_map == Pubkey::default()
                    || payload.canonical_quote_mint == Pubkey::default()
                    || payload.market_account == Pubkey::default()
                    || payload.symbol == [0; 16]
                    || payload.tick_size == 0
                    || payload.funding_interval_seconds == 0
                    || payload.funding_period_seconds == 0
                    || payload.max_funding_age_seconds == 0
                    || payload.max_mark_age_slots == 0
                {
                    return Err(StateError::InvalidConfiguration);
                }
            }
            _ => return Err(StateError::InvalidTag),
        }
        Ok(())
    }
}

impl MarketConfig {
    #[allow(clippy::too_many_arguments)]
    /// Initializes and validates a complete market configuration.
    pub fn initialize_in_place(
        &mut self,
        hard_gross_open_interest_cap_atoms: u128,
        start_timestamp: i64,
        maturity_timestamp: i64,
        minimum_position_size_atoms: u64,
        maximum_position_size_atoms: u64,
        collateral_state: Pubkey,
        settlement_state: Pubkey,
        underlying_state: Pubkey,
        adapter: VenueAdapterConfig,
        rate_floor: FixedPoint,
        initial_margin_factor: FixedPoint,
        maintenance_margin_factor: FixedPoint,
        order_rate_bound_factor: FixedPoint,
        closing_margin_relief_bound_factor: FixedPoint,
        trade_mark_update_bound_factor: FixedPoint,
        taker_fee_rate: FixedPoint,
        annualized_settlement_fee_rate: FixedPoint,
        liquidation_base_incentive: FixedPoint,
        liquidation_incentive_slope: FixedPoint,
        liquidation_fee_rate: FixedPoint,
        market_id: u32,
        mark_observation_window_seconds: u32,
        margin_time_threshold_seconds: u32,
        tick_scale: u8,
        underlying_decimals: u8,
        settlement_decimals: u8,
    ) -> StateResult<()> {
        self.hard_gross_open_interest_cap_atoms = hard_gross_open_interest_cap_atoms;
        self.start_timestamp = start_timestamp;
        self.maturity_timestamp = maturity_timestamp;
        self.minimum_position_size_atoms = minimum_position_size_atoms;
        self.maximum_position_size_atoms = maximum_position_size_atoms;
        self.collateral_state = collateral_state;
        self.settlement_state = settlement_state;
        self.underlying_state = underlying_state;
        self.adapter = adapter;
        self.rate_floor = rate_floor;
        self.initial_margin_factor = initial_margin_factor;
        self.maintenance_margin_factor = maintenance_margin_factor;
        self.order_rate_bound_factor = order_rate_bound_factor;
        self.closing_margin_relief_bound_factor = closing_margin_relief_bound_factor;
        self.trade_mark_update_bound_factor = trade_mark_update_bound_factor;
        self.taker_fee_rate = taker_fee_rate;
        self.annualized_settlement_fee_rate = annualized_settlement_fee_rate;
        self.liquidation_base_incentive = liquidation_base_incentive;
        self.liquidation_incentive_slope = liquidation_incentive_slope;
        self.liquidation_fee_rate = liquidation_fee_rate;
        self.market_id = market_id;
        self.mark_observation_window_seconds = mark_observation_window_seconds;
        self.margin_time_threshold_seconds = margin_time_threshold_seconds;
        self.tick_scale = tick_scale;
        self.underlying_decimals = underlying_decimals;
        self.settlement_decimals = settlement_decimals;
        self.pm_cache_nonce = 0;
        self.zero_opaque_bytes()?;
        self.validate()
    }

    /// Clears all configuration padding before the configuration is persisted.
    fn zero_opaque_bytes(&mut self) -> StateResult<()> {
        self._pad_0 = [0; 16];
        self._pad_1 = [0; 4];
        self.reserved = [0; MARKET_CONFIG_RESERVED];
        self._pad_2 = [0; 12];
        self._pad_3 = [0; 12];
        self._pad_units = 0;
        self.adapter.zero_opaque_bytes()
    }

    /// Validates caller-provided market limits, factors, and tagged adapter data.
    pub fn validate(&self) -> StateResult<()> {
        if self.tick_scale > fixed_perp_math::rate::MAX_DECIMALS
            || self.underlying_decimals > fixed_perp_math::rate::MAX_DECIMALS
            || self.settlement_decimals > fixed_perp_math::rate::MAX_DECIMALS
        {
            return Err(StateError::InvalidConfiguration);
        }
        if self.market_id == 0
            || self.start_timestamp < 0
            || self.maturity_timestamp <= 0
            || self.start_timestamp >= self.maturity_timestamp
            || self.collateral_state == Pubkey::default()
            || self.settlement_state == Pubkey::default()
            || self.underlying_state == Pubkey::default()
            || self.hard_gross_open_interest_cap_atoms == 0
            || self.minimum_position_size_atoms == 0
            || self.maximum_position_size_atoms == 0
            || self.mark_observation_window_seconds == 0
            || self.margin_time_threshold_seconds == 0
            || self.minimum_position_size_atoms > self.maximum_position_size_atoms
            || u128::from(self.maximum_position_size_atoms)
                > self.hard_gross_open_interest_cap_atoms
        {
            return Err(StateError::InvalidConfiguration);
        }
        let nonnegative = [
            self.rate_floor,
            self.initial_margin_factor,
            self.maintenance_margin_factor,
            self.order_rate_bound_factor,
            self.closing_margin_relief_bound_factor,
            self.trade_mark_update_bound_factor,
            self.taker_fee_rate,
            self.annualized_settlement_fee_rate,
            self.liquidation_base_incentive,
            self.liquidation_incentive_slope,
            self.liquidation_fee_rate,
        ];
        if nonnegative.iter().any(|factor| factor.to_raw_bits() < 0)
            || self.initial_margin_factor.to_raw_bits()
                < self.maintenance_margin_factor.to_raw_bits()
        {
            return Err(StateError::InvalidConfiguration);
        }
        self.adapter.validate()
    }

    /// Returns the adapter tag for callers selecting a validated venue path.
    pub fn adapter_kind(&self) -> VenueAdapterKind {
        self.adapter.kind
    }

    /// Confirms immutable decimal metadata read from the configured AssetState accounts.
    pub fn validate_asset_decimals(
        &self,
        underlying_decimals: u8,
        settlement_decimals: u8,
    ) -> StateResult<()> {
        if underlying_decimals > fixed_perp_math::rate::MAX_DECIMALS
            || settlement_decimals > fixed_perp_math::rate::MAX_DECIMALS
            || self.underlying_decimals != underlying_decimals
            || self.settlement_decimals != settlement_decimals
        {
            return Err(StateError::InvalidConfiguration);
        }
        Ok(())
    }
}

impl MarketState {
    #[allow(clippy::too_many_arguments)]
    /// Initializes a fresh market and its embedded book from validated configuration.
    pub fn initialize_in_place(
        &mut self,
        bump: u8,
        mut config: MarketConfig,
        now: i64,
        tick_step: u8,
        minimum_order_size_atoms: u64,
        maximum_order_size_atoms: u64,
        maximum_open_orders_per_subaccount: u8,
        governance_key: Pubkey,
    ) -> StateResult<()> {
        config.zero_opaque_bytes()?;
        config.validate()?;

        if now < 0 {
            return Err(StateError::InvalidClock);
        }

        if governance_key == Pubkey::default() {
            return Err(StateError::InvalidConfiguration);
        }

        self.order_book.initialize_in_place(
            tick_step,
            minimum_order_size_atoms,
            maximum_order_size_atoms,
            maximum_open_orders_per_subaccount,
        )?;

        self.config = config;
        self.floating_index = FixedPoint::zeroed();
        self.fee_index = FixedPoint::zeroed();
        self.source_native_checkpoint = 0;
        let one = FixedPoint::from_raw_bits(1i128 << 64);
        self.position_factors = ValuationFactors {
            discount_factor: one,
            liability_factor: one,
        };

        self.source_last_update_timestamp = now;
        self.protocol_checkpoint_timestamp = now;
        self.epoch_origin_timestamp = now;
        self.checkpoint_epoch = 0;
        self.previous_mark_rate = FixedPoint::zeroed();
        self.target_mark_rate = FixedPoint::zeroed();
        self.last_eligible_trade_timestamp = now;
        self.materialized_gross_open_interest = 0;
        self.pending_maker_oi_reserve = 0;
        self.accrued_protocol_fee_atoms = 0;
        self.bump = bump;
        self.status = super::MarketStatus(MARKET_PENDING);
        self.source_seeded = 0;
        self._pad_0 = [0; 1];
        self.subaccount_count = 0;
        self.frozen_settlement_ratio = SettlementRatio::zeroed();
        self.finalized_at_timestamp = 0;
        self.reserved = [0; MARKET_STATE_RESERVED];
        self.governance_key = governance_key;

        Ok(())
    }

    /// Activates a pending market inside its coordinator-supplied window.
    pub fn activate(
        &mut self,
        activation_window: &mut super::ProposalWindow,
        now: i64,
    ) -> StateResult<()> {
        if self.status.0 != MARKET_PENDING || self.source_seeded != 1 {
            return Err(StateError::InvalidStatus);
        }

        if now >= self.config.start_timestamp {
            return Err(StateError::Conflict);
        }

        activation_window.consume(now)?;
        self.status = super::MarketStatus(MARKET_ACTIVE);

        Ok(())
    }

    /// Returns the mark interpolated from the prior target over the configured observation window.
    pub fn current_mark_rate(&self, now: i64) -> StateResult<FixedPoint> {
        let elapsed = now
            .checked_sub(self.last_eligible_trade_timestamp)
            .ok_or(StateError::InvalidClock)?;

        if elapsed < 0 {
            return Err(StateError::InvalidClock);
        }

        let window = i64::from(self.config.mark_observation_window_seconds);

        if elapsed >= window {
            return Ok(self.target_mark_rate);
        }

        let ratio = fixed::types::I64F64::checked_from_num(elapsed)
            .and_then(|value| value.checked_div(fixed::types::I64F64::checked_from_num(window)?))
            .ok_or(StateError::ArithmeticOverflow)?;

        let previous = fixed::types::I64F64::from_bits(self.previous_mark_rate.to_raw_bits());

        let target = fixed::types::I64F64::from_bits(self.target_mark_rate.to_raw_bits());

        let current = target
            .checked_sub(previous)
            .and_then(|delta| delta.checked_mul(ratio))
            .and_then(|delta| previous.checked_add(delta))
            .ok_or(StateError::ArithmeticOverflow)?;

        Ok(FixedPoint::from_raw_bits(current.to_bits()))
    }

    fn order_risk_scale(&self) -> OrderRiskScale {
        OrderRiskScale {
            underlying_decimals: self.config.underlying_decimals,
            settlement_decimals: self.config.settlement_decimals,
            tick_scale: self.config.tick_scale,
        }
    }
//
    /// Advances fees with a freshly authenticated underlying/settlement price
    /// supplied by the caller's AssetState/Pyth pair.
    pub fn advance_fee_index_with_price(&mut self, now: i64, price: FixedPoint) -> StateResult<()> {
        if now < self.protocol_checkpoint_timestamp {
            return Err(StateError::InvalidClock);
        }
        let elapsed = now
            .checked_sub(self.protocol_checkpoint_timestamp)
            .ok_or(StateError::InvalidClock)?;
        if elapsed == 0 {
            self.protocol_checkpoint_timestamp = now;
            return Ok(());
        }
        if price.to_raw_bits() <= 0 {
            return Err(StateError::InvalidConfiguration);
        }
        let fee_rate = fixed::types::I64F64::from_bits(
            self.config.annualized_settlement_fee_rate.to_raw_bits(),
        );
        let elapsed = fixed::types::I64F64::checked_from_num(elapsed)
            .ok_or(StateError::ArithmeticOverflow)?;
        let year = fixed::types::I64F64::checked_from_num(SECONDS_PER_YEAR)
            .ok_or(StateError::ArithmeticOverflow)?;
        let market_scale = decimal_scale(self.config.tick_scale)?;
        let delta = fee_rate
            .checked_mul(price.to_i64f64())
            .and_then(|value| value.checked_mul(elapsed))
            .and_then(|value| value.checked_mul(market_scale))
            .and_then(|value| value.checked_div(year))
            .ok_or(StateError::ArithmeticOverflow)?;
        let next = self
            .fee_index
            .to_i64f64()
            .checked_add(delta)
            .ok_or(StateError::ArithmeticOverflow)?;
        self.fee_index = FixedPoint::from_raw_bits(next.to_bits());
        self.protocol_checkpoint_timestamp = now;
        Ok(())
    }

    pub fn apply_rate_observation_with_price(
        &mut self,
        dimensionless_delta: RateDelta,
        source_checkpoint: u128,
        source_last_update_timestamp: i64,
        checkpoint_epoch: u64,
        now: i64,
        price: FixedPoint,
    ) -> StateResult<()> {
        let weighted_delta = dimensionless_delta
            .to_i64f64()
            .checked_mul(price.to_i64f64())
            .ok_or(StateError::ArithmeticOverflow)?;
        self.apply_weighted_rate_observation_with_price(
            RateDelta::from_i64f64(weighted_delta),
            source_checkpoint,
            source_last_update_timestamp,
            checkpoint_epoch,
            now,
            price,
        )
    }

    pub fn apply_weighted_rate_observation_with_price(
        &mut self,
        weighted_delta: RateDelta,
        source_checkpoint: u128,
        source_last_update_timestamp: i64,
        checkpoint_epoch: u64,
        now: i64,
        price: FixedPoint,
    ) -> StateResult<()> {
        if self.source_seeded != 1 {
            return Err(StateError::InvalidStatus);
        }
        if now < self.protocol_checkpoint_timestamp
            || source_last_update_timestamp < 0
            || source_last_update_timestamp > now
            || checkpoint_epoch < self.checkpoint_epoch
        {
            return Err(StateError::InvalidClock);
        }
        let market_scale = decimal_scale(self.config.tick_scale)?;
        let scaled_delta = weighted_delta
            .to_i64f64()
            .checked_mul(market_scale)
            .ok_or(StateError::ArithmeticOverflow)?;
        let next_floating_index = self
            .floating_index
            .to_i64f64()
            .checked_add(scaled_delta)
            .ok_or(StateError::ArithmeticOverflow)?;
        // Protocol fees stop at the market's maturity boundary even when a
        // due refresh is submitted late.
        let fee_now = now.min(self.config.maturity_timestamp);
        if fee_now >= self.protocol_checkpoint_timestamp {
            self.advance_fee_index_with_price(fee_now, price)?;
        }

        self.floating_index = FixedPoint::from_raw_bits(next_floating_index.to_bits());
        self.source_native_checkpoint = source_checkpoint;
        self.source_last_update_timestamp = source_last_update_timestamp;
        self.protocol_checkpoint_timestamp = now;
        self.checkpoint_epoch = checkpoint_epoch;
        Ok(())
    }

    /// Seeds a cumulative venue checkpoint without realizing historical value.
    pub fn seed_source_checkpoint(
        &mut self,
        source_checkpoint: u128,
        source_last_update_timestamp: i64,
        checkpoint_epoch: u64,
        now: i64,
    ) -> StateResult<()> {
        if self.status.0 != MARKET_PENDING || self.source_seeded != 0 {
            return Err(StateError::InvalidStatus);
        }
        if now < self.epoch_origin_timestamp
            || source_last_update_timestamp < 0
            || source_last_update_timestamp > now
        {
            return Err(StateError::InvalidClock);
        }
        self.source_native_checkpoint = source_checkpoint;
        self.source_last_update_timestamp = source_last_update_timestamp;
        self.protocol_checkpoint_timestamp = now;
        self.checkpoint_epoch = checkpoint_epoch;
        self.source_seeded = 1;
        Ok(())
    }

    /// Validates one caller-provided placement against market, time, size, and rate bounds.
    fn validate_place_order_input(
        &self,
        now: i64,
        size: i64,
        limit_tick: i16,
        expiry_timestamp: i64,
        current_mark: FixedPoint,
    ) -> StateResult<()> {
        self.order_book.validate_order_size(size)?;
        if now >= self.config.maturity_timestamp
            || !(self.is_active() || self.is_close_only())
            || (expiry_timestamp != 0
                && (expiry_timestamp <= now || expiry_timestamp > self.config.maturity_timestamp))
        {
            return Err(StateError::InvalidConfiguration);
        }
        if !self.rate_within_bound(
            self.rate_at_tick(limit_tick)?.to_fixed_point(),
            current_mark,
            self.config.order_rate_bound_factor,
        )? {
            return Err(StateError::Conflict);
        }
        Ok(())
    }

    /// Returns whether an action would cross the current best maker price.
    fn order_crosses(&self, side: Side, limit_tick: i16) -> StateResult<bool> {
        let maker_side = match side {
            Side::Long => Side::Short,
            Side::Short => Side::Long,
        };

        match self.order_book.best_tick(maker_side) {
            Ok(tick) => Ok(Self::tick_is_matchable(side, limit_tick, tick)),
            Err(StateError::NotFound) => Ok(false),
            Err(error) => Err(error),
        }
    }

    /// Matches one validated action while accumulating batch-wide payment and fee totals.
    #[allow(clippy::too_many_arguments)]
    fn match_order_action(
        &mut self,
        taker: &mut Subaccount,
        taker_slot: usize,
        taker_key: Pubkey,
        size: i64,
        limit_tick: i16,
        time_to_maturity: i64,
        maximum_payment_atoms: u128,
        close_only_makers: &[MakerSubaccount<'_>],
        totals: &mut MatchTotals,
    ) -> StateResult<PlaceOrderResult> {
        let side = Side::from_size(size)?;
        let maker_side = match side {
            Side::Long => Side::Short,
            Side::Short => Side::Long,
        };
        let start_fixed = totals.fixed_payment_atoms;
        let start_fee = totals.taker_fee_atoms;
        let mut remaining = size;
        // To do: We can and should optimize this loop to tread along orders with the same tick and call
        // `best_head` again when we have exhausted all orders at that tick.
        while remaining != 0 {
            let node_index = match self.order_book.best_head(maker_side) {
                Ok(index) => index,
                Err(StateError::NotFound) => break,
                Err(error) => return Err(error),
            };
            let (maker_subaccount, maker_remaining_size, maker_tick) = {
                let node = &self.order_book.nodes[node_index];
                (node.maker_subaccount, node.remaining_size, node.tick)
            };
            if !Self::tick_is_matchable(side, limit_tick, maker_tick) {
                break;
            }
            if maker_subaccount == taker_key {
                return Err(StateError::Conflict);
            }
            let candidate = remaining
                .unsigned_abs()
                .min(maker_remaining_size.unsigned_abs());
            let fill_abs = self.affordable_fill_size(
                candidate,
                maker_side,
                maker_tick,
                time_to_maturity,
                maximum_payment_atoms,
                totals,
            )?;
            if fill_abs == 0 {
                break;
            }
            let maker_fill = Self::signed_amount(fill_abs, maker_side)?;
            let taker_fill = maker_fill
                .checked_neg()
                .ok_or(StateError::ArithmeticOverflow)?;
            if self.is_close_only() {
                self.check_close_only_fill(
                    taker,
                    taker_slot,
                    taker_fill,
                    maker_subaccount,
                    maker_fill,
                    close_only_makers,
                )?;
            }
            let payment = self.fixed_payment_atoms(maker_fill, maker_tick, time_to_maturity)?;
            let next_fixed = totals
                .fixed_payment_atoms
                .checked_add(payment)
                .ok_or(StateError::ArithmeticOverflow)?;
            let next_filled = totals
                .filled_size
                .checked_add(fill_abs)
                .ok_or(StateError::ArithmeticOverflow)?;
            let next_fee = self.taker_fee_atoms(next_filled, time_to_maturity)?;
            self.apply_maker_match_and_taker_fill(
                taker,
                taker_slot,
                node_index,
                taker_fill,
                payment,
                next_fee
                    .checked_sub(totals.taker_fee_atoms)
                    .ok_or(StateError::ArithmeticOverflow)?,
            )?;
            totals.filled_size = next_filled;
            totals.fixed_payment_atoms = next_fixed;
            totals.taker_fee_atoms = next_fee;
            totals.last_matched_tick = Some(maker_tick);
            remaining = remaining
                .checked_sub(taker_fill)
                .ok_or(StateError::ArithmeticOverflow)?;
        }
        Ok(PlaceOrderResult {
            filled_size: size
                .checked_sub(remaining)
                .ok_or(StateError::ArithmeticOverflow)?,
            remaining_size: remaining,
            fixed_payment_atoms: totals
                .fixed_payment_atoms
                .checked_sub(start_fixed)
                .ok_or(StateError::ArithmeticOverflow)?,
            taker_fee_atoms: totals
                .taker_fee_atoms
                .checked_sub(start_fee)
                .ok_or(StateError::ArithmeticOverflow)?,
            resting_order: None,
        })
    }

    /// Matches at available prices up to the submitted limit, then applies the time-in-force rule.
    /// The caller materializes the taker before invoking this entry point.
    /// Close-only matching additionally receives the sorted maker accounts it may touch.
    pub fn place_order(
        &mut self,
        taker: &mut Subaccount,
        taker_key: Pubkey,
        now: i64,
        order_intent: PlaceOrderIntent,
        close_only_makers: &[MakerSubaccount<'_>],
    ) -> StateResult<PlaceOrderResult> {
        order_intent.time_in_force.validate()?;
        let current_mark = self.current_mark_rate(now)?;
        self.validate_place_order_input(
            now,
            order_intent.size,
            order_intent.limit_tick,
            order_intent.expiry_timestamp,
            current_mark,
        )?;
        let side = Side::from_size(order_intent.size)?;
        // Post-only crossing checks do not require a subaccount slot.  Run
        // them before `ensure_market` so a rejected or skipped order cannot
        // leave an empty slot behind.
        if order_intent.time_in_force.is_post_only()
            && self.order_crosses(side, order_intent.limit_tick)?
        {
            if order_intent.time_in_force.rejects_crossing() {
                return Err(StateError::Conflict);
            }
            if order_intent.time_in_force.skips_crossing() {
                return Ok(PlaceOrderResult {
                    remaining_size: order_intent.size,
                    ..PlaceOrderResult::default()
                });
            }
        }
        // Create the slot only after every input gate above succeeds. A
        // rejected order must not increment the market's subaccount count.
        let taker_market_slot = taker.ensure_market(self)?;
        let time_to_maturity = self
            .config
            .maturity_timestamp
            .checked_sub(now)
            .ok_or(StateError::InvalidClock)?;
        let mut totals = MatchTotals::default();
        let mut result = if order_intent.time_in_force.is_post_only() {
            PlaceOrderResult {
                remaining_size: order_intent.size,
                ..PlaceOrderResult::default()
            }
        } else {
            self.match_order_action(
                taker,
                taker_market_slot,
                taker_key,
                order_intent.size,
                order_intent.limit_tick,
                time_to_maturity,
                order_intent.maximum_payment_atoms,
                close_only_makers,
                &mut totals,
            )?
        };
        if let Some(tick) = totals.last_matched_tick {
            self.validate_final_position_size(taker, taker_market_slot)?;
            self.update_mark_after_fill(now, tick)?;
        }
        if order_intent.time_in_force.is_fill_or_kill() && result.remaining_size != 0 {
            return Err(StateError::NotFound);
        }
        if result.remaining_size != 0
            && order_intent.time_in_force.rests_remainder()
            && !self.order_crosses(side, order_intent.limit_tick)?
        {
            result.resting_order = Some(self.rest_order(
                taker,
                taker_market_slot,
                taker_key,
                RestingOrder {
                    size: result.remaining_size,
                    tick: order_intent.limit_tick,
                    time_in_force: order_intent.time_in_force,
                    expiry_timestamp: order_intent.expiry_timestamp,
                },
            )?);
        }
        Ok(result)
    }

    /// Places a bounded same-side batch under one time-in-force and payment budget.
    /// The caller materializes the taker before invoking this entry point.
    /// Close-only matching additionally receives the sorted maker accounts it may touch.
    pub fn place_order_batch(
        &mut self,
        taker: &mut Subaccount,
        taker_key: Pubkey,
        now: i64,
        intent: PlaceOrderBatchIntent<'_>,
        close_only_makers: &[MakerSubaccount<'_>],
        results: &mut [PlaceOrderResult],
    ) -> StateResult<()> {
        intent.time_in_force.validate()?;
        if intent.orders.is_empty()
            || intent.orders.len() > super::MAX_ORDERS
            || results.len() != intent.orders.len()
        {
            return Err(StateError::InvalidConfiguration);
        }
        let current_mark = self.current_mark_rate(now)?;
        let time_to_maturity = self
            .config
            .maturity_timestamp
            .checked_sub(now)
            .ok_or(StateError::InvalidClock)?;
        for order in intent.orders {
            self.validate_place_order_input(
                now,
                Self::signed_amount(order.size, intent.side)?,
                order.limit_tick,
                order.expiry_timestamp,
                current_mark,
            )?;
        }
        // As in the single-order path, defer slot allocation until the whole
        // batch has passed validation.
        let taker_slot = taker.ensure_market(self)?;
        let mut totals = MatchTotals::default();
        for (order, result) in intent.orders.iter().zip(results.iter_mut()) {
            let size = Self::signed_amount(order.size, intent.side)?;
            *result = if intent.time_in_force.is_post_only() {
                if self.order_crosses(intent.side, order.limit_tick)?
                    && intent.time_in_force.rejects_crossing()
                {
                    return Err(StateError::Conflict);
                }
                PlaceOrderResult {
                    remaining_size: size,
                    ..PlaceOrderResult::default()
                }
            } else {
                self.match_order_action(
                    taker,
                    taker_slot,
                    taker_key,
                    size,
                    order.limit_tick,
                    time_to_maturity,
                    intent.maximum_payment_atoms,
                    close_only_makers,
                    &mut totals,
                )?
            };
        }
        if let Some(tick) = totals.last_matched_tick {
            self.validate_final_position_size(taker, taker_slot)?;
            self.update_mark_after_fill(now, tick)?;
        }
        if intent.time_in_force.is_fill_or_kill()
            && results.iter().any(|result| result.remaining_size != 0)
        {
            return Err(StateError::NotFound);
        }
        for (order, result) in intent.orders.iter().zip(results.iter_mut()) {
            if result.remaining_size != 0
                && intent.time_in_force.rests_remainder()
                && !self.order_crosses(intent.side, order.limit_tick)?
            {
                result.resting_order = Some(self.rest_order(
                    taker,
                    taker_slot,
                    taker_key,
                    RestingOrder {
                        size: result.remaining_size,
                        tick: order.limit_tick,
                        time_in_force: intent.time_in_force,
                        expiry_timestamp: order.expiry_timestamp,
                    },
                )?);
            }
        }
        Ok(())
    }

    /// Finds a live reference for a known market slot and tick.
    fn find_order_ref_at_tick(
        &self,
        subaccount: &Subaccount,
        slot_index: usize,
        tick: i16,
    ) -> Option<usize> {
        for (index, reference) in subaccount.iter_order_refs().iter().enumerate() {
            if usize::from(reference.slot_index) != slot_index {
                continue;
            }
            let node = &self.order_book.nodes[usize::from(reference.node_index)];
            if node.tick == tick {
                return Some(index);
            }
        }
        None
    }

    /// Commits a rest-eligible remainder to the book and its already-entered market slot.
    fn rest_order(
        &mut self,
        subaccount: &mut Subaccount,
        slot_index: usize,
        subaccount_key: Pubkey,
        order: RestingOrder,
    ) -> StateResult<OrderId> {
        // A valid submitted size can leave a remainder below the resting-order minimum.
        self.order_book.validate_order_size(order.size)?;
        if self
            .find_order_ref_at_tick(subaccount, slot_index, order.tick)
            .is_some()
        {
            return Err(StateError::Conflict);
        }

        // TODO: Would it make more sense to store it on the struct?
        let market_order_count = subaccount
            .iter_order_refs()
            .iter()
            .filter(|reference| usize::from(reference.slot_index) == slot_index)
            .count();
        if usize::from(subaccount.order_ref_count) >= super::MAX_ORDERS
            || market_order_count >= usize::from(self.order_book.maximum_open_orders_per_subaccount)
        {
            return Err(StateError::CapacityExhausted);
        }
        let rate = self.rate_at_tick(order.tick)?.to_fixed_point();
        let mut next_risk = subaccount.markets[slot_index].order_risk;
        next_risk.add(
            Side::from_size(order.size)?,
            order.size.unsigned_abs(),
            rate,
            self.config.rate_floor,
            self.order_risk_scale(),
        )?;
        // TODO: It would be better to check this only once when on the close batch path.
        if self.is_close_only()
            && !(aggregate_is_closing_action(
                subaccount.markets[slot_index].position_size,
                subaccount.markets[slot_index].position_size,
                next_risk.sum_long_order_size,
                next_risk.sum_short_order_size,
                Some(Side::from_size(order.size)?),
            ) || subaccount.markets[slot_index].close_only_exempt != 0)
        {
            return Err(StateError::Conflict);
        }
        let order_id = self.order_book.add_order(
            subaccount_key,
            order.size,
            order.tick,
            order.time_in_force,
            order.expiry_timestamp,
        )?;
        subaccount.markets[slot_index].order_risk = next_risk;
        subaccount.add_order_ref(OrderRef::new(slot_index, order_id.slab_index as usize));
        Ok(order_id)
    }

    /// Locates this subaccount's live reference in the known market slot for a
    /// stable order ID.
    ///
    /// The node index must match the ID's slab index and the node's allocation
    /// sequence must match the ID's sequence, which rejects both a stale
    /// sequence and a closed ID whose slab slot was reused by a newer order.
    fn find_order_ref_by_id(
        &self,
        subaccount: &Subaccount,
        slot_index: usize,
        order_id: OrderId,
    ) -> Option<usize> {
        if order_id.slab_index >= BOOK_CAPACITY as u64 {
            return None;
        }
        subaccount
            .iter_order_refs()
            .iter()
            .enumerate()
            .find_map(|(index, reference)| {
                let node_index = usize::from(reference.node_index);
                let owned_here = usize::from(reference.slot_index) == slot_index
                    && u64::from(reference.node_index) == order_id.slab_index
                    && self.order_book.nodes[node_index].sequence == order_id.sequence;
                owned_here.then_some(index)
            })
    }

    /// Locates this subaccount's live reference in the known market slot for a
    /// known node index.
    fn find_ref_index_by_node(
        subaccount: &Subaccount,
        slot_index: usize,
        node_index: usize,
    ) -> Option<usize> {
        subaccount
            .iter_order_refs()
            .iter()
            .enumerate()
            .find_map(|(index, reference)| {
                (usize::from(reference.slot_index) == slot_index
                    && usize::from(reference.node_index) == node_index)
                    .then_some(index)
            })
    }

    /// Cancels one live reference's unfilled remainder and updates cached order risk.
    ///
    /// A materialized node is released with its reference. A pending node is
    /// detached but retained until its matched portion is materialized.
    fn cancel_order_ref(
        &mut self,
        subaccount: &mut Subaccount,
        slot_index: usize,
        ref_index: usize,
    ) -> StateResult<()> {
        let node_index = usize::from(subaccount.order_refs[ref_index].node_index);
        let (pending_size, remaining_size, tick, side) = {
            let node = &self.order_book.nodes[node_index];
            (
                node.pending_fill_size(),
                node.remaining_size,
                node.tick,
                node.get_side(),
            )
        };
        let rate = self.rate_at_tick(tick)?.to_fixed_point();
        let mut next_risk = subaccount.markets[slot_index].order_risk;
        next_risk.subtract(
            side,
            remaining_size.unsigned_abs(),
            rate,
            self.config.rate_floor,
            self.order_risk_scale(),
        )?;
        if pending_size == 0 {
            self.order_book.close_order(node_index)?;
            subaccount.remove_order_ref(ref_index);
        } else if remaining_size != 0 {
            self.order_book.detach_from_fifo(node_index)?;
            self.order_book
                .set_order_sizes(node_index, pending_size, 0)?;
        }
        subaccount.markets[slot_index].order_risk = next_risk;
        Ok(())
    }

    /// Changes a materialized order's remaining size, located by its stable
    /// order ID. A zero size takes the close path.
    ///
    /// The caller establishes the market slot and materializes this subaccount
    /// first. The ID locates the exact node and maker reference and rejects a
    /// stale sequence or a reused slab index. Modification never changes the
    /// tick or flips the side: a caller wanting either cancels and places a
    /// new order. A decrease keeps the order's FIFO priority; an increase
    /// moves it to the FIFO tail of its tick level.
    pub fn modify_order(
        &mut self,
        subaccount: &mut Subaccount,
        order_id: OrderId,
        new_size: i64,
    ) -> StateResult<()> {
        if new_size == 0 {
            return self.close_order(subaccount, order_id);
        }
        let slot_index = subaccount
            .lookup_market(self.config.market_id)
            .ok_or(StateError::NotFound)?;
        let Some(ref_index) = self.find_order_ref_by_id(subaccount, slot_index, order_id) else {
            return Err(StateError::NotFound);
        };
        let node_index = usize::from(subaccount.order_refs[ref_index].node_index);
        let (remaining_size, tick, side) = {
            let node = &self.order_book.nodes[node_index];
            (node.remaining_size, node.tick, node.get_side())
        };
        if self.order_book.nodes[node_index].has_pending_fill() {
            return Err(StateError::Conflict);
        }

        // We don't flip side from the modify path: https://pochettes.pulsradio.com/drake%20-%20hotline%20bling.jpg
        if new_size.signum() != remaining_size.signum() {
            return Err(StateError::InvalidConfiguration);
        }
        let old_size = remaining_size.unsigned_abs();
        let new_size_abs = new_size.unsigned_abs();
        if new_size_abs == old_size {
            return Ok(());
        }
        let increases = new_size_abs > old_size;
        if increases && !(self.is_active() || self.is_close_only()) {
            return Err(StateError::InvalidStatus);
        }
        self.order_book.validate_order_size(new_size)?;
        let rate = self.rate_at_tick(tick)?.to_fixed_point();
        let mut next_risk = subaccount.markets[slot_index].order_risk;
        if increases {
            next_risk.add(
                side,
                new_size_abs
                    .checked_sub(old_size)
                    .ok_or(StateError::ArithmeticOverflow)?,
                rate,
                self.config.rate_floor,
                self.order_risk_scale(),
            )?;
        } else {
            next_risk.subtract(
                side,
                old_size
                    .checked_sub(new_size_abs)
                    .ok_or(StateError::ArithmeticOverflow)?,
                rate,
                self.config.rate_floor,
                self.order_risk_scale(),
            )?;
        }
        if self.is_close_only()
            && !(aggregate_is_closing_action(
                subaccount.markets[slot_index].position_size,
                subaccount.markets[slot_index].position_size,
                next_risk.sum_long_order_size,
                next_risk.sum_short_order_size,
                Some(side),
            ) || subaccount.markets[slot_index].close_only_exempt != 0)
        {
            return Err(StateError::Conflict);
        }
        self.order_book
            .set_order_sizes(node_index, new_size, new_size)?;
        subaccount.markets[slot_index].order_risk = next_risk;
        Ok(())
    }

    /// Applies same-market stable-ID size changes after the caller establishes
    /// and materializes the slot.
    pub fn modify_order_batch(
        &mut self,
        subaccount: &mut Subaccount,
        changes: &[ModifyOrderIntent],
    ) -> StateResult<()> {
        if changes.len() > super::MAX_ORDERS {
            return Err(StateError::InvalidConfiguration);
        }
        for change in changes {
            self.modify_order(subaccount, change.order_id, change.new_size)?;
        }
        Ok(())
    }

    /// Cancels the unfilled remainder located by stable ID after the caller establishes the slot.
    /// Any pending matched portion remains detached until materialization.
    pub fn close_order(
        &mut self,
        subaccount: &mut Subaccount,
        order_id: OrderId,
    ) -> StateResult<()> {
        let slot_index = subaccount
            .lookup_market(self.config.market_id)
            .ok_or(StateError::NotFound)?;
        let Some(ref_index) = self.find_order_ref_by_id(subaccount, slot_index, order_id) else {
            return Err(StateError::NotFound);
        };
        self.cancel_order_ref(subaccount, slot_index, ref_index)
    }

    /// Cancels a bounded list of selected unfilled remainders after slot establishment.
    /// Any pending matched portions remain detached until materialization.
    pub fn close_order_batch(
        &mut self,
        subaccount: &mut Subaccount,
        orders: &[OrderId],
    ) -> StateResult<()> {
        if orders.len() > super::MAX_ORDERS {
            return Err(StateError::InvalidConfiguration);
        }
        for order_id in orders {
            // To do: Rather than repeatedly multiplying along the cancel_order_ref path, we can instead
            // choose to just sum up the sizes and update the risk after.
            // We can probably do that more generally for variants with more that one order to cancel, even in
            // in the single case as well.
            // We can also extend the idea to place and modify as well.
            self.close_order(subaccount, *order_id)?;
        }
        Ok(())
    }

    /// Cleanup: cancels every unfilled order remainder this subaccount holds in this market.
    ///
    /// Pending matched portions remain detached until materialization. The pass
    /// walks backward so removing materialized references never revisits one.
    pub fn close_all_orders(
        &mut self,
        subaccount: &mut Subaccount,
        slot_index: usize,
    ) -> StateResult<()> {
        if slot_index >= subaccount.markets.len() {
            return Err(StateError::InvalidReference);
        }
        let mut ref_index = usize::from(subaccount.order_ref_count);
        while ref_index > 0 {
            ref_index -= 1;
            if usize::from(subaccount.order_refs[ref_index].slot_index) == slot_index {
                self.cancel_order_ref(subaccount, slot_index, ref_index)?;
            }
        }
        Ok(())
    }

    /// Cleanup: closes this subaccount's orders resting outside the
    /// caller-supplied inclusive tick band.
    ///
    /// The caller owns the band (for example a mark-relative containment
    /// band); this helper only validates its ordering. Pending matched portions
    /// remain detached until materialization.
    /// The pass walks the reference array backward, so swap-removals never
    /// revisit a reference.
    pub fn close_out_of_bounds_orders(
        &mut self,
        subaccount: &mut Subaccount,
        minimum_tick: i16,
        maximum_tick: i16,
    ) -> StateResult<()> {
        if minimum_tick > maximum_tick {
            return Err(StateError::InvalidConfiguration);
        }
        let slot_index = subaccount
            .lookup_market(self.config.market_id)
            .ok_or(StateError::NotFound)?;
        let mut ref_index = usize::from(subaccount.order_ref_count);
        while ref_index > 0 {
            ref_index -= 1;
            if usize::from(subaccount.order_refs[ref_index].slot_index) == slot_index {
                let node_index = usize::from(subaccount.order_refs[ref_index].node_index);
                let out_of_bounds = {
                    let node = &self.order_book.nodes[node_index];
                    node.tick < minimum_tick || node.tick > maximum_tick
                };
                if out_of_bounds {
                    self.cancel_order_ref(subaccount, slot_index, ref_index)?;
                }
            }
        }
        Ok(())
    }

    /// Cleanup: cancels orders on the projected position's side and preserves
    /// orders on the opposite, possibly exposure-reducing side.
    ///
    /// A zero projected position has no reducing side, so both sides are
    /// cancelled.
    pub fn close_wrong_side_orders(&mut self, subaccount: &mut Subaccount) -> StateResult<()> {
        let slot_index = subaccount
            .lookup_market(self.config.market_id)
            .ok_or(StateError::NotFound)?;
        let position = self.projected_position(subaccount, slot_index)?;
        let mut ref_index = usize::from(subaccount.order_ref_count);
        while ref_index > 0 {
            ref_index -= 1;
            if usize::from(subaccount.order_refs[ref_index].slot_index) == slot_index {
                let node_index = usize::from(subaccount.order_refs[ref_index].node_index);
                let node = &self.order_book.nodes[node_index];
                let increases = position == 0 || node.size.signum() == position.signum();
                if increases {
                    self.cancel_order_ref(subaccount, slot_index, ref_index)?;
                }
            }
        }
        Ok(())
    }

    /// Cleanup: trims this subaccount's opposite-side closing size in this
    /// market down to the position size without allowing a flip, preserving
    /// the best-priced reducing orders.
    ///
    /// Best-priced means the side's fill order: the lowest ticks for resting
    /// shorts closing a long position and the highest ticks for resting longs
    /// closing a short position; equal ticks keep reference order. The order
    /// straddling the remaining position size is reduced (a decrease keeps its
    /// FIFO priority) when the trimmed remainder still satisfies the book's
    /// minimum order size, and closed otherwise. Fully excess orders are
    /// closed. Pending matched portions affect the projected position but are
    /// retained until materialization.
    pub fn close_excess_opposite_orders(&mut self, subaccount: &mut Subaccount) -> StateResult<()> {
        let slot_index = subaccount
            .lookup_market(self.config.market_id)
            .ok_or(StateError::NotFound)?;
        let position = self.projected_position(subaccount, slot_index)?;
        let negated = match position.checked_neg() {
            Some(size) => size,
            None => return Ok(()),
        };
        let opposite_side = match Side::from_size(negated) {
            Ok(side) => side,
            Err(_) => {
                // A zero position has no closing side; wrong-side cleanup owns it.
                return Ok(());
            }
        };
        let opposite_sign = match opposite_side {
            Side::Long => 1i64,
            Side::Short => -1i64,
        };

        // One bounded reference pass collects this subaccount's opposite-side
        // candidates for a best-first walk.
        let mut candidates = [(0i16, 0u16); super::MAX_ORDERS];
        let mut candidate_count = 0usize;
        let mut opposite_total = 0u64;
        for reference in subaccount.iter_order_refs() {
            if usize::from(reference.slot_index) == slot_index {
                let node = &self.order_book.nodes[usize::from(reference.node_index)];
                if node.get_side() != opposite_side || node.remaining_size == 0 {
                    continue;
                }
                opposite_total = opposite_total
                    .checked_add(node.remaining_size.unsigned_abs())
                    .ok_or(StateError::ArithmeticOverflow)?;
                candidates[candidate_count] = (node.tick, reference.node_index);
                candidate_count += 1;
            }
        }
        let needed = position.unsigned_abs();
        if opposite_total <= needed {
            return Ok(());
        }

        // Insertion sort into fill order: shorts fill from the lowest tick and
        // longs from the highest tick, mirroring the book's best-tick choice.
        let fills_first = |candidate: i16, against: i16| match opposite_side {
            Side::Short => candidate < against,
            Side::Long => candidate > against,
        };
        for position_index in 1..candidate_count {
            let entry = candidates[position_index];
            let mut hole = position_index;
            while hole > 0 && fills_first(entry.0, candidates[hole - 1].0) {
                candidates[hole] = candidates[hole - 1];
                hole -= 1;
            }
            candidates[hole] = entry;
        }

        let mut kept = 0u64;
        for &(tick, node_index) in &candidates[..candidate_count] {
            let node_index = usize::from(node_index);
            let size = self.order_book.nodes[node_index]
                .remaining_size
                .unsigned_abs();
            if kept >= needed {
                let Some(ref_index) =
                    Self::find_ref_index_by_node(subaccount, slot_index, node_index)
                else {
                    return Err(StateError::InvalidReference);
                };
                self.cancel_order_ref(subaccount, slot_index, ref_index)?;
            } else if kept.checked_add(size).is_some_and(|total| total <= needed) {
                kept += size;
            } else {
                // The straddling order: trim it to exactly cover the position.
                let trim_target = needed - kept;
                kept = needed;
                let minimum = self.order_book.minimum_order_size_atoms;
                if trim_target < minimum {
                    let Some(ref_index) =
                        Self::find_ref_index_by_node(subaccount, slot_index, node_index)
                    else {
                        return Err(StateError::InvalidReference);
                    };
                    self.cancel_order_ref(subaccount, slot_index, ref_index)?;
                } else {
                    let rate = self.rate_at_tick(tick)?.to_fixed_point();
                    let signed_target = i64::try_from(trim_target)
                        .ok()
                        .and_then(|target| target.checked_mul(opposite_sign))
                        .ok_or(StateError::ArithmeticOverflow)?;
                    let mut next_risk = subaccount.markets[slot_index].order_risk;
                    next_risk.subtract(
                        opposite_side,
                        size.checked_sub(trim_target)
                            .ok_or(StateError::ArithmeticOverflow)?,
                        rate,
                        self.config.rate_floor,
                        self.order_risk_scale(),
                    )?;
                    let pending_size = self.order_book.nodes[node_index].pending_fill_size();
                    let next_size = pending_size
                        .checked_add(signed_target)
                        .ok_or(StateError::ArithmeticOverflow)?;
                    self.order_book
                        .set_order_sizes(node_index, next_size, signed_target)?;
                    subaccount.markets[slot_index].order_risk = next_risk;
                }
            }
        }
        Ok(())
    }

    /// Shapes this subaccount's orders in this market for close-only operation.
    /// Assumes pending fills were materialized before cleanup.
    pub fn close_only_cleanup(&mut self, subaccount: &mut Subaccount) -> StateResult<()> {
        self.close_wrong_side_orders(subaccount)?;
        self.close_excess_opposite_orders(subaccount)
    }

    /// Settles the slot's current position through both indices without resetting snapshots.
    /// The caller materializes pending orders before invoking the separate snapshot reset.
    /// The settlement fee debited here is credited to the market's protocol fee ledger.
    /// The `market_slot_index` passed is expected to be valid.
    pub fn materialize_position_indices(
        &mut self,
        subaccount: &mut Subaccount,
        market_slot_index: usize,
    ) -> StateResult<()> {
        let market_slot = &subaccount.markets[market_slot_index];
        let floating_delta = self
            .floating_index
            .to_raw_bits()
            .checked_sub(market_slot.floating_index_snapshot.to_raw_bits())
            .ok_or(StateError::ArithmeticOverflow)?;
        let floating_atoms = fixed_perp_math::rate::RateDelta::from_i64f64(
            fixed::types::I64F64::from_bits(floating_delta),
        )
        .floating_payment_atoms_scaled(
            i128::from(market_slot.position_size),
            self.config.underlying_decimals,
            self.config.settlement_decimals,
            self.config.tick_scale,
        )
        .map_err(|_| StateError::ArithmeticOverflow)?;

        let fee_delta = self
            .fee_index
            .to_raw_bits()
            .checked_sub(market_slot.fee_index_snapshot.to_raw_bits())
            .ok_or(StateError::ArithmeticOverflow)?;
        let fee_atoms = fixed_perp_math::rate::FeeRate::try_from_i64f64(
            fixed::types::I64F64::from_bits(fee_delta),
        )
        .and_then(|fee| {
            fee.settlement_fee_atoms_scaled(
                market_slot.position_size.unsigned_abs().into(),
                self.config.underlying_decimals,
                self.config.settlement_decimals,
                self.config.tick_scale,
            )
        })
        .map_err(|_| StateError::ArithmeticOverflow)?;
        let next_settlement_balance = market_slot
            .settlement_balance_atoms
            .checked_add(floating_atoms)
            .and_then(|value| value.checked_sub(i128::try_from(fee_atoms).ok()?))
            .ok_or(StateError::ArithmeticOverflow)?;
        let next_accrued_fee = self
            .accrued_protocol_fee_atoms
            .checked_add(fee_atoms)
            .ok_or(StateError::ArithmeticOverflow)?;

        subaccount.markets[market_slot_index].settlement_balance_atoms = next_settlement_balance;
        self.accrued_protocol_fee_atoms = next_accrued_fee;
        Ok(())
    }

    /// Resets this market slot's index snapshots after position and order materialization.
    /// The caller completes both forms of settlement before invoking this final step.
    /// The `market_slot_index` passed is expected to be valid.
    pub fn reset_position_index_snapshots(
        &self,
        subaccount: &mut Subaccount,
        market_slot_index: usize,
    ) {
        subaccount.markets[market_slot_index].floating_index_snapshot = self.floating_index;
        subaccount.markets[market_slot_index].fee_index_snapshot = self.fee_index;
    }

    /// Materializes one caller-selected live maker reference into its subaccount.
    /// Snapshot reset remains a separate caller-owned step after all materialization.
    /// The `ref_index` passed is expected to be valid.
    pub fn materialize_pending_order(
        &mut self,
        subaccount: &mut Subaccount,
        ref_index: usize,
    ) -> StateResult<()> {
        let reference = subaccount.order_refs[ref_index];
        let slot_index = usize::from(reference.slot_index);
        let node_index = usize::from(reference.node_index);
        let (
            pending,
            remaining_size,
            tick,
            side,
            pending_floating_index_basis,
            pending_fee_index_basis,
            pending_fixed_payment_atoms,
        ) = {
            let node = &self.order_book.nodes[node_index];
            (
                node.pending_fill_size(),
                node.remaining_size,
                node.tick,
                node.get_side(),
                node.pending_floating_index_basis,
                node.pending_fee_index_basis,
                node.pending_fixed_payment_atoms,
            )
        };
        if pending == 0 {
            return Ok(());
        }

        // The pending basis is already fill-size weighted. The payment formula
        // therefore multiplies by one atom and still divides by the underlying
        // decimal scale once, matching position-index settlement.
        let next_position = subaccount.markets[slot_index]
            .position_size
            .checked_add(pending)
            .ok_or(StateError::ArithmeticOverflow)?;
        let floating_delta = i128::from(pending)
            .checked_mul(self.floating_index.to_raw_bits())
            .and_then(|value| value.checked_sub(pending_floating_index_basis.to_raw_bits()))
            .ok_or(StateError::ArithmeticOverflow)?;
        let floating_atoms = fixed_perp_math::rate::RateDelta::from_i64f64(
            fixed::types::I64F64::from_bits(floating_delta),
        )
        .floating_payment_atoms_scaled(
            1,
            self.config.underlying_decimals,
            self.config.settlement_decimals,
            self.config.tick_scale,
        )
        .map_err(|_| StateError::ArithmeticOverflow)?;
        let fee_delta = i128::from(pending.unsigned_abs())
            .checked_mul(self.fee_index.to_raw_bits())
            .and_then(|value| value.checked_sub(pending_fee_index_basis.to_raw_bits()))
            .ok_or(StateError::ArithmeticOverflow)?;
        let fee_atoms = fixed_perp_math::rate::FeeRate::try_from_i64f64(
            fixed::types::I64F64::from_bits(fee_delta),
        )
        .map_err(|_| StateError::ArithmeticOverflow)?
        .settlement_fee_atoms_scaled(
            1,
            self.config.underlying_decimals,
            self.config.settlement_decimals,
            self.config.tick_scale,
        )
        .map_err(|_| StateError::ArithmeticOverflow)?;
        let cash_delta = floating_atoms
            .checked_sub(i128::try_from(fee_atoms).map_err(|_| StateError::ArithmeticOverflow)?)
            .and_then(|value| value.checked_add(pending_fixed_payment_atoms))
            .ok_or(StateError::ArithmeticOverflow)?;
        let next_cash = subaccount.markets[slot_index]
            .settlement_balance_atoms
            .checked_add(cash_delta)
            .ok_or(StateError::ArithmeticOverflow)?;
        let next_accrued_fee = self
            .accrued_protocol_fee_atoms
            .checked_add(fee_atoms)
            .ok_or(StateError::ArithmeticOverflow)?;
        let old_position_abs =
            u128::from(subaccount.markets[slot_index].position_size.unsigned_abs());
        let new_position_abs = u128::from(next_position.unsigned_abs());
        let pending_abs = u128::from(pending.unsigned_abs());
        let rate = self.rate_at_tick(tick)?.to_fixed_point();
        let mut next_risk = subaccount.markets[slot_index].order_risk;
        next_risk.subtract(
            side,
            pending.unsigned_abs(),
            rate,
            self.config.rate_floor,
            self.order_risk_scale(),
        )?;
        self.materialize_pending_maker_oi(pending_abs, old_position_abs, new_position_abs)?;

        subaccount.markets[slot_index].settlement_balance_atoms = next_cash;
        self.accrued_protocol_fee_atoms = next_accrued_fee;
        subaccount.markets[slot_index].position_size = next_position;
        subaccount.markets[slot_index].order_risk = next_risk;
        if remaining_size == 0 {
            self.order_book.release_detached_node(node_index);
            subaccount.remove_order_ref(ref_index);
        } else {
            self.order_book.commit_pending_fill(node_index);
        }
        Ok(())
    }

    /// Calculates a signed fixed-leg settlement-atom change for a bounded fill
    /// at one tick using the market's atomic-first decimal boundary.
    fn fixed_payment_atoms(
        &self,
        signed_size: i64,
        tick: i16,
        time_to_maturity: i64,
    ) -> StateResult<i128> {
        self.rate_at_tick(tick)?
            .fixed_payment_atoms_scaled(
                i128::from(signed_size),
                time_to_maturity,
                SECONDS_PER_YEAR,
                self.config.underlying_decimals,
                self.config.settlement_decimals,
                self.config.tick_scale,
            )
            .map_err(|_| StateError::ArithmeticOverflow)
    }

    /// Calculates the incremental taker fee for a bounded aggregate fill size.
    fn taker_fee_atoms(&self, size: u64, time_to_maturity: i64) -> StateResult<u128> {
        let rate = fixed::types::I64F64::from_bits(self.config.taker_fee_rate.to_raw_bits());
        let elapsed = fixed::types::I64F64::checked_from_num(time_to_maturity)
            .ok_or(StateError::ArithmeticOverflow)?;
        let year = fixed::types::I64F64::checked_from_num(SECONDS_PER_YEAR)
            .ok_or(StateError::ArithmeticOverflow)?;
        // Premultiply both decimal scales before annual division so a small
        // fee is not truncated at Q64.64 precision. The conversion below
        // removes tick scale and normalizes the underlying atoms once.
        let market_scale = decimal_scale(self.config.tick_scale)?;
        let settlement_scale = decimal_scale(self.config.settlement_decimals)?;
        let maturity_scaled_rate = rate
            .checked_mul(elapsed)
            .and_then(|value| value.checked_mul(market_scale))
            .and_then(|value| value.checked_mul(settlement_scale))
            .and_then(|value| value.checked_div(year))
            .ok_or(StateError::ArithmeticOverflow)?;
        fixed_perp_math::rate::FeeRate::try_from_i64f64(maturity_scaled_rate)
            .map_err(|_| StateError::InvalidConfiguration)?
            .settlement_fee_atoms_scaled(
                u128::from(size),
                self.config.underlying_decimals,
                0,
                self.config.tick_scale,
            )
            .map_err(|_| StateError::ArithmeticOverflow)
    }

    /// Splits one fill's fixed payment and taker fee into the two cash legs.
    ///
    /// The maker is credited `fixed_payment_atoms`. The taker is debited that
    /// amount and the fee. The booked fee is `fee_atoms`. On success the three
    /// legs sum to zero.
    pub fn split_fill_cash(
        fixed_payment_atoms: i128,
        fee_atoms: u128,
    ) -> StateResult<(i128, i128, i128)> {
        let payment_bits = fixed_payment_atoms as u128;
        let (taker, maker, fee) = Self::split_fill_cash_limbs(
            payment_bits as u64,
            (payment_bits >> 64) as u64,
            fee_atoms as u64,
            (fee_atoms >> 64) as u64,
        )
        .ok_or(StateError::ArithmeticOverflow)?;
        Ok((
            Self::i128_from_limbs(taker.0, taker.1),
            Self::i128_from_limbs(maker.0, maker.1),
            Self::i128_from_limbs(fee.0, fee.1),
        ))
    }

    fn i128_from_limbs(low: u64, high: u64) -> i128 {
        ((u128::from(high) << 64) | u128::from(low)) as i128
    }

    fn signed_fee_limbs(low: u64, high: u64) -> Option<(u64, u64)> {
        if high >= 1u64 << 63 {
            None
        } else {
            Some((low, high))
        }
    }

    fn checked_neg_limbs(low: u64, high: u64) -> Option<(u64, u64)> {
        if low == 0 && high == 1u64 << 63 {
            return None;
        }
        let carry = u64::from(low == 0);
        let negated_low = (!low).wrapping_add(1);
        let negated_high = (!high).wrapping_add(carry);
        Some((negated_low, negated_high))
    }

    fn checked_sub_limbs(
        left_low: u64,
        left_high: u64,
        right_low: u64,
        right_high: u64,
    ) -> Option<(u64, u64)> {
        let borrow = u64::from(left_low < right_low);
        let low = left_low.wrapping_sub(right_low);
        let high = left_high.wrapping_sub(right_high).wrapping_sub(borrow);
        let left_negative = left_high & (1u64 << 63) != 0;
        let right_negative = right_high & (1u64 << 63) != 0;
        let result_negative = high & (1u64 << 63) != 0;
        if left_negative != right_negative && left_negative != result_negative {
            None
        } else {
            Some((low, high))
        }
    }

    /// Same split as `split_fill_cash`, in the two 64-bit words of each value.
    ///
    /// Each pair is `(low, high)` in two's complement. The fee's high word must
    /// fit in a signed 128-bit value. Negating `i128::MIN`, or a subtraction
    /// that leaves that range, returns `None`.
    pub fn split_fill_cash_limbs(
        payment_low: u64,
        payment_high: u64,
        fee_low: u64,
        fee_high: u64,
    ) -> Option<((u64, u64), (u64, u64), (u64, u64))> {
        let fee = Self::signed_fee_limbs(fee_low, fee_high)?;
        let negated = Self::checked_neg_limbs(payment_low, payment_high)?;
        let taker = Self::checked_sub_limbs(negated.0, negated.1, fee.0, fee.1)?;
        Some((taker, (payment_low, payment_high), fee))
    }

    /// Applies a validated best-head fill only after every arithmetic result is known.
    /// Position index snapshots remain unchanged for the caller's final reset step.
    fn apply_maker_match_and_taker_fill(
        &mut self,
        taker: &mut Subaccount,
        market_slot_index: usize,
        order_node_index: usize,
        taker_fill: i64,
        fixed_payment_atoms: i128,
        fee_atoms: u128,
    ) -> StateResult<()> {
        // Maker and taker receive equal amounts on opposite sides of the trade.
        let maker_fill = taker_fill
            .checked_neg()
            .ok_or(StateError::ArithmeticOverflow)?;
        let (cash_delta, maker_fixed_delta, _) =
            Self::split_fill_cash(fixed_payment_atoms, fee_atoms)?;
        let (next_remaining, next_pending_floating, next_pending_fee, next_pending_payment) = {
            let node = &self.order_book.nodes[order_node_index];
            let next_remaining = node
                .remaining_size
                .checked_sub(maker_fill)
                .ok_or(StateError::ArithmeticOverflow)?;
            // Preserve each maker fill at its fill-time indices until the maker is materialized.
            let floating_delta = i128::from(maker_fill)
                .checked_mul(self.floating_index.to_raw_bits())
                .ok_or(StateError::ArithmeticOverflow)?;
            let next_pending_floating = node
                .pending_floating_index_basis
                .to_raw_bits()
                .checked_add(floating_delta)
                .ok_or(StateError::ArithmeticOverflow)?;
            let fee_delta = i128::from(maker_fill.unsigned_abs())
                .checked_mul(self.fee_index.to_raw_bits())
                .ok_or(StateError::ArithmeticOverflow)?;
            let next_pending_fee = node
                .pending_fee_index_basis
                .to_raw_bits()
                .checked_add(fee_delta)
                .ok_or(StateError::ArithmeticOverflow)?;
            let next_pending_payment = node
                .pending_fixed_payment_atoms
                .checked_add(maker_fixed_delta)
                .ok_or(StateError::ArithmeticOverflow)?;
            (
                next_remaining,
                next_pending_floating,
                next_pending_fee,
                next_pending_payment,
            )
        };
        // The absent maker may be reducing exposure, so reserve its full matched amount
        // conservatively until its subaccount is available for exact materialization.
        let next_reserve = self
            .pending_maker_oi_reserve
            .checked_add(u128::from(maker_fill.unsigned_abs()))
            .ok_or(StateError::ArithmeticOverflow)?;
        let slot = &taker.markets[market_slot_index];
        let next_position = slot
            .position_size
            .checked_add(taker_fill)
            .ok_or(StateError::ArithmeticOverflow)?;
        let next_cash = taker.markets[market_slot_index]
            .settlement_balance_atoms
            .checked_add(cash_delta)
            .ok_or(StateError::ArithmeticOverflow)?;
        let next_materialized = self
            .materialized_gross_open_interest
            .checked_sub(u128::from(slot.position_size.unsigned_abs()))
            .and_then(|value| value.checked_add(u128::from(next_position.unsigned_abs())))
            .ok_or(StateError::Conflict)?;
        let capped = next_materialized
            .checked_add(next_reserve)
            .ok_or(StateError::ArithmeticOverflow)?;
        if capped > self.config.hard_gross_open_interest_cap_atoms {
            return Err(StateError::Conflict);
        }
        let next_accrued_fee = self
            .accrued_protocol_fee_atoms
            .checked_add(fee_atoms)
            .ok_or(StateError::ArithmeticOverflow)?;

        // A fully matched node no longer provides liquidity. Detach it so later matching
        // does not traverse it again, but retain its pending fields until maker materialization.
        if next_remaining == 0 {
            self.order_book.detach_from_fifo(order_node_index)?;
        }
        // Commit only after every fallible arithmetic and cap check above has succeeded.
        let node = &mut self.order_book.nodes[order_node_index];
        node.remaining_size = next_remaining;
        node.pending_floating_index_basis = FixedPoint::from_raw_bits(next_pending_floating);
        node.pending_fee_index_basis = FixedPoint::from_raw_bits(next_pending_fee);
        node.pending_fixed_payment_atoms = next_pending_payment;
        self.pending_maker_oi_reserve = next_reserve;
        self.materialized_gross_open_interest = next_materialized;
        taker.markets[market_slot_index].settlement_balance_atoms = next_cash;
        taker.markets[market_slot_index].position_size = next_position;
        self.accrued_protocol_fee_atoms = next_accrued_fee;
        Ok(())
    }

    /// Returns materialized plus reserved OI and verifies the persisted cap invariant.
    pub fn capped_open_interest(&self) -> StateResult<u128> {
        let capped = self
            .materialized_gross_open_interest
            .checked_add(self.pending_maker_oi_reserve)
            .ok_or(StateError::ArithmeticOverflow)?;
        if capped > self.config.hard_gross_open_interest_cap_atoms {
            return Err(StateError::Conflict);
        }
        Ok(capped)
    }

    /// Replaces a maker's pending OI reserve with its exact position contribution.
    pub fn materialize_pending_maker_oi(
        &mut self,
        reserved_amount: u128,
        old_position_abs: u128,
        new_position_abs: u128,
    ) -> StateResult<()> {
        // An executed fill is settled even when the resulting position is
        // outside the bounds that govern new orders. Those bounds still apply
        // before maturity.
        if !self.is_matured()
            && new_position_abs != 0
            && (new_position_abs < u128::from(self.config.minimum_position_size_atoms)
                || new_position_abs > u128::from(self.config.maximum_position_size_atoms))
        {
            return Err(StateError::Conflict);
        }
        // Pending fills reserve their full absolute size. Materialization
        // replaces that conservative reserve with the exact position OI delta.
        let exact_delta = new_position_abs.abs_diff(old_position_abs);
        // Exact replacement must not turn a bounded reservation into extra OI.
        if exact_delta > reserved_amount {
            return Err(StateError::Conflict);
        }
        let after_old = self
            .materialized_gross_open_interest
            .checked_sub(old_position_abs)
            .ok_or(StateError::Conflict)?;
        let next_materialized = after_old
            .checked_add(new_position_abs)
            .ok_or(StateError::ArithmeticOverflow)?;
        let next_reserve = self
            .pending_maker_oi_reserve
            .checked_sub(reserved_amount)
            .ok_or(StateError::Conflict)?;
        let next_capped = next_materialized
            .checked_add(next_reserve)
            .ok_or(StateError::ArithmeticOverflow)?;
        let current_capped = self
            .materialized_gross_open_interest
            .checked_add(self.pending_maker_oi_reserve)
            .ok_or(StateError::ArithmeticOverflow)?;
        // A cap cut must not trap a fill that was already reserved. Maturity
        // can replace that reserve as long as capped open interest does not grow.
        if next_capped > self.config.hard_gross_open_interest_cap_atoms
            && !(self.is_matured() && next_capped <= current_capped)
        {
            return Err(StateError::Conflict);
        }
        self.materialized_gross_open_interest = next_materialized;
        self.pending_maker_oi_reserve = next_reserve;
        Ok(())
    }

    /// Returns the position value in settlement atoms using the canonical
    /// price-weighted mark and one annualized time factor.
    ///
    /// Positive values round down and negative values round further down,
    /// mirroring `PaymentLib.calcPositionValue`'s `rawDivFloor`.
    pub fn position_value(
        &self,
        position_size: i64,
        mark_rate: FixedPoint,
        time_to_maturity: i64,
    ) -> StateResult<i128> {
        if time_to_maturity < 0 {
            return Err(StateError::InvalidClock);
        }
        Rate::from_i64f64(mark_rate.to_i64f64())
            .fixed_payment_atoms_scaled(
                i128::from(position_size),
                time_to_maturity,
                SECONDS_PER_YEAR,
                self.config.underlying_decimals,
                self.config.settlement_decimals,
                self.config.tick_scale,
            )
            .map_err(|_| StateError::ArithmeticOverflow)
            .and_then(|value| value.checked_neg().ok_or(StateError::ArithmeticOverflow))
    }

    /// Returns the conservative-ceiling initial margin in settlement atoms:
    /// `ceil(risk_principal * initial_margin_factor *
    /// max(time_to_maturity, margin_time_threshold) / one_year)`.
    pub fn initial_margin(
        &self,
        risk_principal: FixedPoint,
        time_to_maturity: i64,
    ) -> StateResult<i128> {
        self.ceiling_margin(
            risk_principal,
            self.config.initial_margin_factor,
            time_to_maturity,
        )
    }

    /// Returns the conservative-ceiling maintenance margin in settlement atoms:
    /// `ceil(risk_principal * maintenance_margin_factor *
    /// max(time_to_maturity, margin_time_threshold) / one_year)`.
    pub fn maintenance_margin(
        &self,
        risk_principal: FixedPoint,
        time_to_maturity: i64,
    ) -> StateResult<i128> {
        self.ceiling_margin(
            risk_principal,
            self.config.maintenance_margin_factor,
            time_to_maturity,
        )
    }

    /// Returns the aggregated risk principal: the Q64.64
    /// potential margin combining the settled position and live pending-order
    /// risk.
    ///
    /// Live orders contribute to strict IM risk: same-side orders add to the
    /// position's own principal, opposite-side orders net against it up to the
    /// position size, and the heavier side wins.
    pub fn risk_principal(
        &self,
        position_size: i64,
        order_risk: &super::OrderRiskAggregate,
        mark_rate: FixedPoint,
    ) -> StateResult<FixedPoint> {
        let position_abs = position_size.unsigned_abs();
        let pm_alone = FixedPoint::from_raw_bits(super::OrderRiskAggregate::pm_contribution(
            position_abs,
            mark_rate,
            self.config.rate_floor,
            self.order_risk_scale(),
        )?);
        let total_long = Self::side_pm_total(
            position_size,
            order_risk.sum_long_order_size,
            order_risk.sum_long_order_pm,
            pm_alone,
        )?;
        let total_short = Self::side_pm_total(
            -position_size,
            order_risk.sum_short_order_size,
            order_risk.sum_short_order_pm,
            pm_alone,
        )?;
        if total_long.to_raw_bits() >= total_short.to_raw_bits() {
            Ok(total_long)
        } else {
            Ok(total_short)
        }
    }

    /// Aggregates one order side against the settled position exactly as Boros
    /// does: an opposite-side order book fully covered by the position carries
    /// no marginal risk, same-side orders add their principal, and a netted
    /// book only keeps the excess above the position's own principal.
    fn side_pm_total(
        signed_position: i64,
        order_size: u64,
        order_pm: FixedPoint,
        pm_alone: FixedPoint,
    ) -> StateResult<FixedPoint> {
        let position_abs = signed_position.unsigned_abs();
        if signed_position < 0 && order_size <= position_abs {
            return Ok(FixedPoint::from_raw_bits(0));
        }
        if signed_position > 0 {
            let sum = order_pm
                .to_raw_bits()
                .checked_add(pm_alone.to_raw_bits())
                .ok_or(StateError::ArithmeticOverflow)?;
            return Ok(FixedPoint::from_raw_bits(sum));
        }
        if order_pm.to_raw_bits() > pm_alone.to_raw_bits() {
            let excess = order_pm
                .to_raw_bits()
                .checked_sub(pm_alone.to_raw_bits())
                .ok_or(StateError::ArithmeticOverflow)?;
            Ok(FixedPoint::from_raw_bits(excess))
        } else {
            Ok(FixedPoint::from_raw_bits(0))
        }
    }

    /// Conservative-ceiling margin formula shared by IM and MM:
    /// `ceil(risk_principal * factor * max(time, threshold) / one_year)`.
    fn ceiling_margin(
        &self,
        risk_principal: FixedPoint,
        factor: FixedPoint,
        time_to_maturity: i64,
    ) -> StateResult<i128> {
        if time_to_maturity < 0 {
            return Err(StateError::InvalidClock);
        }
        let duration = I64F64::checked_from_num(time_to_maturity)
            .ok_or(StateError::ArithmeticOverflow)?
            .max(
                I64F64::checked_from_num(i64::from(self.config.margin_time_threshold_seconds))
                    .ok_or(StateError::ArithmeticOverflow)?,
            );
        let year =
            I64F64::checked_from_num(SECONDS_PER_YEAR).ok_or(StateError::ArithmeticOverflow)?;
        let unrounded = risk_principal
            .to_i64f64()
            .checked_mul(factor.to_i64f64())
            .and_then(|value| value.checked_mul(duration))
            .and_then(|value| value.checked_div(year))
            .ok_or(StateError::ArithmeticOverflow)?;
        unrounded
            .checked_ceil()
            .ok_or(StateError::ArithmeticOverflow)?
            .to_bits()
            .checked_shr(I64F64::FRAC_NBITS)
            .ok_or(StateError::ArithmeticOverflow)
    }

    /// Returns the single authoritative post-batch aggregate closing-action
    /// classification and its strict-IM decision.
    ///
    /// Inputs are the pre-batch settled position and order risk, the
    /// post-batch final position and final aggregate order risk (whose
    /// `sum_long_order_size`/`sum_short_order_size` carry the final aggregate
    /// order sizes), the side of any newly resting order, the close-only exemption, the
    /// account's validated critical-health ratio, the net payment including
    /// fees `P`, the worst rate among newly resting closing orders, and the
    /// current clock. Callers must have materialized every pending fill
    /// first: `final_value` is a realized position valuation only, and resting
    /// orders enter the final margin risk separately.
    ///
    /// An exposure-increasing action always requires strict IM. A genuinely
    /// closing action bypasses strict IM only when the Boros closing-order
    /// mark-displacement bound holds and `V_pre - V_post - P <=
    /// critical_health_ratio * (MM_pre - MM_post)`. The close-only exemption
    /// bypasses the market close-only gate but never strict IM.
    #[allow(clippy::too_many_arguments)]
    pub fn closing_action(
        &self,
        pre_position_size: i64,
        pre_order_risk: &super::OrderRiskAggregate,
        post_position_size: i64,
        post_order_risk: &super::OrderRiskAggregate,
        new_resting_order_side: Option<Side>,
        close_only_exempt: bool,
        critical_health_ratio: FixedPoint,
        payment_atoms: i128,
        worst_closing_rate: Option<FixedPoint>,
        now: i64,
    ) -> StateResult<ClosingAction> {
        let mark = self.current_mark_rate(now)?;
        let time_to_maturity = self
            .config
            .maturity_timestamp
            .checked_sub(now)
            .ok_or(StateError::InvalidClock)?;
        let final_value = self.position_value(post_position_size, mark, time_to_maturity)?;
        let final_risk = self.risk_principal(post_position_size, post_order_risk, mark)?;
        let final_margin = self.maintenance_margin(final_risk, time_to_maturity)?;
        let is_closing_action = aggregate_is_closing_action(
            pre_position_size,
            post_position_size,
            post_order_risk.sum_long_order_size,
            post_order_risk.sum_short_order_size,
            new_resting_order_side,
        );
        let mut requires_strict_im = true;
        if is_closing_action {
            // Boros `_checkMargin` closing-order mark displacement: the
            // proposed worst closing rate must stay within
            // `closing_margin_relief_bound_factor * max(abs(mark), rate_floor)`
            // of the mark, signed by the final position side.
            if new_resting_order_side.is_some() {
                let worst = worst_closing_rate.ok_or(StateError::InvalidConfiguration)?;
                let base = mark
                    .to_i64f64()
                    .checked_abs()
                    .ok_or(StateError::ArithmeticOverflow)?
                    .max(I64F64::from_bits(self.config.rate_floor.to_raw_bits()));
                let bound = base
                    .checked_mul(self.config.closing_margin_relief_bound_factor.to_i64f64())
                    .ok_or(StateError::ArithmeticOverflow)?;
                let sign = I64F64::checked_from_num(post_position_size.signum())
                    .ok_or(StateError::ArithmeticOverflow)?;
                let displacement = mark
                    .to_i64f64()
                    .checked_sub(worst.to_i64f64())
                    .and_then(|delta| delta.checked_mul(sign))
                    .ok_or(StateError::ArithmeticOverflow)?;
                if displacement > bound {
                    return Ok(ClosingAction {
                        is_closing_action,
                        closing_status_allowed: is_closing_action || close_only_exempt,
                        requires_strict_im: true,
                        final_value,
                        final_margin,
                    });
                }
            }
            // Margin relief: `value_consumed = V_pre - V_post - P` must stay
            // at most `critical_health_ratio * (MM_pre - MM_post)`, with the
            // ratio product floored as Boros `mulFloor` does. The close-only
            // exemption never enters this relief decision.
            let pre_value = self.position_value(pre_position_size, mark, time_to_maturity)?;
            let pre_risk = self.risk_principal(pre_position_size, pre_order_risk, mark)?;
            let pre_margin = self.maintenance_margin(pre_risk, time_to_maturity)?;
            let value_consumed = pre_value
                .checked_sub(final_value)
                .and_then(|value| value.checked_sub(payment_atoms))
                .ok_or(StateError::ArithmeticOverflow)?;
            let mm_reduction = pre_margin
                .checked_sub(final_margin)
                .ok_or(StateError::ArithmeticOverflow)?;
            let allowed = critical_health_ratio
                .checked_mul_int(mm_reduction)
                .map_err(|_| StateError::ArithmeticOverflow)?
                .floor_atoms()
                .map_err(|_| StateError::ArithmeticOverflow)?;
            requires_strict_im = value_consumed > allowed;
        }
        Ok(ClosingAction {
            is_closing_action,
            closing_status_allowed: is_closing_action || close_only_exempt,
            requires_strict_im,
            final_value,
            final_margin,
        })
    }
}

/// Result of the single authoritative post-batch aggregate closing-action
/// classification and its strict-IM decision.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClosingAction {
    /// Genuinely closing under the aggregate position and order-capacity
    /// rules. A close-only exemption never changes this value.
    pub is_closing_action: bool,
    /// Whether the market close-only gate admits this action:
    /// `is_closing_action || close_only_exempt`. The exemption bypasses the
    /// status gate but never strict IM.
    pub closing_status_allowed: bool,
    /// Whether strict initial margin is required. Exposure-increasing actions
    /// always require it; a genuinely closing action bypasses it only when the
    /// Boros closing-rate bound and the margin-relief condition both hold.
    pub requires_strict_im: bool,
    /// Post-batch realized position value `V_post`.
    pub final_value: i128,
    /// Post-batch maintenance margin `MM_post` including final order risk.
    pub final_margin: i128,
}

/// Returns whether a post-batch aggregate action is genuinely closing.
///
/// The realized position must not grow or flip sign; the final resting order
/// capacity must be bounded by the final position (a zero position permits no
/// resting orders); and a new nonzero same-side resting order is rejected while a
/// pre-existing same-side order alone never disqualifies. This is the single
/// authoritative position/order classification shared by placement,
/// modification, matching, batch, status, and cleanup callers; `closing_action`
/// adds the post-batch value/margin and the strict-IM relief decision.
pub fn aggregate_is_closing_action(
    pre_position: i64,
    post_position: i64,
    final_long_order_size: u64,
    final_short_order_size: u64,
    new_resting_order_side: Option<Side>,
) -> bool {
    if post_position.unsigned_abs() > pre_position.unsigned_abs()
        || (post_position != 0
            && pre_position != 0
            && post_position.signum() != pre_position.signum())
    {
        return false;
    }
    let final_sign = post_position.signum();
    match final_sign {
        0 => {
            if final_long_order_size != 0 || final_short_order_size != 0 {
                return false;
            }
        }
        1 => {
            if final_short_order_size > post_position.unsigned_abs() {
                return false;
            }
        }
        _ => {
            if final_long_order_size > post_position.unsigned_abs() {
                return false;
            }
        }
    }
    if let Some(side) = new_resting_order_side {
        // This check mainly enforces cases the above doesn't cover because it doesn't detect
        // match whether or not a match occurs.
        if final_sign == 0
            || (final_sign > 0 && side == Side::Long)
            || (final_sign < 0 && side == Side::Short)
        {
            return false;
        }
    }
    true
}

/// Aggregated portfolio health inputs for one subaccount across its entered
/// markets.
///
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PortfolioHealth {
    /// Adjusted collateral and settlement balances in the common numeraire.
    pub adjusted_total_assets: i128,
    /// Signed position value in the common numeraire.
    pub total_value: i128,
    /// Selected margin requirement, valued in the portfolio numeraire.
    pub required_margin: MarginRequirement,
}

/// Identifies which margin was calculated and carries its portfolio value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MarginRequirement {
    Initial(i128),
    Maintenance(i128),
}

impl Default for MarginRequirement {
    fn default() -> Self {
        Self::Initial(0)
    }
}

/// Selects which margin requirement portfolio aggregation computes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MarginRequirementType {
    Initial,
    Maintenance,
}

impl PortfolioHealth {
    /// Checks initial-margin health and rejects a maintenance-margin aggregate.
    pub fn strict_im_met(&self) -> StateResult<bool> {
        let MarginRequirement::Initial(required_margin) = self.required_margin else {
            return Err(StateError::Conflict);
        };
        let available = self
            .adjusted_total_assets
            .checked_add(self.total_value)
            .ok_or(StateError::ArithmeticOverflow)?;
        Ok(available >= required_margin)
    }

    /// Checks critical health from a maintenance-margin aggregate.
    pub fn critical_health_met(&self, critical_health_ratio: FixedPoint) -> StateResult<bool> {
        let MarginRequirement::Maintenance(required_margin) = self.required_margin else {
            return Err(StateError::Conflict);
        };
        let required = critical_health_ratio
            .checked_mul_int(required_margin)
            .map_err(|_| StateError::ArithmeticOverflow)?
            .ceil_atoms()
            .map_err(|_| StateError::ArithmeticOverflow)?;
        let available = self
            .adjusted_total_assets
            .checked_add(self.total_value)
            .ok_or(StateError::ArithmeticOverflow)?;
        Ok(available >= required)
    }

    /// Checks risky-order eligibility from a maintenance-margin aggregate.
    ///
    /// The risky threshold is an earlier containment boundary than critical
    /// health and never blocks deposits, settlement, materialization,
    /// cancellation, or genuine reductions.
    pub fn risky_order_cleanup_eligible(
        &self,
        risky_order_cancellation_ratio: FixedPoint,
    ) -> StateResult<bool> {
        let MarginRequirement::Maintenance(required_margin) = self.required_margin else {
            return Err(StateError::Conflict);
        };
        let threshold = risky_order_cancellation_ratio
            .checked_mul_int(required_margin)
            .map_err(|_| StateError::ArithmeticOverflow)?
            .ceil_atoms()
            .map_err(|_| StateError::ArithmeticOverflow)?;
        let available = self
            .adjusted_total_assets
            .checked_add(self.total_value)
            .ok_or(StateError::ArithmeticOverflow)?;
        Ok(available < threshold)
    }

    /// Boros health ratio `value / maintenance`, accepted only on `[0, 1)`.
    /// Negative value is bad debt and is not liquidatable here.
    pub fn liquidation_health_ratio(&self) -> StateResult<FixedPoint> {
        let MarginRequirement::Maintenance(maintenance) = self.required_margin else {
            return Err(StateError::Conflict);
        };
        if maintenance <= 0 {
            return Err(StateError::InvalidStatus);
        }
        let available = self
            .adjusted_total_assets
            .checked_add(self.total_value)
            .ok_or(StateError::ArithmeticOverflow)?;
        if available < 0 || available >= maintenance {
            return Err(StateError::InvalidStatus);
        }
        self.maintenance_health_ratio()
    }

    /// Portfolio maintenance ratio `value / maintenance`, including ratios
    /// outside `[0, 1)`. Deleveraging compares these directly.
    pub fn maintenance_health_ratio(&self) -> StateResult<FixedPoint> {
        let MarginRequirement::Maintenance(maintenance) = self.required_margin else {
            return Err(StateError::Conflict);
        };
        if maintenance <= 0 {
            return Err(StateError::InvalidStatus);
        }
        let available = self
            .adjusted_total_assets
            .checked_add(self.total_value)
            .ok_or(StateError::ArithmeticOverflow)?;
        let ratio = I64F64::checked_from_num(available)
            .and_then(|value| value.checked_div(I64F64::checked_from_num(maintenance)?))
            .ok_or(StateError::ArithmeticOverflow)?;
        Ok(FixedPoint::from_i64f64(ratio))
    }
}

impl MarginGroup {
    /// Ensures the group and market reference the same collateral and
    /// settlement AssetState accounts.
    pub fn ensure_market_assets_match(&self, market: &MarketState) -> StateResult<()> {
        if self.collateral_state != market.config.collateral_state
            || self.settlement_state != market.config.settlement_state
        {
            return Err(StateError::Conflict);
        }
        Ok(())
    }

    /// Ensures the subaccount belongs to this group.
    pub fn ensure_subaccount_matches(&self, subaccount: &Subaccount) -> StateResult<()> {
        if self.group_id != subaccount.group_id {
            return Err(StateError::Conflict);
        }
        Ok(())
    }

    /// Initializes immutable group identity, AssetState references, and health
    /// thresholds from caller input.
    ///
    /// Membership is established by creating the discriminator-only
    /// `MarketGroupMembership` account per market, not by a group-held list.
    pub fn initialize_in_place(
        &mut self,
        bump: u8,
        group_id: u16,
        collateral_state: Pubkey,
        settlement_state: Pubkey,
        governance_key: Pubkey,
        critical_health_ratio: FixedPoint,
        risky_order_cancellation_ratio: FixedPoint,
    ) -> StateResult<()> {
        if group_id == 0
            || collateral_state == Pubkey::default()
            || settlement_state == Pubkey::default()
            || governance_key == Pubkey::default()
            || critical_health_ratio.to_raw_bits() <= 0
            || risky_order_cancellation_ratio.to_raw_bits() < critical_health_ratio.to_raw_bits()
        {
            return Err(StateError::InvalidConfiguration);
        }
        self.group_id = group_id;
        self.bump = bump;
        self.status = GROUP_PENDING;
        self.collateral_state = collateral_state;
        self.settlement_state = settlement_state;
        self.governance_key = governance_key;
        self.critical_health_ratio = critical_health_ratio;
        self.risky_order_cancellation_ratio = risky_order_cancellation_ratio;
        self.market_count = 0;
        self.subaccount_count = 0;
        let one = FixedPoint::from_raw_bits(1i128 << 64);
        self.collateral_factors = ValuationFactors {
            discount_factor: one,
            liability_factor: one,
        };
        self.settlement_factors = self.collateral_factors;
        self._pad_0 = [0; 4];
        self.reserved = [0; MARGIN_GROUP_RESERVED];
        Ok(())
    }

    /// Activates a pending group inside its coordinator-supplied window.
    pub fn activate(
        &mut self,
        activation_window: &mut super::ProposalWindow,
        now: i64,
    ) -> StateResult<()> {
        if self.status != GROUP_PENDING {
            return Err(StateError::InvalidStatus);
        }

        activation_window.consume(now)?;

        self.status = GROUP_ACTIVE;

        Ok(())
    }

    /// Computes portfolio health from the transient AssetState observations
    /// supplied by the caller while preserving the same sign-directed
    /// arithmetic.
    #[allow(clippy::too_many_arguments)]
    pub fn initial_portfolio_health_with_assets<'a>(
        &self,
        subaccount: &Subaccount,
        collateral_asset: &AssetState,
        collateral_observation: OracleObservation,
        settlement_asset: &AssetState,
        settlement_observation: OracleObservation,
        markets: impl IntoIterator<Item = PortfolioMarketObservation<'a>>,
        now: i64,
    ) -> StateResult<PortfolioHealth> {
        self.portfolio_health_with_assets(
            subaccount,
            collateral_asset,
            collateral_observation,
            settlement_asset,
            settlement_observation,
            markets,
            now,
            MarginRequirementType::Initial,
        )
    }

    /// Maintenance-margin form of [`Self::initial_portfolio_health_with_assets`].
    #[allow(clippy::too_many_arguments)]
    pub fn maintenance_portfolio_health_with_assets<'a>(
        &self,
        subaccount: &Subaccount,
        collateral_asset: &AssetState,
        collateral_observation: OracleObservation,
        settlement_asset: &AssetState,
        settlement_observation: OracleObservation,
        markets: impl IntoIterator<Item = PortfolioMarketObservation<'a>>,
        now: i64,
    ) -> StateResult<PortfolioHealth> {
        self.portfolio_health_with_assets(
            subaccount,
            collateral_asset,
            collateral_observation,
            settlement_asset,
            settlement_observation,
            markets,
            now,
            MarginRequirementType::Maintenance,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn portfolio_health_with_assets<'a>(
        &self,
        subaccount: &Subaccount,
        collateral_asset: &AssetState,
        collateral_observation: OracleObservation,
        settlement_asset: &AssetState,
        settlement_observation: OracleObservation,
        markets: impl IntoIterator<Item = PortfolioMarketObservation<'a>>,
        now: i64,
        requirement_type: MarginRequirementType,
    ) -> StateResult<PortfolioHealth> {
        let mut markets_by_slot: [Option<PortfolioMarketObservation<'a>>; super::MAX_MARKETS] =
            [None; super::MAX_MARKETS];
        for observation in markets {
            let Some(slot) = subaccount.lookup_market(observation.market.config.market_id) else {
                continue;
            };
            markets_by_slot[slot] = Some(observation);
        }

        let collateral_adjusted = self
            .collateral_factors
            .apply(subaccount.collateral_balance_atoms)?;
        let collateral_endpoint = PriceEndpoint::for_signed_value(collateral_adjusted);
        let mut assets = collateral_observation
            .conservative_price_for(collateral_asset, collateral_endpoint)?
            .scaled_value_atoms(collateral_adjusted, collateral_asset.decimals)
            .map_err(|_| StateError::ArithmeticOverflow)?;
        let mut total_value = 0i128;
        let mut total_margin = 0i128;

        for (slot_index, slot) in subaccount.markets.iter().enumerate() {
            if slot.market_id == 0 {
                continue;
            }
            let observation = markets_by_slot[slot_index].ok_or(StateError::NotFound)?;
            let market = observation.market;
            let underlying_asset = observation.underlying_asset;
            observation.underlying.validate(
                now,
                underlying_asset.max_age_seconds,
                underlying_asset.max_confidence_ratio,
                underlying_asset.max_ema_deviation,
            )?;
            let time = market
                .config
                .maturity_timestamp
                .checked_sub(now)
                .ok_or(StateError::InvalidClock)?;
            let mark = market.current_mark_rate(now)?;
            let position_value = market.position_value(slot.position_size, mark, time)?;
            let position_value = market.position_factors.apply(position_value)?;

            // The source observation is an underlying/settlement price.  The
            // fixed-leg value is scaled by that cross-rate exactly once and
            // then conservatively rounded at the settlement-atom boundary.
            let position_endpoint = PriceEndpoint::for_signed_value(position_value);
            let underlying_price = observation
                .underlying
                .conservative_price_for(underlying_asset, position_endpoint)?;
            let settlement_price = settlement_observation
                .conservative_price_for(settlement_asset, position_endpoint)?;
            let cross_rate = underlying_price
                .to_i64f64()
                .checked_div(settlement_price.to_i64f64())
                .ok_or(StateError::ArithmeticOverflow)?;
            let position_value = FixedPoint::from_i64f64(cross_rate)
                .checked_mul_int(position_value)
                .map_err(|_| StateError::ArithmeticOverflow)?
                .floor_atoms()
                .map_err(|_| StateError::ArithmeticOverflow)?;
            let position_value = self.settlement_factors.apply(position_value)?;
            let position_endpoint = PriceEndpoint::for_signed_value(position_value);
            let settlement_price = settlement_observation
                .conservative_price_for(settlement_asset, position_endpoint)?;
            total_value = total_value
                .checked_add(
                    settlement_price
                        .scaled_value_atoms(position_value, settlement_asset.decimals)
                        .map_err(|_| StateError::ArithmeticOverflow)?,
                )
                .ok_or(StateError::ArithmeticOverflow)?;

            let balance = self
                .settlement_factors
                .apply(slot.settlement_balance_atoms)?;
            let balance_endpoint = PriceEndpoint::for_signed_value(balance);
            let balance_price = settlement_observation
                .conservative_price_for(settlement_asset, balance_endpoint)?;
            assets = assets
                .checked_add(
                    balance_price
                        .scaled_value_atoms(balance, settlement_asset.decimals)
                        .map_err(|_| StateError::ArithmeticOverflow)?,
                )
                .ok_or(StateError::ArithmeticOverflow)?;

            let risk = market.risk_principal(slot.position_size, &slot.order_risk, mark)?;
            let margin = match requirement_type {
                MarginRequirementType::Initial => market.initial_margin(risk, time)?,
                MarginRequirementType::Maintenance => market.maintenance_margin(risk, time)?,
            };
            let margin = self.settlement_factors.apply(-margin)?;
            let margin_endpoint = PriceEndpoint::for_signed_value(margin);
            let margin_price =
                settlement_observation.conservative_price_for(settlement_asset, margin_endpoint)?;
            total_margin = total_margin
                .checked_add(
                    margin_price
                        .scaled_value_atoms(margin, settlement_asset.decimals)
                        .map_err(|_| StateError::ArithmeticOverflow)?
                        .checked_neg()
                        .ok_or(StateError::ArithmeticOverflow)?,
                )
                .ok_or(StateError::ArithmeticOverflow)?;
        }

        let required_margin = match requirement_type {
            MarginRequirementType::Initial => MarginRequirement::Initial(total_margin),
            MarginRequirementType::Maintenance => MarginRequirement::Maintenance(total_margin),
        };
        Ok(PortfolioHealth {
            adjusted_total_assets: assets,
            total_value,
            required_margin,
        })
    }

    /// Cancels all unfilled orders for every entered market that has live
    /// references, using the caller-supplied market states.
    pub fn close_all_orders(
        subaccount: &mut Subaccount,
        markets: &mut [Option<&mut MarketState>],
    ) -> StateResult<()> {
        for slot_index in 0..super::MAX_MARKETS {
            let (market_id, has_orders) = {
                let slot = &subaccount.markets[slot_index];
                (
                    slot.market_id,
                    slot.order_risk.sum_long_order_size != 0
                        || slot.order_risk.sum_short_order_size != 0,
                )
            };
            if market_id == 0 || !has_orders {
                continue;
            }
            let market = markets
                .iter_mut()
                .flatten()
                .find(|market| market.config.market_id == market_id)
                .ok_or(StateError::InvalidReference)?;
            market.close_all_orders(subaccount, slot_index)?;
        }
        Ok(())
    }

    /// Dispatches one bounded sweep across the supplied markets, touching only
    /// the slots that hold this subaccount's live references.
    ///
    /// The references are bucketed by market slot in one pass, then each
    /// supplied market that holds references dispatches its own market-local
    /// sweep. No market or book scan repeats per market, and markets without
    /// references are never touched.
    fn dispatch_cleanup(
        subaccount: &mut Subaccount,
        markets: &mut [&mut MarketState],
    ) -> StateResult<()> {
        let mut slots_with_refs = [false; super::MAX_MARKETS];
        for reference in subaccount.iter_order_refs() {
            slots_with_refs[usize::from(reference.slot_index)] = true;
        }

        for market in markets.iter_mut() {
            let Some(slot_index) = subaccount.lookup_market(market.config.market_id) else {
                continue;
            };
            if !slots_with_refs[slot_index] {
                continue;
            }
            market.close_only_cleanup(subaccount)?;
        }
        Ok(())
    }

    /// Close-only containment cleanup for this group's subaccount.
    ///
    /// Shapes the subaccount's resting orders toward a close-only-safe book:
    /// each entered market cancels exposure-increasing orders and then trims
    /// excess opposite-side closing size so that filling everything that
    /// remains closes the position without flipping. The caller owns
    /// activation; this helper performs no health or authority checks.
    /// Assumes the caller materialized every pending fill first.
    pub fn close_only_cleanup(
        subaccount: &mut Subaccount,
        markets: &mut [&mut MarketState],
    ) -> StateResult<()> {
        Self::dispatch_cleanup(subaccount, markets)
    }
}
