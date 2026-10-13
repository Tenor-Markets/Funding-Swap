// CHECKED
use anchor_lang::prelude::*;
use bytemuck::{Pod, Zeroable};

use super::{FixedPoint, MarketState, Side, StateError, StateResult};

pub const SUBACCOUNT_RESERVED: usize = 64;
pub const MARKET_SLOT_RESERVED: usize = 8;
pub const MAX_MARKETS: usize = 6;
pub const MAX_ORDERS: usize = 32;

macro_rules! tag_type {
    ($name:ident) => {
        #[repr(transparent)]
        #[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Pod, Zeroable)]
        #[cfg_attr(feature = "idl-build", derive(AnchorSerialize, AnchorDeserialize))]
        pub struct $name(pub u8);
    };
}

tag_type!(SubaccountStatus);

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Pod, Zeroable)]
#[cfg_attr(feature = "idl-build", derive(AnchorSerialize, AnchorDeserialize))]
pub struct OrderRiskAggregate {
    // Cached remaining-order risk. It is rebuilt on demand from canonical book
    // nodes when the market's cache nonce changes.
    pub sum_long_order_size: u64,
    pub sum_long_order_pm: FixedPoint,
    pub sum_short_order_size: u64,
    pub sum_short_order_pm: FixedPoint,
    pub pm_cache_nonce_snapshot: u32,
    // Completes the fixed 56-byte cache record without implicit ABI padding.
    pub _pad_0: [u8; 4],
}

/// Copyable market unit scales used when converting order risk into settlement atoms.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct OrderRiskScale {
    pub(crate) underlying_decimals: u8,
    pub(crate) settlement_decimals: u8,
    pub(crate) tick_scale: u8,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Pod, Zeroable)]
#[cfg_attr(feature = "idl-build", derive(AnchorSerialize, AnchorDeserialize))]
pub struct OrderRef {
    pub slot_index: u8,
    pub _pad_0: [u8; 1],
    pub node_index: u16,
}

impl OrderRef {
    /// Creates a reference from slot and node indices validated by the market transition.
    pub(crate) fn new(slot_index: usize, node_index: usize) -> Self {
        Self {
            slot_index: slot_index as u8,
            node_index: node_index as u16,
            ..Self::zeroed()
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Pod, Zeroable)]
#[cfg_attr(feature = "idl-build", derive(AnchorSerialize, AnchorDeserialize))]
pub struct MarketSlot {
    pub market_id: u64,
    /// Signed derivative position in underlying atoms.
    pub position_size: i64,
    pub floating_index_snapshot: FixedPoint,
    pub fee_index_snapshot: FixedPoint,
    pub order_risk: OrderRiskAggregate,
    pub close_only_exempt: u8,
    // Aligns settlement ownership before the personal-risk tail.
    pub _pad_1: [u8; 23],
    /// Conserved settlement cash in settlement atoms.
    pub settlement_balance_atoms: i128,
    pub personal_k_im: FixedPoint,
    pub personal_k_mm: FixedPoint,
    pub reserved: [u8; MARKET_SLOT_RESERVED],
    pub _pad_2: [u8; 8],
}

#[account(zero_copy)]
#[derive(Debug, Eq, PartialEq)]
pub struct Subaccount {
    pub authority: Pubkey,
    pub subaccount_id: u16,
    pub group_id: u16,
    pub status: SubaccountStatus,
    pub bump: u8,
    pub _pad_0: [u8; 10],
    pub collateral_balance_atoms: i128,
    pub delegate: Pubkey,
    pub markets: [MarketSlot; MAX_MARKETS],
    pub order_refs: [OrderRef; MAX_ORDERS],
    pub order_ref_count: u8,
    pub _pad_1: [u8; 7],
    pub reserved: [u8; SUBACCOUNT_RESERVED],
    pub _pad_2: [u8; 8],
}

impl Subaccount {
    pub const DATA_LEN: usize = core::mem::size_of::<Self>();
    pub const SPACE: usize = 8 + Self::DATA_LEN;

    pub fn is_authority(&self, authority: Pubkey) -> bool {
        self.authority == authority
    }

    pub fn is_same_group_id(&self, group_id: u16) -> bool {
        self.group_id == group_id
    }
}

pub const SUBACCOUNT_ACTIVE: u8 = 1;

impl Subaccount {
    /// Initializes a fresh subaccount in its authority's selected margin group.
    pub fn initialize_in_place(
        &mut self,
        bump: u8,
        authority: Pubkey,
        subaccount_id: u16,
        group_id: u16,
    ) -> StateResult<()> {
        if authority == Pubkey::default() || subaccount_id == 0 || group_id == 0 {
            return Err(StateError::InvalidConfiguration);
        }

        self.authority = authority;
        self.subaccount_id = subaccount_id;
        self.group_id = group_id;
        self.status = super::SubaccountStatus(SUBACCOUNT_ACTIVE);
        self.bump = bump;
        self._pad_0 = [0; 10];
        self.collateral_balance_atoms = 0;
        self.delegate = Pubkey::default();
        self.markets = [MarketSlot::zeroed(); MAX_MARKETS];
        self.order_refs = [OrderRef::zeroed(); MAX_ORDERS];
        self.order_ref_count = 0;
        self._pad_1 = [0; 7];
        self.reserved = [0; SUBACCOUNT_RESERVED];
        self._pad_2 = [0; 8];
        Ok(())
    }

    /// Changes the delegate; the default key deliberately clears delegation.
    pub fn set_delegate(&mut self, delegate: Pubkey) -> StateResult<()> {
        if delegate == self.authority {
            return Err(StateError::InvalidAuthority);
        }

        self.delegate = delegate;

        Ok(())
    }

    /// Credits transferred collateral atoms after the instruction validates custody.
    pub fn deposit_collateral(&mut self, amount: u64) -> StateResult<()> {
        self.collateral_balance_atoms = self
            .collateral_balance_atoms
            .checked_add(i128::from(amount))
            .ok_or(StateError::ArithmeticOverflow)?;

        Ok(())
    }

    /// Returns this market's slot, allocating an empty slot with current index snapshots.
    pub fn ensure_market(&mut self, market: &mut MarketState) -> StateResult<usize> {
        if let Some(market_slot_index) = self.lookup_market(market.config.market_id) {
            return Ok(market_slot_index);
        }

        let market_slot_index = self
            .markets
            .iter()
            .position(|market_slot| market_slot.market_id == 0)
            .ok_or(StateError::CapacityExhausted)?;

        market.add_subaccount()?;

        self.markets[market_slot_index] = MarketSlot {
            market_id: market.config.market_id,
            floating_index_snapshot: market.floating_index,
            fee_index_snapshot: market.fee_index,
            ..MarketSlot::zeroed()
        };

        Ok(market_slot_index)
    }

    /// Releases one occupied market slot and updates the market's subaccount count.
    /// It assumes the index provided is valid.
    pub(crate) fn close_market(
        &mut self,
        market_slot_index: usize,
        market: &mut MarketState,
    ) -> StateResult<()> {
        let market_slot = self
            .markets
            .get(market_slot_index)
            .ok_or(StateError::InvalidReference)?;

        if market_slot.market_id != 0 {
            market.remove_subaccount()?;
            self.markets[market_slot_index] = MarketSlot::zeroed();
        }

        Ok(())
    }

    /// Converts a market slot's signed settlement balance into collateral.
    /// The slot remains open and its position/order state is unchanged.
    pub(crate) fn settle_market_into_collateral(
        &mut self,
        market_slot_index: usize,
        market: &MarketState,
        ratio: super::SettlementRatio,
    ) -> StateResult<()> {
        let slot = self
            .markets
            .get(market_slot_index)
            .ok_or(StateError::InvalidReference)?;

        if slot.market_id != market.config.market_id || market.subaccount_count == 0 {
            return Err(StateError::InvalidMarket);
        }

        if slot.settlement_balance_atoms == 0 {
            return Ok(());
        }

        let collateral = ratio.apply(slot.settlement_balance_atoms)?;

        self.collateral_balance_atoms = self
            .collateral_balance_atoms
            .checked_add(collateral)
            .ok_or(StateError::ArithmeticOverflow)?;

        self.markets[market_slot_index].settlement_balance_atoms = 0;

        Ok(())
    }

    /// Converts a finalized market's signed settlement balance into this
    /// subaccount's signed collateral balance, then releases the slot.  The
    /// market ratio is applied exactly once; a missing ratio keeps the slot
    /// open so callers cannot close around an unsettled obligation.  A
    /// non-zero position is the expired contract itself and is retired here
    /// after all pending orders have already been cancelled by the caller.
    pub fn settle_and_close_market(
        &mut self,
        market_slot_index: usize,
        market: &mut MarketState,
    ) -> StateResult<()> {
        // TODO: Remove the two checks below, the callers handle this.
        if !market.is_matured() {
            return Err(StateError::InvalidStatus);
        }

        let ratio = market
            .frozen_settlement_ratio()
            .ok_or(StateError::InvalidStatus)?;

        let slot = self
            .markets
            .get(market_slot_index)
            .ok_or(StateError::InvalidReference)?;

        if !slot.order_risk.is_empty()
            || self
                .iter_order_refs()
                .iter()
                .any(|reference| usize::from(reference.slot_index) == market_slot_index)
        {
            return Err(StateError::Conflict);
        }

        // The position is already in materialized open interest. Retiring it
        // removes that contribution. Pending maker reserve stays until the
        // maker node is materialized, which also removes its order reference.
        let position_abs = u128::from(slot.position_size.unsigned_abs());

        let next_open_interest = market
            .materialized_gross_open_interest
            .checked_sub(position_abs)
            .ok_or(StateError::Conflict)?;

        // Settlement is idempotent once the balance is zero, so a later close
        // failure can be retried without subtracting open interest twice.
        self.settle_market_into_collateral(market_slot_index, market, ratio)?;

        market.materialized_gross_open_interest = next_open_interest;

        self.close_market(market_slot_index, market)
    }

    /// Appends a unique reference after the market transition checks capacity and ownership.
    pub(crate) fn add_order_ref(&mut self, reference: OrderRef) {
        let index = usize::from(self.order_ref_count);
        self.order_refs[index] = reference;
        self.order_ref_count = (index + 1) as u8;
    }

    /// Swap-removes a validated live reference and clears the vacated tail.
    pub(crate) fn remove_order_ref(&mut self, index: usize) {
        let count = usize::from(self.order_ref_count);
        let last = count - 1;
        self.order_refs[index] = self.order_refs[last];
        self.order_refs[last] = OrderRef::zeroed();
        self.order_ref_count = last as u8;
    }

    /// Returns the live reference prefix; `order_ref_count` is maintained by add/remove.
    pub(crate) fn iter_order_refs(&self) -> &[OrderRef] {
        let count = usize::from(self.order_ref_count);
        &self.order_refs[..count]
    }

    /// Finds an active slot for the requested market identifier.
    pub(crate) fn lookup_market(&self, market_id: u64) -> Option<usize> {
        self.markets
            .iter()
            .position(|slot| slot.market_id == market_id)
    }
}

impl OrderRiskAggregate {
    /// Returns whether both sides have no remaining order size.
    /// Floating-point PM values and the nonce do not determine live order risk.
    pub(crate) fn is_empty(&self) -> bool {
        self.sum_long_order_size == 0 && self.sum_short_order_size == 0
    }

    /// Computes one side's Q64.64 potential-margin contribution from bounded inputs.
    pub(crate) fn pm_contribution(
        size: u64,
        rate: super::FixedPoint,
        rate_floor: super::FixedPoint,
        unit_scale: OrderRiskScale,
    ) -> StateResult<i128> {
        let rate_raw = rate
            .to_raw_bits()
            .checked_abs()
            .ok_or(StateError::ArithmeticOverflow)?;

        let floor_raw = rate_floor.to_raw_bits();

        if floor_raw < 0 {
            return Err(StateError::InvalidConfiguration);
        }

        let effective_raw = rate_raw.max(floor_raw);

        let scale = |decimals: u8| -> StateResult<fixed::types::I64F64> {
            let mut value = fixed::types::I64F64::ONE;
            for _ in 0..decimals {
                value = value
                    .checked_mul(fixed::types::I64F64::from_num(10))
                    .ok_or(StateError::ArithmeticOverflow)?;
            }
            Ok(value)
        };

        let size =
            fixed::types::I64F64::checked_from_num(size).ok_or(StateError::ArithmeticOverflow)?;

        let effective_rate = fixed::types::I64F64::from_bits(effective_raw);

        let settlement_scale = scale(unit_scale.settlement_decimals)?;

        let underlying_scale = scale(unit_scale.underlying_decimals)?;

        let tick_scale = scale(unit_scale.tick_scale)?;

        effective_rate
            .checked_mul(size)
            .and_then(|value| value.checked_mul(settlement_scale))
            .and_then(|value| value.checked_div(underlying_scale))
            .and_then(|value| value.checked_div(tick_scale))
            .map(|value| value.to_bits())
            .ok_or(StateError::ArithmeticOverflow)
    }

    /// Adds remaining order risk to the selected side's cached aggregate.
    pub(crate) fn add(
        &mut self,
        side: Side,
        size: u64,
        rate: super::FixedPoint,
        rate_floor: super::FixedPoint,
        unit_scale: OrderRiskScale,
    ) -> StateResult<()> {
        let contribution = Self::pm_contribution(size, rate, rate_floor, unit_scale)?;

        if side == Side::Long {
            let next_size = self
                .sum_long_order_size
                .checked_add(size)
                .ok_or(StateError::ArithmeticOverflow)?;

            let next_pm = self
                .sum_long_order_pm
                .to_raw_bits()
                .checked_add(contribution)
                .ok_or(StateError::ArithmeticOverflow)?;

            self.sum_long_order_size = next_size;
            self.sum_long_order_pm = super::FixedPoint::from_raw_bits(next_pm);
        } else {
            let next_size = self
                .sum_short_order_size
                .checked_add(size)
                .ok_or(StateError::ArithmeticOverflow)?;

            let next_pm = self
                .sum_short_order_pm
                .to_raw_bits()
                .checked_add(contribution)
                .ok_or(StateError::ArithmeticOverflow)?;

            self.sum_short_order_size = next_size;
            self.sum_short_order_pm = super::FixedPoint::from_raw_bits(next_pm);
        }
        Ok(())
    }

    /// Removes remaining order risk from the selected side's cached aggregate.
    pub(crate) fn subtract(
        &mut self,
        side: Side,
        size: u64,
        rate: super::FixedPoint,
        rate_floor: super::FixedPoint,
        unit_scale: OrderRiskScale,
    ) -> StateResult<()> {
        let contribution = Self::pm_contribution(size, rate, rate_floor, unit_scale)?;

        let (current_size, current_pm) = if side == Side::Long {
            (
                self.sum_long_order_size,
                self.sum_long_order_pm.to_raw_bits(),
            )
        } else {
            (
                self.sum_short_order_size,
                self.sum_short_order_pm.to_raw_bits(),
            )
        };

        let next_size = current_size.checked_sub(size).ok_or(StateError::Conflict)?;

        // TODO: Why can't pm be negative?
        let next_pm = current_pm
            .checked_sub(contribution)
            .filter(|value| *value >= 0)
            .ok_or(StateError::Conflict)?;

        if side == Side::Long {
            self.sum_long_order_size = next_size;
            self.sum_long_order_pm = super::FixedPoint::from_raw_bits(next_pm);
        } else {
            self.sum_short_order_size = next_size;
            self.sum_short_order_pm = super::FixedPoint::from_raw_bits(next_pm);
        }

        Ok(())
    }
}
