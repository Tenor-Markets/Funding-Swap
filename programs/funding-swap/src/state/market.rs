use anchor_lang::prelude::*;
use bytemuck::{Pod, Zeroable};
use fixed::types::I64F64;
use funding_swap_math::{rate::RateDelta, Rate};

use super::user::OrderRiskScale;
use super::{
    FixedPoint, OracleConfig, OrderBook, OrderId, OrderRef, Side, StateError, StateResult,
    Subaccount, TimeInForce, ValuationFactors, BOOK_CAPACITY, FREE_INDEX,
};

pub use funding_swap_math::rate::SECONDS_PER_YEAR;

fn decimal_scale(decimals: u8) -> StateResult<I64F64> {
    let scale =
        funding_swap_math::pow10(u32::from(decimals)).ok_or(StateError::ArithmeticOverflow)?;
    I64F64::checked_from_num(scale).ok_or(StateError::ArithmeticOverflow)
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

struct AffordableFill {
    maker_fill: i64,
    fixed_payment_atoms: i128,
    totals: MatchTotals,
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
// u128 alignment is 16 on the host and 8 on Solana. These fields name the
// existing gaps without changing either target's persisted layout.
pub const MARKET_U128_PADDING: usize = core::mem::align_of::<u128>() - 8;

/// Explicit host alignment bytes; this field occupies zero bytes on Solana.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Pod, Zeroable)]
pub struct MarketAlignmentPadding {
    pub bytes: [u8; MARKET_U128_PADDING],
}

// IDLs describe the on-chain layout, not the host's extra alignment bytes.
#[cfg(feature = "idl-build")]
impl anchor_lang::idl::build::IdlBuild for MarketAlignmentPadding {
    fn create_type() -> Option<anchor_lang::idl::types::IdlTypeDef> {
        use anchor_lang::idl::types::{
            IdlArrayLen, IdlSerialization, IdlType, IdlTypeDef, IdlTypeDefTy,
        };
        Some(IdlTypeDef {
            name: Self::get_full_path(),
            docs: vec!["Zero bytes in the Solana account layout.".into()],
            serialization: IdlSerialization::Bytemuck,
            repr: None,
            generics: vec![],
            ty: IdlTypeDefTy::Type {
                alias: IdlType::Array(Box::new(IdlType::U8), IdlArrayLen::Value(0)),
            },
        })
    }
}

pub const ADAPTER_JUPITER: u8 = 1;
pub const ADAPTER_PHOENIX: u8 = 2;
pub const GROUP_PENDING: u8 = 0;
pub const GROUP_ACTIVE: u8 = 1;
pub const GROUP_PAUSED: u8 = 2;
pub const MARKET_PENDING: u8 = 0;
pub const MARKET_ACTIVE: u8 = 1;
pub const MARKET_CLOSE_ONLY: u8 = 2;
pub const MARKET_PAUSED: u8 = 3;
pub const MARKET_MATURED: u8 = 4;
pub const MARKET_ORDERS_ONLY: u8 = 5;

/// One protocol refresh epoch.
pub const REFRESH_EPOCH_SECONDS: i64 = 3_600;
/// A live market may use an index cached for at most this long.
pub const MAX_INDEX_STALENESS_SECONDS: i64 = REFRESH_EPOCH_SECONDS;

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
    pub critical_health_ratio: FixedPoint,
    pub risky_order_cancellation_ratio: FixedPoint,
    pub market_count: u32,
    pub subaccount_count: u32,
    pub reserved: [u8; MARGIN_GROUP_RESERVED],
}

impl MarginGroup {
    pub fn is_pending(&self) -> bool {
        self.status == GROUP_PENDING
    }

    pub fn is_active(&self) -> bool {
        self.status == GROUP_ACTIVE
    }

    pub fn is_paused(&self) -> bool {
        self.status == GROUP_PAUSED
    }

    /// Pauses every user action in this group.
    pub fn pause(&mut self) -> StateResult<()> {
        if !self.is_active() {
            return Err(StateError::InvalidStatus);
        }

        self.status = GROUP_PAUSED;

        Ok(())
    }

    /// Restores a paused group to active operation.
    pub fn resume(&mut self) -> StateResult<()> {
        if !self.is_paused() {
            return Err(StateError::InvalidStatus);
        }

        self.status = GROUP_ACTIVE;

        Ok(())
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
            .ok_or(StateError::SubaccountCountUnderflow)?;

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
    pub underlying_oracle: OracleConfig,
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
    pub market_id: u64,
    pub mark_observation_window_seconds: u32,
    pub margin_time_threshold_seconds: u32,
    pub pm_cache_nonce: u32,
    /// Implicit decimal exponent for all market rate/index mantissas.
    pub tick_scale: u8,
    /// Decimal metadata for position/order/OI underlying atoms.
    pub underlying_decimals: u8,
    pub _pad_units: [u8; 2],
    pub reserved: [u8; MARKET_CONFIG_RESERVED],
    pub _pad_2: [u8; 12],
    pub _pad_3: [u8; 12],
}

#[account(zero_copy)]
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
    pub _pad_open_interest: MarketAlignmentPadding,
    pub materialized_gross_open_interest: u128,
    pub pending_maker_oi_reserve: u128,
    pub accrued_protocol_fee_atoms: u128,
    pub position_factors: ValuationFactors,
    pub status: MarketStatus,
    pub source_seeded: u8,
    pub _pad_0: [u8; 2],
    pub subaccount_count: u32,
    /// Timestamp at which maturity finalization completed.
    pub finalized_at_timestamp: i64,
    pub _pad_tail: [u8; 8],
    pub reserved: [u8; MARKET_STATE_RESERVED],
}

impl MarketState {
    pub fn finalized_at(&self) -> Option<i64> {
        (self.finalized_at_timestamp > 0).then_some(self.finalized_at_timestamp)
    }

    pub fn finalize(&mut self, finalized_at: i64) -> StateResult<()> {
        if !matches!(
            self.status.0,
            MARKET_ACTIVE | MARKET_CLOSE_ONLY | MARKET_ORDERS_ONLY
        ) {
            return Err(StateError::InvalidStatus);
        }

        if self.source_seeded != 1 {
            return Err(StateError::MarketSourceNotSeeded);
        }

        if finalized_at < self.config.maturity_timestamp {
            return Err(StateError::MaturityNotReached);
        }

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
            .ok_or(StateError::SubaccountCountUnderflow)?;

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

    /// Returns whether the market accepts new order placement.
    pub fn may_place_order(&self) -> bool {
        self.is_active() || self.is_close_only()
    }

    /// Returns whether existing orders may be canceled or reduced.
    pub fn may_cancel_order(&self) -> bool {
        matches!(
            self.status.0,
            MARKET_ACTIVE | MARKET_CLOSE_ONLY | MARKET_ORDERS_ONLY | MARKET_MATURED
        )
    }

    /// Returns whether a supplied market may be materialized or closed.
    pub fn may_touch_orders(&self) -> bool {
        !self.is_paused()
    }

    /// Returns whether the market permits only existing-order maintenance.
    pub fn is_orders_only(&self) -> bool {
        self.status.0 == MARKET_ORDERS_ONLY
    }

    /// Returns whether all user actions are blocked.
    pub fn is_paused(&self) -> bool {
        self.status.0 == MARKET_PAUSED
    }

    /// Rejects risk-sensitive use while the cached index is too old.
    pub fn ensure_index_fresh(&self, now: i64) -> StateResult<()> {
        if self.is_matured() {
            return Ok(());
        }

        if now < self.protocol_checkpoint_timestamp {
            return Err(StateError::InvalidClock);
        }

        if now - self.protocol_checkpoint_timestamp > MAX_INDEX_STALENESS_SECONDS {
            return Err(StateError::ProtocolCheckpointTooOld);
        }

        Ok(())
    }

    pub fn may_refresh(&self) -> bool {
        !self.is_pending() && !self.is_matured()
    }

    pub fn may_seed_source(&self) -> bool {
        self.is_pending() && !self.is_seeded()
    }

    /// Returns whether maturity finalization has completed for this market.
    pub fn is_matured(&self) -> bool {
        self.status.0 == MARKET_MATURED
    }

    /// USDC fees become withdrawable only after maturity and every slot has closed.
    pub fn accrued_fee_collateral_atoms(&self) -> StateResult<u128> {
        if !self.is_matured() {
            return Err(StateError::MarketNotMatured);
        }
        if self.subaccount_count != 0 {
            return Err(StateError::SubaccountsNotEmpty);
        }

        if self.materialized_gross_open_interest != 0 || self.pending_maker_oi_reserve != 0 {
            return Err(StateError::UnsettledOpenInterest);
        }

        Ok(self.accrued_protocol_fee_atoms)
    }

    /// Zeros booked fees on the supplied matured markets and returns the
    /// collateral atoms those fees entitle the protocol to withdraw.
    /// Nothing is changed if any market is ineligible or the total is zero.
    pub fn take_matured_protocol_fees(markets: &mut [&mut Self]) -> StateResult<u64> {
        let mut total = 0u128;

        for market in markets.iter_mut() {
            total = total
                .checked_add(market.accrued_fee_collateral_atoms()?)
                .ok_or(StateError::ArithmeticOverflow)?;

            market.accrued_protocol_fee_atoms = 0;
        }

        let amount = u64::try_from(total).map_err(|_| StateError::ArithmeticOverflow)?;

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
        if !self.is_active() && !self.is_close_only() && !self.is_orders_only() {
            return Err(StateError::InvalidStatus);
        }

        let ratio = health_ratio.to_i64f64();

        if ratio < I64F64::ZERO || ratio >= I64F64::ONE {
            return Err(StateError::InvalidLiquidationHealthRatio);
        }
        let time_to_maturity = self
            .config
            .maturity_timestamp
            .checked_sub(now)
            .ok_or(StateError::InvalidClock)?;
        if time_to_maturity <= 0 {
            return Err(StateError::MarketExpired);
        }

        let victim_slot = victim
            .lookup_market(self.config.market_id)
            .ok_or(StateError::MarketNotEntered)?;
        let old_victim = victim.markets[victim_slot].position_size;
        if size_to_liquidator == 0
            || old_victim == 0
            || size_to_liquidator.signum() != old_victim.signum()
            || size_to_liquidator.unsigned_abs() > old_victim.unsigned_abs()
        {
            return Err(StateError::InvalidLiquidationSize);
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
        let margin_before = self.maintenance_margin(
            self.position_risk_principal(old_victim, mark)?,
            time_to_maturity,
        )?;
        let margin_after = self.maintenance_margin(
            self.position_risk_principal(new_victim, mark)?,
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
            return Err(StateError::InvalidMarketFactors);
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
                super::USDC_DECIMALS,
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
            .ok_or(StateError::OpenInterestAccountingMismatch)?;
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
                    return Err(StateError::MarketSlotLimitExceeded);
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
        if status != MARKET_ACTIVE
            && status != MARKET_CLOSE_ONLY
            && status != MARKET_ORDERS_ONLY
            && status != MARKET_PAUSED
        {
            return Err(StateError::InvalidStatus);
        }
        let time_to_maturity = self
            .config
            .maturity_timestamp
            .checked_sub(now)
            .ok_or(StateError::InvalidClock)?;
        if time_to_maturity <= 0 {
            return Err(StateError::MarketExpired);
        }
        let alpha_bits = alpha.to_raw_bits();
        if !(0..=(1i128 << 64)).contains(&alpha_bits) {
            return Err(StateError::InvalidDeleveragingFraction);
        }

        let winner_slot = winner
            .lookup_market(self.config.market_id)
            .ok_or(StateError::MarketNotEntered)?;
        let loser_slot = loser
            .lookup_market(self.config.market_id)
            .ok_or(StateError::MarketNotEntered)?;
        let old_winner = winner.markets[winner_slot].position_size;
        let old_loser = loser.markets[loser_slot].position_size;
        let loser_trade = size_to_winner
            .checked_neg()
            .ok_or(StateError::ArithmeticOverflow)?;
        if !Self::trade_reduces(old_winner, size_to_winner)
            || !Self::trade_reduces(old_loser, loser_trade)
        {
            return Err(StateError::DeleveragingMustReduceBothPositions);
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
                super::USDC_DECIMALS,
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
            .ok_or(StateError::OpenInterestAccountingMismatch)?;

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
        let settlement_scale = decimal_scale(super::USDC_DECIMALS)?;
        let maturity_scaled_rate = rate
            .checked_mul(elapsed)
            .and_then(|value| value.checked_mul(market_scale))
            .and_then(|value| value.checked_mul(settlement_scale))
            .and_then(|value| value.checked_div(year))
            .ok_or(StateError::ArithmeticOverflow)?;
        funding_swap_math::rate::FeeRate::try_from_i64f64(maturity_scaled_rate)
            .map_err(|_| StateError::InvalidFeeRate)?
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
    fn affordable_fill(
        &self,
        candidate: u64,
        maker_side: Side,
        tick: i16,
        time_to_maturity: i64,
        maximum_payment_atoms: u128,
        totals: &MatchTotals,
    ) -> StateResult<Option<AffordableFill>> {
        // Affordability is monotonic over size, so retain the largest passing
        // prefix of this node rather than testing every possible fill amount.
        let mut low = 1u64;
        let mut high = candidate;
        let mut accepted = None;

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

            let filled_size = totals
                .filled_size
                .checked_add(middle)
                .ok_or(StateError::ArithmeticOverflow)?;
            let fee = self.taker_fee_atoms(filled_size, time_to_maturity)?;

            let cash = fixed
                .checked_neg()
                .and_then(|value| value.checked_sub(i128::try_from(fee).ok()?))
                .ok_or(StateError::ArithmeticOverflow)?;

            if Self::cash_debit_atoms(cash) <= maximum_payment_atoms {
                // This size fits; search the larger half while retaining it as
                // the best known fill in case every larger size exceeds the cap.
                accepted = Some(AffordableFill {
                    maker_fill: fill,
                    fixed_payment_atoms: payment,
                    totals: MatchTotals {
                        filled_size,
                        fixed_payment_atoms: fixed,
                        taker_fee_atoms: fee,
                        last_matched_tick: Some(tick),
                    },
                });

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
        Rate::from_tick(tick, self.order_book.tick_step).map_err(|error| match error {
            funding_swap_math::error::MathError::InvalidTickStep => StateError::InvalidTickStep,
            funding_swap_math::error::MathError::Overflow => StateError::ArithmeticOverflow,
            funding_swap_math::error::MathError::NegativeFee => StateError::InvalidFeeRate,
            funding_swap_math::error::MathError::InvalidDuration => StateError::InvalidMarketTiming,
        })
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
            return Err(StateError::TradeRateOutsideMarkBound);
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
            return Err(StateError::CloseOnlyViolation);
        }

        let maker_index = close_only_makers
            .binary_search_by_key(&maker_subaccount, |maker| maker.key)
            .map_err(|_| StateError::MissingMakerSubaccount)?;

        let maker = close_only_makers[maker_index].subaccount;

        let slot_index = maker
            .lookup_market(self.config.market_id)
            .ok_or(StateError::MarketNotEntered)?;

        if maker.markets[slot_index].close_only_exempt == 0
            && !Self::fill_is_closing(self.projected_position(maker, slot_index)?, maker_fill)
        {
            return Err(StateError::CloseOnlyViolation);
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
            return Err(StateError::AdapterKindMismatch);
        }

        bytemuck::try_from_bytes(
            &bytemuck::bytes_of(&self.payload)[..core::mem::size_of::<JupiterAdapterPayload>()],
        )
        .map_err(|_| StateError::InvalidAdapterPayload)
    }

    /// Mutably views a Jupiter payload after confirming the active adapter tag.
    pub fn jupiter_mut(&mut self) -> StateResult<&mut JupiterAdapterPayload> {
        if self.kind.0 != ADAPTER_JUPITER {
            return Err(StateError::AdapterKindMismatch);
        }

        bytemuck::try_from_bytes_mut(
            &mut bytemuck::bytes_of_mut(&mut self.payload)
                [..core::mem::size_of::<JupiterAdapterPayload>()],
        )
        .map_err(|_| StateError::InvalidAdapterPayload)
    }

    /// Views a Phoenix payload after confirming the active adapter tag.
    pub fn phoenix(&self) -> StateResult<&PhoenixAdapterPayload> {
        if self.kind.0 != ADAPTER_PHOENIX {
            return Err(StateError::AdapterKindMismatch);
        }

        bytemuck::try_from_bytes(bytemuck::bytes_of(&self.payload))
            .map_err(|_| StateError::InvalidAdapterPayload)
    }

    /// Mutably views a Phoenix payload after confirming the active adapter tag.
    pub fn phoenix_mut(&mut self) -> StateResult<&mut PhoenixAdapterPayload> {
        if self.kind.0 != ADAPTER_PHOENIX {
            return Err(StateError::AdapterKindMismatch);
        }

        bytemuck::try_from_bytes_mut(bytemuck::bytes_of_mut(&mut self.payload))
            .map_err(|_| StateError::InvalidAdapterPayload)
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
            _ => return Err(StateError::InvalidAdapterKind),
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
                    return Err(StateError::InvalidJupiterConfiguration);
                }
            }
            ADAPTER_PHOENIX => {
                let payload = self.phoenix()?;
                if payload.layout.0 != 1
                    || payload.global_configuration == Pubkey::default()
                    || payload.perp_asset_map == Pubkey::default()
                    || payload.market_account == Pubkey::default()
                    || payload.symbol == [0; 16]
                    || payload.tick_size == 0
                    || payload.funding_interval_seconds == 0
                    || payload.funding_period_seconds == 0
                    || payload.max_funding_age_seconds == 0
                    || payload.max_mark_age_slots == 0
                {
                    return Err(StateError::InvalidPhoenixConfiguration);
                }
            }
            _ => return Err(StateError::InvalidAdapterKind),
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
        underlying_oracle: OracleConfig,
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
        market_id: u64,
        mark_observation_window_seconds: u32,
        margin_time_threshold_seconds: u32,
        tick_scale: u8,
        underlying_decimals: u8,
    ) -> StateResult<()> {
        *self = Self {
            hard_gross_open_interest_cap_atoms,
            start_timestamp,
            maturity_timestamp,
            underlying_oracle,
            adapter,
            rate_floor,
            initial_margin_factor,
            maintenance_margin_factor,
            order_rate_bound_factor,
            closing_margin_relief_bound_factor,
            trade_mark_update_bound_factor,
            taker_fee_rate,
            annualized_settlement_fee_rate,
            liquidation_base_incentive,
            liquidation_incentive_slope,
            liquidation_fee_rate,
            market_id,
            mark_observation_window_seconds,
            margin_time_threshold_seconds,
            tick_scale,
            underlying_decimals,
            pm_cache_nonce: 0,
            _pad_units: [0; 2],
            reserved: [0; MARKET_CONFIG_RESERVED],
            _pad_2: [0; 12],
            _pad_3: [0; 12],
        };
        self.zero_opaque_bytes()?;
        self.validate()
    }

    /// Clears all configuration padding before the configuration is persisted.
    fn zero_opaque_bytes(&mut self) -> StateResult<()> {
        self.underlying_oracle._pad = [0; 8];
        self.reserved = [0; MARKET_CONFIG_RESERVED];
        self._pad_2 = [0; 12];
        self._pad_3 = [0; 12];
        self._pad_units = [0; 2];
        self.adapter.zero_opaque_bytes()
    }

    /// Validates caller-provided market limits, factors, and tagged adapter data.
    pub fn validate(&self) -> StateResult<()> {
        if self.tick_scale > funding_swap_math::rate::MAX_DECIMALS
            || self.underlying_decimals > funding_swap_math::rate::MAX_DECIMALS
        {
            return Err(StateError::InvalidAssetDecimals);
        }

        if self.start_timestamp < 0
            || self.maturity_timestamp <= 0
            || self.start_timestamp >= self.maturity_timestamp
        {
            return Err(StateError::InvalidMarketTiming);
        }
        if self.market_id == 0 {
            return Err(StateError::InvalidMarket);
        }
        if self.adapter_kind().0 == ADAPTER_PHOENIX {
            self.underlying_oracle.validate()?;
        }
        if self.hard_gross_open_interest_cap_atoms == 0
            || self.mark_observation_window_seconds == 0
            || self.margin_time_threshold_seconds == 0
        {
            return Err(StateError::InvalidMarketLimits);
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
            return Err(StateError::InvalidMarketFactors);
        }

        self.adapter.validate()
    }

    /// Returns the adapter tag for callers selecting a validated venue path.
    pub fn adapter_kind(&self) -> VenueAdapterKind {
        self.adapter.kind
    }
}

impl MarketState {
    /// Initializes a fresh market and its embedded book from validated configuration.
    pub fn initialize_in_place(
        &mut self,
        config: MarketConfig,
        now: i64,
        tick_step: u8,
        minimum_order_size_atoms: u64,
        maximum_order_size_atoms: u64,
        maximum_open_orders_per_subaccount: u8,
    ) -> StateResult<()> {

        if now < 0 {
            return Err(StateError::InvalidClock);
        }

        self.order_book.initialize_in_place(
            tick_step,
            minimum_order_size_atoms,
            maximum_order_size_atoms,
            maximum_open_orders_per_subaccount,
        )?;

        let one = FixedPoint::from_raw_bits(1i128 << 64);

        let Self {
            config: stored_config,
            floating_index: stored_floating_index,
            fee_index: stored_fee_index,
            source_native_checkpoint: stored_source_native_checkpoint,
            position_factors: stored_position_factors,
            source_last_update_timestamp: stored_source_last_update_timestamp,
            protocol_checkpoint_timestamp: stored_protocol_checkpoint_timestamp,
            epoch_origin_timestamp: stored_epoch_origin_timestamp,
            checkpoint_epoch: stored_checkpoint_epoch,
            previous_mark_rate: stored_previous_mark_rate,
            target_mark_rate: stored_target_mark_rate,
            last_eligible_trade_timestamp: stored_last_eligible_trade_timestamp,
            materialized_gross_open_interest: stored_materialized_gross_open_interest,
            pending_maker_oi_reserve: stored_pending_maker_oi_reserve,
            accrued_protocol_fee_atoms: stored_accrued_protocol_fee_atoms,
            status: stored_status,
            source_seeded: stored_source_seeded,
            _pad_0: stored_pad_0,
            subaccount_count: stored_subaccount_count,
            finalized_at_timestamp: stored_finalized_at_timestamp,
            _pad_tail: stored_pad_tail,
            reserved: stored_reserved,
            order_book: _,
            _pad_open_interest: stored_pad_open_interest,
        } = self;
        *stored_config = config;
        *stored_floating_index = FixedPoint::zeroed();
        *stored_fee_index = FixedPoint::zeroed();
        *stored_source_native_checkpoint = 0;
        *stored_position_factors = ValuationFactors {
            discount_factor: one,
            liability_factor: one,
        };
        *stored_source_last_update_timestamp = now;
        *stored_protocol_checkpoint_timestamp = now;
        *stored_epoch_origin_timestamp = now;
        *stored_checkpoint_epoch = 0;
        *stored_previous_mark_rate = FixedPoint::zeroed();
        *stored_target_mark_rate = FixedPoint::zeroed();
        *stored_last_eligible_trade_timestamp = now;
        *stored_materialized_gross_open_interest = 0;
        *stored_pending_maker_oi_reserve = 0;
        *stored_accrued_protocol_fee_atoms = 0;
        *stored_status = super::MarketStatus(MARKET_PENDING);
        *stored_source_seeded = 0;
        *stored_pad_0 = [0; 2];
        *stored_subaccount_count = 0;
        *stored_finalized_at_timestamp = 0;
        *stored_pad_tail = [0; 8];
        *stored_reserved = [0; MARKET_STATE_RESERVED];
        *stored_pad_open_interest = MarketAlignmentPadding {
            bytes: [0; MARKET_U128_PADDING],
        };

        Ok(())
    }

    /// Activates a seeded market before its configured start time.
    pub fn activate(&mut self) {
        self.status = super::MarketStatus(MARKET_ACTIVE);
    }

    pub fn is_seeded(&self) -> bool {
        self.source_seeded == 1
    }

    pub fn can_start(&self, now: i64) -> bool {
        now < self.config.start_timestamp
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
            tick_scale: self.config.tick_scale,
        }
    }

    /// Lazily rebuilds one slot's cached order risk after a rate-floor change.
    pub fn sync_order_risk(
        &self,
        subaccount: &mut Subaccount,
        slot_index: usize,
    ) -> StateResult<()> {
        let slot = subaccount
            .markets
            .get(slot_index)
            .ok_or(StateError::InvalidMarketSlot)?;

        if slot.market_id != self.config.market_id {
            return Err(StateError::MarketSlotMismatch);
        }

        if slot.order_risk.pm_cache_nonce_snapshot == self.config.pm_cache_nonce {
            return Ok(());
        }

        let mut rebuilt = super::OrderRiskAggregate::default();

        for reference in subaccount.iter_order_refs() {
            if usize::from(reference.slot_index) != slot_index {
                continue;
            }

            let node = self
                .order_book
                .nodes
                .get(usize::from(reference.node_index))
                .ok_or(StateError::OrderNotFound)?;

            if node.size == 0 {
                return Err(StateError::OrderNotFound);
            }

            rebuilt.add(
                node.get_side(),
                node.size.unsigned_abs(),
                self.rate_at_tick(node.tick)?.to_fixed_point(),
                self.config.rate_floor,
                self.order_risk_scale(),
            )?;
        }

        rebuilt.pm_cache_nonce_snapshot = self.config.pm_cache_nonce;
        subaccount.markets[slot_index].order_risk = rebuilt;

        Ok(())
    }

    /// Advances fees with a freshly authenticated underlying USD price
    /// supplied by the market's authenticated underlying USD feed.
    pub fn advance_fee_index_with_price(
        &mut self,
        now: i64,
        underlying_usdc_price: FixedPoint,
    ) -> StateResult<()> {
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

        if underlying_usdc_price.to_raw_bits() <= 0 {
            return Err(StateError::InvalidOraclePrice);
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
            .checked_mul(market_scale)
            .and_then(|value| value.checked_mul(underlying_usdc_price.to_i64f64()))
            .and_then(|value| value.checked_mul(elapsed))
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

    /// Stores a quote-denominated funding delta. Underlying underlying_usdc_price is used only for fees.
    pub fn apply_rate_observation(
        &mut self,
        weighted_delta: RateDelta,
        source_checkpoint: u128,
        source_last_update_timestamp: i64,
        checkpoint_epoch: u64,
        now: i64,
        underlying_usdc_price: FixedPoint,
    ) -> StateResult<()> {
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
            self.advance_fee_index_with_price(fee_now, underlying_usdc_price)?;
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
    ) {
        self.source_native_checkpoint = source_checkpoint;
        self.source_last_update_timestamp = source_last_update_timestamp;
        self.protocol_checkpoint_timestamp = now;
        self.checkpoint_epoch = checkpoint_epoch;
        self.source_seeded = 1;
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

        if expiry_timestamp != 0
            && (expiry_timestamp <= now || expiry_timestamp > self.config.maturity_timestamp)
        {
            return Err(StateError::InvalidOrderExpiry);
        }

        if !self.rate_within_bound(
            self.rate_at_tick(limit_tick)?.to_fixed_point(),
            current_mark,
            self.config.order_rate_bound_factor,
        )? {
            return Err(StateError::OrderRateOutsideMarkBound);
        }

        Ok(())
    }

    /// Returns whether an action would cross the current best maker price.
    fn order_crosses(&self, side: Side, limit_tick: i16) -> bool {
        let maker_side = match side {
            Side::Long => Side::Short,
            Side::Short => Side::Long,
        };

        self.order_book
            .best_tick(maker_side)
            .is_some_and(|tick| Self::tick_is_matchable(side, limit_tick, tick))
    }

    /// Matches one validated action while accumulating batch-wide payment and fee totals.
    #[allow(clippy::too_many_arguments)]
    fn match_order_action(
        &mut self,
        taker: &mut Subaccount,
        taker_slot: usize,
        taker_key: Pubkey,
        now: i64,
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

        let mut head = self.order_book.best_head(maker_side)?;
        while remaining != 0 {
            let Some(node_index) = head else {
                break;
            };

            let (maker_subaccount, maker_remaining_size, maker_tick, next_node, maker_expired) = {
                let node = &self.order_book.nodes[node_index];
                // Full fills detach the node and clear its links, so retain the next FIFO node.
                (
                    node.maker_subaccount,
                    node.remaining_size,
                    node.tick,
                    node.next_node,
                    node.is_expired(now),
                )
            };

            if !Self::tick_is_matchable(side, limit_tick, maker_tick) {
                break;
            }

            if maker_expired {
                head = if next_node == FREE_INDEX {
                    None
                } else {
                    Some(usize::from(next_node))
                };
                continue;
            }

            if maker_subaccount == taker_key {
                return Err(StateError::SelfTrade);
            }

            let candidate = remaining
                .unsigned_abs()
                .min(maker_remaining_size.unsigned_abs());

            let Some(fill) = self.affordable_fill(
                candidate,
                maker_side,
                maker_tick,
                time_to_maturity,
                maximum_payment_atoms,
                totals,
            )?
            else {
                break;
            };

            let maker_fill = fill.maker_fill;

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

            self.apply_maker_match_and_taker_fill(
                taker,
                taker_slot,
                node_index,
                taker_fill,
                fill.fixed_payment_atoms,
                fill.totals
                    .taker_fee_atoms
                    .checked_sub(totals.taker_fee_atoms)
                    .ok_or(StateError::ArithmeticOverflow)?,
            )?;

            *totals = fill.totals;

            remaining = remaining
                .checked_sub(taker_fill)
                .ok_or(StateError::ArithmeticOverflow)?;

            if remaining != 0 && maker_fill == maker_remaining_size {
                head = if next_node == FREE_INDEX {
                    self.order_book.best_head(maker_side)?
                } else {
                    Some(usize::from(next_node))
                };
            }
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
        if !self.may_place_order() {
            return Err(StateError::InvalidStatus);
        }
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
            && self.order_crosses(side, order_intent.limit_tick)
        {
            if order_intent.time_in_force.rejects_crossing() {
                return Err(StateError::PostOnlyOrderWouldCross);
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
        self.sync_order_risk(taker, taker_market_slot)?;

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
                now,
                order_intent.size,
                order_intent.limit_tick,
                time_to_maturity,
                order_intent.maximum_payment_atoms,
                close_only_makers,
                &mut totals,
            )?
        };

        if let Some(tick) = totals.last_matched_tick {
            self.update_mark_after_fill(now, tick)?;
        }

        if order_intent.time_in_force.is_fill_or_kill() && result.remaining_size != 0 {
            return Err(StateError::FillOrKillUnfilled);
        }

        if result.remaining_size != 0
            && order_intent.time_in_force.rests_remainder()
            && !self.order_crosses(side, order_intent.limit_tick)
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
                true,
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
        if !self.may_place_order() {
            return Err(StateError::InvalidStatus);
        }
        if intent.orders.is_empty()
            || intent.orders.len() > super::MAX_ORDERS
            || results.len() != intent.orders.len()
        {
            return Err(StateError::InvalidOrderBatch);
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
        self.sync_order_risk(taker, taker_slot)?;

        let mut totals = MatchTotals::default();

        for (order, result) in intent.orders.iter().zip(results.iter_mut()) {
            let size = Self::signed_amount(order.size, intent.side)?;

            *result = if intent.time_in_force.is_post_only() {
                if self.order_crosses(intent.side, order.limit_tick)
                    && intent.time_in_force.rejects_crossing()
                {
                    return Err(StateError::PostOnlyOrderWouldCross);
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
                    now,
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
            self.update_mark_after_fill(now, tick)?;
        }

        if intent.time_in_force.is_fill_or_kill()
            && results.iter().any(|result| result.remaining_size != 0)
        {
            return Err(StateError::FillOrKillUnfilled);
        }

        for (order, result) in intent.orders.iter().zip(results.iter_mut()) {
            if result.remaining_size != 0
                && intent.time_in_force.rests_remainder()
                && !self.order_crosses(intent.side, order.limit_tick)
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
                    false,
                )?);
            }
        }

        if results.iter().any(|result| result.resting_order.is_some()) {
            self.validate_resting_close_only(
                taker,
                taker_slot,
                intent.side,
                &taker.markets[taker_slot].order_risk,
            )?;
        }

        Ok(())
    }

    /// Validates projected single-order risk or the final same-side batch aggregate.
    fn validate_resting_close_only(
        &self,
        subaccount: &Subaccount,
        slot_index: usize,
        side: Side,
        risk: &super::OrderRiskAggregate,
    ) -> StateResult<()> {
        let slot = &subaccount.markets[slot_index];
        if self.is_close_only()
            && !(aggregate_is_closing_action(
                slot.position_size,
                slot.position_size,
                risk.sum_long_order_size,
                risk.sum_short_order_size,
                Some(side),
            ) || slot.close_only_exempt != 0)
        {
            return Err(StateError::CloseOnlyViolation);
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
    /// Batches defer close-only validation until all remainders have been added.
    fn rest_order(
        &mut self,
        subaccount: &mut Subaccount,
        slot_index: usize,
        subaccount_key: Pubkey,
        order: RestingOrder,
        check_close_only: bool,
    ) -> StateResult<OrderId> {
        // The originally submitted size was validated at placement. A partial
        // fill may leave a resting remainder below the book minimum.
        if self
            .find_order_ref_at_tick(subaccount, slot_index, order.tick)
            .is_some()
        {
            return Err(StateError::DuplicateOrderAtTick);
        }

        let market_order_count = subaccount
            .iter_order_refs()
            .iter()
            .filter(|reference| usize::from(reference.slot_index) == slot_index)
            .count();

        if usize::from(subaccount.order_ref_count) >= super::MAX_ORDERS {
            return Err(StateError::SubaccountOrderLimitExceeded);
        }
        if market_order_count >= usize::from(self.order_book.maximum_open_orders_per_subaccount) {
            return Err(StateError::MarketOrderLimitExceeded);
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

        if check_close_only {
            self.validate_resting_close_only(
                subaccount,
                slot_index,
                Side::from_size(order.size)?,
                &next_risk,
            )?;
        }

        let order_id = self.order_book.add_order(
            subaccount_key,
            order.size,
            order.tick,
            order.time_in_force,
            order.expiry_timestamp,
        )?;

        subaccount.markets[slot_index].order_risk = next_risk;
        subaccount.add_order_ref(OrderRef::new(
            slot_index,
            usize::try_from(order_id.slab_index).map_err(|_| StateError::ArithmeticOverflow)?,
        ));

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
        if order_id.slab_index >= u64::try_from(BOOK_CAPACITY).ok()? {
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

    /// Returns whether this subaccount still holds the stable order ID.
    pub(crate) fn has_order(&self, subaccount: &Subaccount, order_id: OrderId) -> bool {
        let Some(slot_index) = subaccount.lookup_market(self.config.market_id) else {
            return false;
        };

        self.find_order_ref_by_id(subaccount, slot_index, order_id)
            .is_some()
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
    /// order ID. The caller handles zero-sized cancellation separately. Returns
    /// whether the remaining size increases and requires an initial-margin check.
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
        now: i64,
    ) -> StateResult<bool> {
        if !self.may_cancel_order() {
            return Err(StateError::InvalidStatus);
        }

        let slot_index = subaccount
            .lookup_market(self.config.market_id)
            .ok_or(StateError::MarketNotEntered)?;
        self.sync_order_risk(subaccount, slot_index)?;

        let Some(ref_index) = self.find_order_ref_by_id(subaccount, slot_index, order_id) else {
            return Err(StateError::OrderNotFound);
        };

        let node_index = usize::from(subaccount.order_refs[ref_index].node_index);

        let (remaining_size, tick, side, expired) = {
            let node = &self.order_book.nodes[node_index];

            (
                node.remaining_size,
                node.tick,
                node.get_side(),
                node.is_expired(now),
            )
        };

        if expired {
            return Err(StateError::OrderExpired);
        }

        if self.order_book.nodes[node_index].has_pending_fill() {
            return Err(StateError::UnmaterializedOrderFill);
        }

        // We don't flip side from the modify path: https://pochettes.pulsradio.com/drake%20-%20hotline%20bling.jpg
        if new_size.signum() != remaining_size.signum() {
            return Err(StateError::OrderSideChangeNotAllowed);
        }

        let old_size = remaining_size.unsigned_abs();
        let new_size_abs = new_size.unsigned_abs();

        if new_size_abs == old_size {
            return Ok(false);
        }

        let increases = new_size_abs > old_size;

        if increases && !self.may_place_order() {
            return Err(StateError::InvalidStatus);
        }

        if increases {
            self.order_book.validate_order_size(new_size)?;
        }

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
            return Err(StateError::CloseOnlyViolation);
        }

        self.order_book
            .set_order_sizes(node_index, new_size, new_size)?;

        subaccount.markets[slot_index].order_risk = next_risk;

        Ok(increases)
    }

    /// Applies same-market stable-ID size changes after the caller establishes
    /// and materializes the slot.
    pub fn modify_order_batch(
        &mut self,
        subaccount: &mut Subaccount,
        changes: &[ModifyOrderIntent],
        now: i64,
    ) -> StateResult<()> {
        if changes.len() > super::MAX_ORDERS {
            return Err(StateError::InvalidOrderBatch);
        }

        for change in changes {
            self.modify_order(subaccount, change.order_id, change.new_size, now)?;
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
        if !self.may_cancel_order() {
            return Err(StateError::InvalidStatus);
        }

        let slot_index = subaccount
            .lookup_market(self.config.market_id)
            .ok_or(StateError::MarketNotEntered)?;
        self.sync_order_risk(subaccount, slot_index)?;

        let Some(ref_index) = self.find_order_ref_by_id(subaccount, slot_index, order_id) else {
            return Err(StateError::OrderNotFound);
        };

        self.cancel_order_ref(subaccount, slot_index, ref_index)
    }

    /// Removes one unfilled order remainder after its expiry has passed.
    ///
    /// The caller must materialize pending fills first. A fully resolved order
    /// is already absent and this cleanup is idempotent.
    pub fn purge_expired_order(
        &mut self,
        subaccount: &mut Subaccount,
        order_id: OrderId,
        now: i64,
    ) -> StateResult<()> {
        if !self.may_cancel_order() {
            return Err(StateError::InvalidStatus);
        }

        let slot_index = subaccount
            .lookup_market(self.config.market_id)
            .ok_or(StateError::MarketNotEntered)?;
        self.sync_order_risk(subaccount, slot_index)?;

        let Some(ref_index) = self.find_order_ref_by_id(subaccount, slot_index, order_id) else {
            return Ok(());
        };

        let node_index = usize::from(subaccount.order_refs[ref_index].node_index);

        if self.order_book.nodes[node_index].has_pending_fill() {
            return Err(StateError::UnmaterializedOrderFill);
        }

        if !self.order_book.nodes[node_index].is_expired(now) {
            return Err(StateError::OrderNotExpired);
        }

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
            return Err(StateError::InvalidOrderBatch);
        }

        for order_id in orders {
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
            return Err(StateError::InvalidMarketSlot);
        }
        self.sync_order_risk(subaccount, slot_index)?;

        let ref_index = usize::from(subaccount.order_ref_count);

        for index in (0..ref_index).rev() {
            if usize::from(subaccount.order_refs[index].slot_index) == slot_index {
                self.cancel_order_ref(subaccount, slot_index, index)?;
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
            return Err(StateError::InvalidTickRange);
        }
        if !self.may_cancel_order() {
            return Err(StateError::InvalidStatus);
        }

        let slot_index = subaccount
            .lookup_market(self.config.market_id)
            .ok_or(StateError::MarketNotEntered)?;
        self.sync_order_risk(subaccount, slot_index)?;

        let ref_index = usize::from(subaccount.order_ref_count);

        for index in (0..ref_index).rev() {
            if usize::from(subaccount.order_refs[index].slot_index) == slot_index {
                let node_index = usize::from(subaccount.order_refs[index].node_index);

                let out_of_bounds = {
                    let node = &self.order_book.nodes[node_index];
                    node.tick < minimum_tick || node.tick > maximum_tick
                };

                if out_of_bounds {
                    self.cancel_order_ref(subaccount, slot_index, index)?;
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
        if !self.may_cancel_order() {
            return Err(StateError::InvalidStatus);
        }

        let slot_index = subaccount
            .lookup_market(self.config.market_id)
            .ok_or(StateError::MarketNotEntered)?;
        self.sync_order_risk(subaccount, slot_index)?;

        let position = self.projected_position(subaccount, slot_index)?;

        let ref_index = usize::from(subaccount.order_ref_count);

        for index in (0..ref_index).rev() {
            if usize::from(subaccount.order_refs[index].slot_index) == slot_index {
                let node_index = usize::from(subaccount.order_refs[index].node_index);

                let node = &self.order_book.nodes[node_index];

                let increases = position == 0 || node.size.signum() == position.signum();

                if increases {
                    self.cancel_order_ref(subaccount, slot_index, index)?;
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
        if !self.may_cancel_order() {
            return Err(StateError::InvalidStatus);
        }

        let slot_index = subaccount
            .lookup_market(self.config.market_id)
            .ok_or(StateError::MarketNotEntered)?;
        self.sync_order_risk(subaccount, slot_index)?;

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

        // Shorts fill from the lowest tick and
        // longs from the highest tick, mirroring the book's best-tick choice.
        let fills_first = |candidate: &(i16, u16), against: &(i16, u16)| match opposite_side {
            Side::Short => candidate.0.cmp(&against.0),
            Side::Long => against.0.cmp(&candidate.0),
        };

        candidates[..candidate_count].sort_unstable_by(fills_first);

        let mut kept = 0u64;

        for &(tick, node_index) in &candidates[..candidate_count] {
            let node_index = usize::from(node_index);

            let size = self.order_book.nodes[node_index]
                .remaining_size
                .unsigned_abs();

            if kept == needed {
                let Some(ref_index) =
                    Self::find_ref_index_by_node(subaccount, slot_index, node_index)
                else {
                    return Err(StateError::OrderNotFound);
                };

                self.cancel_order_ref(subaccount, slot_index, ref_index)?;
            } else if kept + size <= needed {
                kept += size;
            } else {
                // The straddling order: trim it to exactly cover the position.
                let trim_target = needed - kept;

                kept = needed;

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

        let floating_atoms = funding_swap_math::rate::RateDelta::from_i64f64(
            fixed::types::I64F64::from_bits(floating_delta),
        )
        .floating_payment_atoms_scaled(
            i128::from(market_slot.position_size),
            self.config.underlying_decimals,
            super::USDC_DECIMALS,
            self.config.tick_scale,
        )
        .map_err(|_| StateError::ArithmeticOverflow)?;

        let fee_delta = self
            .fee_index
            .to_raw_bits()
            .checked_sub(market_slot.fee_index_snapshot.to_raw_bits())
            .ok_or(StateError::ArithmeticOverflow)?;

        let fee_atoms = funding_swap_math::rate::FeeRate::try_from_i64f64(
            fixed::types::I64F64::from_bits(fee_delta),
        )
        .and_then(|fee| {
            fee.settlement_fee_atoms_scaled(
                market_slot.position_size.unsigned_abs().into(),
                self.config.underlying_decimals,
                super::USDC_DECIMALS,
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
        now: i64,
    ) -> StateResult<()> {
        let reference = subaccount.order_refs[ref_index];
        let slot_index = usize::from(reference.slot_index);
        self.sync_order_risk(subaccount, slot_index)?;
        let node_index = usize::from(reference.node_index);

        let (
            pending,
            remaining_size,
            tick,
            side,
            pending_floating_index_basis,
            pending_fee_index_basis,
            pending_fixed_payment_atoms,
            expired,
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
                node.is_expired(now),
            )
        };

        if pending == 0 {
            if expired {
                return self.cancel_order_ref(subaccount, slot_index, ref_index);
            }

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

        let floating_atoms = funding_swap_math::rate::RateDelta::from_i64f64(
            fixed::types::I64F64::from_bits(floating_delta),
        )
        .floating_payment_atoms_scaled(
            1,
            self.config.underlying_decimals,
            super::USDC_DECIMALS,
            self.config.tick_scale,
        )
        .map_err(|_| StateError::ArithmeticOverflow)?;

        let fee_delta = i128::from(pending.unsigned_abs())
            .checked_mul(self.fee_index.to_raw_bits())
            .and_then(|value| value.checked_sub(pending_fee_index_basis.to_raw_bits()))
            .ok_or(StateError::ArithmeticOverflow)?;

        let fee_atoms = funding_swap_math::rate::FeeRate::try_from_i64f64(
            fixed::types::I64F64::from_bits(fee_delta),
        )
        .map_err(|_| StateError::ArithmeticOverflow)?
        .settlement_fee_atoms_scaled(
            1,
            self.config.underlying_decimals,
            super::USDC_DECIMALS,
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

            // The matched portion is now settled. An expired remainder is
            // removed in the same pass using the normal cancellation path.
            if expired {
                self.cancel_order_ref(subaccount, slot_index, ref_index)?;
            }
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
                super::USDC_DECIMALS,
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
        let settlement_scale = decimal_scale(super::USDC_DECIMALS)?;

        let maturity_scaled_rate = rate
            .checked_mul(elapsed)
            .and_then(|value| value.checked_mul(market_scale))
            .and_then(|value| value.checked_mul(settlement_scale))
            .and_then(|value| value.checked_div(year))
            .ok_or(StateError::ArithmeticOverflow)?;

        funding_swap_math::rate::FeeRate::try_from_i64f64(maturity_scaled_rate)
            .map_err(|_| StateError::InvalidFeeRate)?
            .settlement_fee_atoms_scaled(
                u128::from(size),
                self.config.underlying_decimals,
                0,
                self.config.tick_scale,
            )
            .map_err(|_| StateError::ArithmeticOverflow)
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
                .checked_add(fixed_payment_atoms)
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

        let taker_payment = fixed_payment_atoms
            .checked_neg()
            .ok_or(StateError::ArithmeticOverflow)?;

        let cash_delta = taker_payment
            .checked_sub(i128::try_from(fee_atoms).map_err(|_| StateError::ArithmeticOverflow)?)
            .ok_or(StateError::ArithmeticOverflow)?;

        let next_cash = taker.markets[market_slot_index]
            .settlement_balance_atoms
            .checked_add(cash_delta)
            .ok_or(StateError::ArithmeticOverflow)?;

        let next_materialized = self
            .materialized_gross_open_interest
            .checked_sub(u128::from(slot.position_size.unsigned_abs()))
            .and_then(|value| value.checked_add(u128::from(next_position.unsigned_abs())))
            .ok_or(StateError::OpenInterestAccountingMismatch)?;

        let capped = next_materialized
            .checked_add(next_reserve)
            .ok_or(StateError::ArithmeticOverflow)?;

        if capped > self.config.hard_gross_open_interest_cap_atoms {
            return Err(StateError::OpenInterestCapExceeded);
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
            return Err(StateError::OpenInterestCapExceeded);
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
        // Pending fills reserve their full absolute size. Materialization
        // replaces that conservative reserve with the exact position OI delta.
        let exact_delta = new_position_abs.abs_diff(old_position_abs);

        // Exact replacement must not turn a bounded reservation into extra OI.
        if exact_delta > reserved_amount {
            return Err(StateError::InsufficientOpenInterestReserve);
        }

        let after_old = self
            .materialized_gross_open_interest
            .checked_sub(old_position_abs)
            .ok_or(StateError::OpenInterestAccountingMismatch)?;
        let next_materialized = after_old
            .checked_add(new_position_abs)
            .ok_or(StateError::ArithmeticOverflow)?;
        let next_reserve = self
            .pending_maker_oi_reserve
            .checked_sub(reserved_amount)
            .ok_or(StateError::InsufficientOpenInterestReserve)?;

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
                super::USDC_DECIMALS,
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
    /// Callers supply position-only risk; resting orders reserve initial margin.
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

    /// Position-only potential margin used by maintenance-margin calculations.
    fn position_risk_principal(
        &self,
        position_size: i64,
        mark_rate: FixedPoint,
    ) -> StateResult<FixedPoint> {
        Ok(FixedPoint::from_raw_bits(
            super::OrderRiskAggregate::pm_contribution(
                position_size.unsigned_abs(),
                mark_rate,
                self.config.rate_floor,
                self.order_risk_scale(),
            )?,
        ))
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
        let pm_alone = self.position_risk_principal(position_size, mark_rate)?;

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

    /// An opposite-side order book fully covered by the position carries
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
    /// Inputs are the pre-batch settled position, the
    /// post-batch final position and final aggregate order risk (whose
    /// `sum_long_order_size`/`sum_short_order_size` carry the final aggregate
    /// order sizes), the side of any newly resting order, the close-only exemption, the
    /// account's validated critical-health ratio, the net payment including
    /// fees `P`, the worst rate among newly resting closing orders, and the
    /// current clock. Callers must have materialized every pending fill
    /// first. Maintenance margin uses only realized positions; resting orders
    /// still participate in closing-action classification and initial margin.
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
        let final_risk = self.position_risk_principal(post_position_size, mark)?;
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
            // The proposed worst closing rate must stay within
            // `closing_margin_relief_bound_factor * max(abs(mark), rate_floor)`
            // of the mark, signed by the final position side.
            if new_resting_order_side.is_some() {
                let worst = worst_closing_rate.ok_or(StateError::MissingClosingOrderRate)?;
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
            // ratio product floored. The close-only
            // exemption never enters this relief decision.
            let pre_value = self.position_value(pre_position_size, mark, time_to_maturity)?;
            let pre_risk = self.position_risk_principal(pre_position_size, mark)?;
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
    /// Collateral and settlement cash in USDC atoms.
    pub adjusted_total_assets: i128,
    /// Signed position value in USDC atoms.
    pub total_value: i128,
    /// Selected margin requirement in USDC atoms.
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
            return Err(StateError::ExpectedInitialMargin);
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
            return Err(StateError::ExpectedMaintenanceMargin);
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
            return Err(StateError::ExpectedMaintenanceMargin);
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

    /// Accepted only on `[0, 1)`.
    /// Negative value is bad debt and is not liquidatable here.
    pub fn liquidation_health_ratio(&self) -> StateResult<FixedPoint> {
        let MarginRequirement::Maintenance(maintenance) = self.required_margin else {
            return Err(StateError::ExpectedMaintenanceMargin);
        };

        if maintenance <= 0 {
            return Err(StateError::NoMaintenanceMargin);
        }

        let available = self
            .adjusted_total_assets
            .checked_add(self.total_value)
            .ok_or(StateError::ArithmeticOverflow)?;

        if available < 0 {
            return Err(StateError::NegativeEquity);
        }
        if available >= maintenance {
            return Err(StateError::NotLiquidatable);
        }

        self.maintenance_health_ratio()
    }

    /// Portfolio maintenance ratio `value / maintenance`, including ratios
    /// outside `[0, 1)`. Deleveraging compares these directly.
    pub fn maintenance_health_ratio(&self) -> StateResult<FixedPoint> {
        let MarginRequirement::Maintenance(maintenance) = self.required_margin else {
            return Err(StateError::ExpectedMaintenanceMargin);
        };

        if maintenance <= 0 {
            return Err(StateError::NoMaintenanceMargin);
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
    /// Ensures the subaccount belongs to this group.
    pub fn ensure_subaccount_matches(&self, subaccount: &Subaccount) -> StateResult<()> {
        if self.group_id != subaccount.group_id {
            return Err(StateError::InvalidGroupID);
        }

        Ok(())
    }

    /// Initializes immutable group identity and health
    /// thresholds from caller input.
    ///
    /// Membership is established by creating the discriminator-only
    /// `MarketGroupMembership` account per market, not by a group-held list.
    pub fn initialize_in_place(
        &mut self,
        bump: u8,
        group_id: u16,
        critical_health_ratio: FixedPoint,
        risky_order_cancellation_ratio: FixedPoint,
    ) -> StateResult<()> {
        if group_id == 0 {
            return Err(StateError::InvalidGroupID);
        }
        if critical_health_ratio.to_raw_bits() <= 0
            || risky_order_cancellation_ratio.to_raw_bits() < critical_health_ratio.to_raw_bits()
        {
            return Err(StateError::InvalidGroupHealthRatios);
        }

        *self = Self {
            group_id,
            bump,
            status: GROUP_PENDING,
            critical_health_ratio,
            risky_order_cancellation_ratio,
            market_count: 0,
            subaccount_count: 0,
            _pad_0: [0; 4],
            reserved: [0; MARGIN_GROUP_RESERVED],
        };

        Ok(())
    }

    /// Activates a structurally initialized group after resource authorization.
    pub fn activate(&mut self) {
        self.status = GROUP_ACTIVE;
    }

    /// Computes health in USDC atoms. Callers gate risk-sensitive actions on the USDC oracle.
    pub fn initial_portfolio_health<'a>(
        &self,
        subaccount: &Subaccount,
        markets: impl IntoIterator<Item = &'a MarketState>,
        now: i64,
    ) -> StateResult<PortfolioHealth> {
        self.portfolio_health(subaccount, markets, now, MarginRequirementType::Initial)
    }

    pub fn maintenance_portfolio_health<'a>(
        &self,
        subaccount: &Subaccount,
        markets: impl IntoIterator<Item = &'a MarketState>,
        now: i64,
    ) -> StateResult<PortfolioHealth> {
        self.portfolio_health(subaccount, markets, now, MarginRequirementType::Maintenance)
    }

    fn portfolio_health<'a>(
        &self,
        subaccount: &Subaccount,
        markets: impl IntoIterator<Item = &'a MarketState>,
        now: i64,
        requirement_type: MarginRequirementType,
    ) -> StateResult<PortfolioHealth> {
        let mut markets_by_slot = [None; super::MAX_MARKETS];
        for market in markets {
            if let Some(slot) = subaccount.lookup_market(market.config.market_id) {
                markets_by_slot[slot] = Some(market);
            }
        }
        let mut assets = subaccount.collateral_balance_atoms;
        let mut total_value = 0i128;
        let mut total_margin = 0i128;
        for (slot_index, slot) in subaccount.markets.iter().enumerate() {
            if slot.market_id == 0 {
                continue;
            }
            let market = markets_by_slot[slot_index].ok_or(StateError::MissingMarketObservation)?;
            if slot.order_risk.pm_cache_nonce_snapshot != market.config.pm_cache_nonce {
                return Err(StateError::OrderRiskAccountingMismatch);
            }
            let time = market
                .config
                .maturity_timestamp
                .checked_sub(now)
                .ok_or(StateError::InvalidClock)?;
            let mark = market.current_mark_rate(now)?;
            let value = market.position_factors.apply(market.position_value(
                slot.position_size,
                mark,
                time,
            )?)?;
            total_value = total_value
                .checked_add(value)
                .ok_or(StateError::ArithmeticOverflow)?;
            let balance = slot.settlement_balance_atoms;
            assets = assets
                .checked_add(balance)
                .ok_or(StateError::ArithmeticOverflow)?;
            let margin = match requirement_type {
                MarginRequirementType::Initial => market.initial_margin(
                    market.risk_principal(slot.position_size, &slot.order_risk, mark)?,
                    time,
                )?,
                MarginRequirementType::Maintenance => market.maintenance_margin(
                    market.position_risk_principal(slot.position_size, mark)?,
                    time,
                )?,
            };
            total_margin = total_margin
                .checked_add(margin)
                .ok_or(StateError::ArithmeticOverflow)?;
        }
        Ok(PortfolioHealth {
            adjusted_total_assets: assets,
            total_value,
            required_margin: match requirement_type {
                MarginRequirementType::Initial => MarginRequirement::Initial(total_margin),
                MarginRequirementType::Maintenance => MarginRequirement::Maintenance(total_margin),
            },
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
                .ok_or(StateError::MissingMarketAccount)?;

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
    fn dispatch_cleanup<'a>(
        subaccount: &mut Subaccount,
        markets: impl IntoIterator<Item = &'a mut MarketState>,
    ) -> StateResult<()> {
        let mut slots_with_refs = [false; super::MAX_MARKETS];

        for reference in subaccount.iter_order_refs() {
            slots_with_refs[usize::from(reference.slot_index)] = true;
        }

        for market in markets {
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
    pub fn close_only_cleanup<'a>(
        subaccount: &mut Subaccount,
        markets: impl IntoIterator<Item = &'a mut MarketState>,
    ) -> StateResult<()> {
        Self::dispatch_cleanup(subaccount, markets)
    }
}
