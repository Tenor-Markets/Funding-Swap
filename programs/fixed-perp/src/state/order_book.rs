// CHECKED
use super::{FixedPoint, StateError, StateResult, MAX_ORDERS};
use anchor_lang::prelude::*;
use bytemuck::{Pod, Zeroable};
use fixed_perp_math::MAX_TICK_STEP;

pub const TICK_METADATA_RESERVED: usize = 8;
pub const ORDER_NODE_RESERVED: usize = 16;
pub const ORDER_BOOK_RESERVED: usize = 64;
pub const BOOK_CAPACITY: usize = 1_024;
pub const TICK_DIRECTORY_LEN: usize = 65_536;
pub const FREE_INDEX: u16 = u16::MAX;

/// Maps the signed tick ABI to its shared bitmap and directory position.
/// Basically it starts counting from the lowest number(i16::min()-(two complement representation: 0x8000)-(this representation: 0x0000)) to
/// highest number(i16::max()-(two complement representation: 0x7fff)-(this representation: 0xffff)).
fn tick_index(tick: i16) -> usize {
    usize::from(u16::from_ne_bytes(tick.to_ne_bytes()) ^ 0x8000)
}

/// Restores a signed tick from its shared bitmap and directory position.
/// Reverse of the above.
fn tick_from_index(index: usize) -> i16 {
    i16::from_ne_bytes(((u16::try_from(index).expect("bounded book index")) ^ 0x8000).to_ne_bytes())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Side {
    Long,
    Short,
}

impl Side {
    /// Classifies a validated nonzero signed order size.
    pub(crate) fn from_size(size: i64) -> StateResult<Self> {
        if size > 0 {
            Ok(Self::Long)
        } else if size < 0 {
            Ok(Self::Short)
        } else {
            Err(StateError::ZeroOrderSize)
        }
    }
}

macro_rules! tag_type {
    ($name:ident) => {
        #[repr(transparent)]
        #[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Pod, Zeroable)]
        #[cfg_attr(feature = "idl-build", derive(AnchorSerialize, AnchorDeserialize))]
        pub struct $name(pub u8);
    };
}

tag_type!(TimeInForce);

impl TimeInForce {
    /// Takes available liquidity and rests any unfilled remainder.
    pub const GTC: Self = Self(0);
    /// Takes available liquidity and cancels any unfilled remainder.
    pub const IOC: Self = Self(1);
    /// Requires the entire order to fill immediately or fails.
    pub const FOK: Self = Self(2);
    /// Rests only if no immediate match is possible; otherwise fails.
    pub const ALO: Self = Self(3);
    /// Rests only if no immediate match is possible; otherwise cancels.
    pub const SOFT_ALO: Self = Self(4);

    /// Validates a time-in-force value received at a public state boundary.
    pub fn validate(self) -> StateResult<()> {
        if self.0 > Self::SOFT_ALO.0 {
            return Err(StateError::InvalidTimeInForce);
        }

        Ok(())
    }

    /// Returns whether this policy may leave an unmatched remainder on the book.
    pub fn rests_remainder(self) -> bool {
        matches!(self, Self::GTC | Self::ALO | Self::SOFT_ALO)
    }

    /// Returns whether the policy permits placement only when the order does not cross.
    pub fn is_post_only(self) -> bool {
        matches!(self, Self::ALO | Self::SOFT_ALO)
    }

    /// Returns whether crossing causes the entire post-only action to fail.
    pub fn rejects_crossing(self) -> bool {
        self == Self::ALO
    }

    /// Returns whether a crossing post-only action is skipped without failing.
    pub fn skips_crossing(self) -> bool {
        self == Self::SOFT_ALO
    }

    /// Returns whether the policy requires the entire requested size to fill immediately.
    pub fn is_fill_or_kill(self) -> bool {
        self == Self::FOK
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Pod, Zeroable)]
#[cfg_attr(feature = "idl-build", derive(AnchorSerialize, AnchorDeserialize))]
pub struct OrderId {
    pub sequence: u64,
    pub slab_index: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Pod, Zeroable)]
#[cfg_attr(feature = "idl-build", derive(AnchorSerialize, AnchorDeserialize))]
pub struct HierarchicalBitmap {
    pub level_zero: [u64; 1_024],
    pub level_one: [u64; 16],
    pub top: u64, // We only use the first 16 bits.
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Pod, Zeroable)]
#[cfg_attr(feature = "idl-build", derive(AnchorSerialize, AnchorDeserialize))]
pub struct TickMetadata {
    pub tick: i16,
    pub head: u16,
    pub tail: u16,
    pub next_free: u16,
    pub _pad_0: [u8; 1],
    pub reserved: [u8; TICK_METADATA_RESERVED],
    pub _pad_1: [u8; 1],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Pod, Zeroable)]
#[cfg_attr(feature = "idl-build", derive(AnchorSerialize, AnchorDeserialize))]
pub struct OrderNode {
    pub sequence: u64,
    pub expiry_timestamp: i64,
    pub size: i64,
    pub remaining_size: i64,
    pub pending_floating_index_basis: FixedPoint,
    pub pending_fee_index_basis: FixedPoint,
    pub pending_fixed_payment_atoms: i128,
    pub maker_subaccount: Pubkey,
    pub previous_node: u16,
    pub next_node: u16,
    pub tick: i16,
    pub time_in_force: TimeInForce,
    pub _pad_0: [u8; 3],
    pub reserved: [u8; ORDER_NODE_RESERVED],
    pub _pad_1: [u8; 6],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Pod, Zeroable)]
#[cfg_attr(feature = "idl-build", derive(AnchorSerialize, AnchorDeserialize))]
pub struct OrderBook {
    pub long_bitmap: HierarchicalBitmap,
    pub short_bitmap: HierarchicalBitmap,
    pub long_tick_directory: [u16; TICK_DIRECTORY_LEN],
    pub short_tick_directory: [u16; TICK_DIRECTORY_LEN],
    pub tick_metadata: [TickMetadata; BOOK_CAPACITY],
    pub nodes: [OrderNode; BOOK_CAPACITY],
    pub next_order_sequence: u64,
    /// Resting order bounds in underlying atoms.
    pub minimum_order_size_atoms: u64,
    pub maximum_order_size_atoms: u64,
    pub free_node_head: u16,
    pub free_tick_metadata_head: u16,
    pub maximum_open_orders_per_subaccount: u8,
    pub tick_step: u8,
    pub _pad_0: [u8; 2],
    pub reserved: [u8; ORDER_BOOK_RESERVED],
}

impl OrderBook {
    /// Validates a caller-provided resting size against this book's configured bounds.
    pub(crate) fn validate_order_size(&self, size: i64) -> StateResult<()> {
        let absolute_size = size.unsigned_abs();

        if absolute_size < self.minimum_order_size_atoms
            || absolute_size > self.maximum_order_size_atoms
        {
            return Err(StateError::OrderSizeOutOfBounds);
        }

        Ok(())
    }

    pub const DATA_LEN: usize = core::mem::size_of::<Self>();
}

impl OrderNode {
    /// Returns whether this node is free.
    pub fn is_free(&self) -> bool {
        self.size == 0
    }

    /// Returns whether this node can still receive a match.
    pub fn is_executable(&self) -> bool {
        self.remaining_size != 0
    }

    /// Returns whether this live node is fully matched and awaits materialization.
    pub fn is_awaiting_resolution(&self) -> bool {
        self.size != 0 && self.remaining_size == 0
    }

    /// Returns the matched signed amount, assuming remaining size preserves side and bounds.
    pub(crate) fn pending_fill_size(&self) -> i64 {
        self.size - self.remaining_size
    }

    /// Returns whether unresolved maker accounting still owns this node.
    pub(crate) fn has_pending_fill(&self) -> bool {
        self.pending_fill_size() != 0
    }

    /// Returns this live order's side; callers must not use it for a free node.
    pub fn get_side(&self) -> Side {
        if self.size > 0 {
            Side::Long
        } else {
            Side::Short
        }
    }
}

impl HierarchicalBitmap {
    /// Marks a price level present; callers add metadata before setting it.
    fn set(&mut self, tick: i16) {
        let mapped = tick_index(tick);

        let word = mapped / 64;
        let bit = mapped % 64;

        self.level_zero[word] |= 1u64 << bit;

        let upper_word = word / 64;

        self.level_one[upper_word] |= 1u64 << (word % 64);

        self.top |= 1u64 << upper_word;
    }

    /// Marks an empty price level absent after its metadata is released.
    fn clear(&mut self, tick: i16) {
        let mapped = tick_index(tick);

        let word = mapped / 64;
        let bit = mapped % 64;

        self.level_zero[word] &= !(1u64 << bit);

        if self.level_zero[word] == 0 {
            let upper_word = word / 64;

            self.level_one[upper_word] &= !(1u64 << (word % 64));

            if self.level_one[upper_word] == 0 {
                self.top &= !(1u64 << upper_word);
            }
        }
    }

    /// Returns whether this bitmap contains a tick; retained for bounded callers.
    pub fn contains(&self, tick: i16) -> bool {
        let mapped = tick_index(tick);

        self.level_zero[mapped / 64] & (1u64 << (mapped % 64)) != 0
    }

    /// Returns the lowest occupied tick, or None for an empty bitmap.
    fn best_lowest(&self) -> Option<i16> {
        if self.top == 0 {
            return None;
        }

        let upper_word =
            usize::try_from(self.top.trailing_zeros()).expect("bitmap bit index fits usize");
        let level_one = self.level_one[upper_word];

        let word = upper_word * 64
            + usize::try_from(level_one.trailing_zeros()).expect("bitmap bit index fits usize");
        let level_zero = self.level_zero[word];

        let mapped = word * 64
            + usize::try_from(level_zero.trailing_zeros()).expect("bitmap bit index fits usize");

        Some(tick_from_index(mapped))
    }

    /// Returns the highest occupied tick, or None for an empty bitmap.
    fn best_highest(&self) -> Option<i16> {
        if self.top == 0 {
            return None;
        }

        let upper_word =
            63 - usize::try_from(self.top.leading_zeros()).expect("bitmap bit index fits usize");
        let level_one = self.level_one[upper_word];

        let word = upper_word * 64
            + (63
                - usize::try_from(level_one.leading_zeros()).expect("bitmap bit index fits usize"));
        let level_zero = self.level_zero[word];

        let mapped = word * 64
            + (63
                - usize::try_from(level_zero.leading_zeros())
                    .expect("bitmap bit index fits usize"));

        Some(tick_from_index(mapped))
    }
}

impl OrderBook {
    /// Initializes a fresh zeroed book and its free lists from validated limits.
    pub(crate) fn initialize_in_place(
        &mut self,
        tick_step: u8,
        minimum_order_size_atoms: u64,
        maximum_order_size_atoms: u64,
        maximum_open_orders_per_subaccount: u8,
    ) -> StateResult<()> {
        if tick_step == 0 || tick_step > MAX_TICK_STEP {
            return Err(StateError::InvalidTickStep);
        }
        if minimum_order_size_atoms == 0
            || minimum_order_size_atoms > maximum_order_size_atoms
            || maximum_order_size_atoms
                > u64::try_from(i64::MAX).expect("nonnegative signed maximum")
            || maximum_open_orders_per_subaccount == 0
            || usize::from(maximum_open_orders_per_subaccount) > MAX_ORDERS
        {
            return Err(StateError::InvalidOrderBookLimits);
        }

        // Bind every field without constructing a book-sized stack temporary.
        let Self {
            long_bitmap: stored_long_bitmap,
            short_bitmap: stored_short_bitmap,
            long_tick_directory: stored_long_tick_directory,
            short_tick_directory: stored_short_tick_directory,
            tick_metadata: stored_tick_metadata,
            nodes: stored_nodes,
            next_order_sequence: stored_next_order_sequence,
            minimum_order_size_atoms: stored_minimum_order_size_atoms,
            maximum_order_size_atoms: stored_maximum_order_size_atoms,
            free_node_head: stored_free_node_head,
            free_tick_metadata_head: stored_free_tick_metadata_head,
            maximum_open_orders_per_subaccount: stored_maximum_open_orders_per_subaccount,
            tick_step: stored_tick_step,
            _pad_0: stored_pad_0,
            reserved: stored_reserved,
        } = self;
        for bitmap in [stored_long_bitmap, stored_short_bitmap] {
            let HierarchicalBitmap {
                level_zero,
                level_one,
                top,
            } = bitmap;
            level_zero.fill(0);
            level_one.fill(0);
            *top = 0;
        }
        stored_long_tick_directory.fill(FREE_INDEX);
        stored_short_tick_directory.fill(FREE_INDEX);
        for index in 0..BOOK_CAPACITY {
            stored_tick_metadata[index] = TickMetadata {
                tick: 0,
                head: FREE_INDEX,
                tail: FREE_INDEX,
                next_free: if index + 1 < BOOK_CAPACITY {
                    u16::try_from(index + 1).expect("bounded book index")
                } else {
                    FREE_INDEX
                },
                _pad_0: [0; 1],
                reserved: [0; TICK_METADATA_RESERVED],
                _pad_1: [0; 1],
            };
            stored_nodes[index] = OrderNode {
                sequence: 0,
                expiry_timestamp: 0,
                size: 0,
                remaining_size: 0,
                pending_floating_index_basis: FixedPoint::zeroed(),
                pending_fee_index_basis: FixedPoint::zeroed(),
                pending_fixed_payment_atoms: 0,
                maker_subaccount: Pubkey::default(),
                previous_node: FREE_INDEX,
                next_node: if index + 1 < BOOK_CAPACITY {
                    u16::try_from(index + 1).expect("bounded book index")
                } else {
                    FREE_INDEX
                },
                tick: 0,
                time_in_force: TimeInForce::default(),
                _pad_0: [0; 3],
                reserved: [0; ORDER_NODE_RESERVED],
                _pad_1: [0; 6],
            };
        }
        *stored_next_order_sequence = 1;
        *stored_minimum_order_size_atoms = minimum_order_size_atoms;
        *stored_maximum_order_size_atoms = maximum_order_size_atoms;
        *stored_free_node_head = 0;
        *stored_free_tick_metadata_head = 0;
        *stored_maximum_open_orders_per_subaccount = maximum_open_orders_per_subaccount;
        *stored_tick_step = tick_step;
        *stored_pad_0 = [0; 2];
        *stored_reserved = [0; ORDER_BOOK_RESERVED];

        Ok(())
    }

    /// Returns the directory for a known order side.
    fn directory(&self, side: Side) -> &[u16; TICK_DIRECTORY_LEN] {
        if side == Side::Long {
            &self.long_tick_directory
        } else {
            &self.short_tick_directory
        }
    }

    /// Returns the mutable directory for a known order side.
    fn directory_mut(&mut self, side: Side) -> &mut [u16; TICK_DIRECTORY_LEN] {
        if side == Side::Long {
            &mut self.long_tick_directory
        } else {
            &mut self.short_tick_directory
        }
    }

    /// Returns the mutable bitmap for a known order side.
    fn bitmap_mut(&mut self, side: Side) -> &mut HierarchicalBitmap {
        if side == Side::Long {
            &mut self.long_bitmap
        } else {
            &mut self.short_bitmap
        }
    }

    /// Returns a detached metadata record to the metadata free list.
    fn free_metadata(&mut self, index: usize) {
        self.tick_metadata[index] = TickMetadata {
            head: FREE_INDEX,
            tail: FREE_INDEX,
            next_free: self.free_tick_metadata_head,
            ..TickMetadata::zeroed()
        };

        self.free_tick_metadata_head = u16::try_from(index).expect("bounded book index");
    }

    /// Returns the active metadata for a side and tick, if the level exists.
    fn get_metadata(&self, side: Side, tick: i16) -> Option<usize> {
        let index = self.directory(side)[tick_index(tick)];

        (index != FREE_INDEX).then_some(usize::from(index))
    }

    /// Publishes a nonempty price level after the caller confirms metadata capacity.
    fn add_metadata(&mut self, side: Side, tick: i16, head: u16, tail: u16) {
        let index = usize::from(self.free_tick_metadata_head);
        let next_free = self.tick_metadata[index].next_free;
        self.free_tick_metadata_head = next_free;

        self.tick_metadata[index] = TickMetadata {
            tick,
            head,
            tail,
            next_free: FREE_INDEX,
            ..TickMetadata::zeroed()
        };

        self.directory_mut(side)[tick_index(tick)] =
            u16::try_from(index).expect("bounded book index");
    }

    /// Allocates a node and appends it to its side/tick FIFO.
    pub(crate) fn add_order(
        &mut self,
        maker_subaccount: Pubkey,
        size: i64,
        tick: i16,
        time_in_force: TimeInForce,
        expiry_timestamp: i64,
    ) -> StateResult<OrderId> {
        let side = if size > 0 { Side::Long } else { Side::Short };

        let metadata_index = self.get_metadata(side, tick);

        // Check if we still have some room.
        if self.free_node_head == FREE_INDEX
            || (metadata_index.is_none() && self.free_tick_metadata_head == FREE_INDEX)
        {
            return Err(StateError::BookCapacityExhausted);
        }

        let index = usize::from(self.free_node_head);
        let next_free = self.nodes[index].next_node;
        let sequence = self.next_order_sequence;

        let next_sequence = sequence
            .checked_add(1)
            .ok_or(StateError::ArithmeticOverflow)?;

        let order_id = OrderId {
            sequence,
            slab_index: u64::try_from(index).map_err(|_| StateError::ArithmeticOverflow)?,
        };

        self.free_node_head = next_free;
        self.next_order_sequence = next_sequence;

        self.nodes[index] = OrderNode {
            sequence,
            expiry_timestamp,
            size,
            remaining_size: size,
            maker_subaccount,
            tick,
            time_in_force,
            previous_node: FREE_INDEX,
            next_node: FREE_INDEX,
            ..OrderNode::zeroed()
        };

        if let Some(metadata_index) = metadata_index {
            let tail = self.tick_metadata[metadata_index].tail;
            self.nodes[index].previous_node = tail;
            self.nodes[usize::from(tail)].next_node =
                u16::try_from(index).expect("bounded book index");
            self.tick_metadata[metadata_index].tail =
                u16::try_from(index).expect("bounded book index");
        } else {
            self.add_metadata(
                side,
                tick,
                u16::try_from(index).expect("bounded book index"),
                u16::try_from(index).expect("bounded book index"),
            );
            self.bitmap_mut(side).set(tick);
        }

        Ok(order_id)
    }

    /// Sets a validated live node's total and unfilled sizes, moving an increased remainder last.
    pub(crate) fn set_order_sizes(
        &mut self,
        index: usize,
        size: i64,
        remaining_size: i64,
    ) -> StateResult<()> {
        if remaining_size.unsigned_abs() > self.nodes[index].remaining_size.unsigned_abs() {
            self.move_to_tail(index)?;
        }

        self.nodes[index].size = size;
        self.nodes[index].remaining_size = remaining_size;

        Ok(())
    }

    /// Commits materialized maker accounting into a still-resting order.
    pub(crate) fn commit_pending_fill(&mut self, index: usize) {
        let remaining = self.nodes[index].remaining_size;

        self.nodes[index].size = remaining;
        self.nodes[index].pending_floating_index_basis = FixedPoint::from_raw_bits(0);
        self.nodes[index].pending_fee_index_basis = FixedPoint::from_raw_bits(0);
        self.nodes[index].pending_fixed_payment_atoms = 0;
    }

    /// Moves a live order to its FIFO tail after an increase in unfilled size.
    fn move_to_tail(&mut self, index: usize) -> StateResult<()> {
        let side = self.nodes[index].get_side();
        let tick = self.nodes[index].tick;

        let previous = self.nodes[index].previous_node;
        let next = self.nodes[index].next_node;

        // A live node always has metadata, we keep an explicit error instead of unwrapping.
        let metadata_index = self
            .get_metadata(side, tick)
            .ok_or(StateError::MissingTickMetadata)?;

        if self.tick_metadata[metadata_index].tail
            == u16::try_from(index).expect("bounded book index")
        {
            return Ok(());
        }

        if previous == FREE_INDEX {
            self.tick_metadata[metadata_index].head = next;
        } else {
            self.nodes[usize::from(previous)].next_node = next;
        }

        // We handle the case where next == FREE_INDEX, by handling the case where the order is
        // the tail, that is done above.
        self.nodes[usize::from(next)].previous_node = previous;

        let previous_tail = self.tick_metadata[metadata_index].tail;

        self.nodes[index].previous_node = previous_tail;
        self.nodes[index].next_node = FREE_INDEX;
        self.nodes[usize::from(previous_tail)].next_node =
            u16::try_from(index).expect("bounded book index");
        self.tick_metadata[metadata_index].tail = u16::try_from(index).expect("bounded book index");

        Ok(())
    }

    /// Closes a validated, materialized node selected by its owning market transition.
    pub(crate) fn close_order(&mut self, index: usize) -> StateResult<()> {
        self.detach_from_fifo(index)?;
        self.release_detached_node(index);

        Ok(())
    }

    /// Removes a node from its active price-level FIFO without making it reusable.
    /// The caller is responsible for ensuring the metadata reference left in the order is not reused.
    pub(crate) fn detach_from_fifo(&mut self, index: usize) -> StateResult<()> {
        let side = self.nodes[index].get_side();
        let tick = self.nodes[index].tick;

        // A live node always has metadata, we keep an explicit error instead of unwrapping.
        let metadata_index = self
            .get_metadata(side, tick)
            .ok_or(StateError::MissingTickMetadata)?;

        let previous = self.nodes[index].previous_node;
        let next = self.nodes[index].next_node;

        // Detach the tick from the list, updating the metadata as well.
        if previous == FREE_INDEX {
            self.tick_metadata[metadata_index].head = next;
        } else {
            self.nodes[usize::from(previous)].next_node = next;
        }

        if next == FREE_INDEX {
            self.tick_metadata[metadata_index].tail = previous;
        } else {
            self.nodes[usize::from(next)].previous_node = previous;
        }

        self.nodes[index].previous_node = FREE_INDEX;
        self.nodes[index].next_node = FREE_INDEX;

        // If there are no more orders at that tick clear the metadata.
        if self.tick_metadata[metadata_index].head == FREE_INDEX {
            self.bitmap_mut(side).clear(tick);
            self.directory_mut(side)[tick_index(tick)] = FREE_INDEX;
            self.free_metadata(metadata_index);
        }

        Ok(())
    }

    /// Clears a caller-confirmed detached node and returns it to the free list.
    pub(crate) fn release_detached_node(&mut self, index: usize) {
        self.nodes[index] = OrderNode {
            previous_node: FREE_INDEX,
            next_node: self.free_node_head,
            ..OrderNode::zeroed()
        };

        self.free_node_head = u16::try_from(index).expect("bounded book index");
    }

    /// Returns the FIFO head at the best price, or None when the side is empty.
    /// Called with the maker side.
    pub(crate) fn best_head(&self, side: Side) -> StateResult<Option<usize>> {
        let Some(tick) = self.best_tick(side) else {
            return Ok(None);
        };

        // A live node always has metadata, we keep an explicit error instead of unwrapping.
        let metadata = self
            .get_metadata(side, tick)
            .ok_or(StateError::MissingTickMetadata)?;

        Ok(Some(usize::from(self.tick_metadata[metadata].head)))
    }

    /// Returns the best occupied tick, or None when the requested side is empty.
    /// Called with the maker side.
    pub(crate) fn best_tick(&self, side: Side) -> Option<i16> {
        if side == Side::Long {
            self.long_bitmap.best_highest()
        } else {
            self.short_bitmap.best_lowest()
        }
    }
}
