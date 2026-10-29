use anchor_lang::prelude::*;

use crate::instructions::materialize_subaccount::materialize_market;
use crate::state::{MarginGroup, MarketState, OrderId, StateError, Subaccount};

#[derive(Accounts)]
pub struct PurgeExpiredOrder<'info> {
    #[account(mut)]
    pub subaccount: AccountLoader<'info, Subaccount>,
    #[account(mut)]
    pub market: AccountLoader<'info, MarketState>,
    #[account(
        constraint = margin_group.load()?.is_active() @ StateError::InvalidStatus,
        constraint = subaccount.load()?.is_same_group_id(margin_group.load()?.group_id) @ StateError::InvalidGroupID
    )]
    pub margin_group: AccountLoader<'info, MarginGroup>,
}

/// Permissionlessly removes an expired resting order after materializing any
/// already executed maker fills.
pub fn handle_purge_expired_order<'info>(
    context: Context<'info, PurgeExpiredOrder<'info>>,
    sequence: u64,
    slab_index: u64,
) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;

    let mut subaccount = context.accounts.subaccount.load_mut()?;
    let mut market = context.accounts.market.load_mut()?;

    if !market.may_cancel_order() {
        return Err(StateError::InvalidStatus.into());
    }

    let slot_index = subaccount
        .lookup_market(market.config.market_id)
        .ok_or(StateError::MarketNotEntered)?;

    materialize_market(&mut subaccount, &mut market, slot_index, now)?;

    market.purge_expired_order(
        &mut subaccount,
        OrderId {
            sequence,
            slab_index,
        },
        now,
    )?;

    Ok(())
}
