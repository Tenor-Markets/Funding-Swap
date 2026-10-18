use anchor_lang::prelude::*;

use crate::state::{MarketState, StateError, StateResult, Subaccount};

/// Materializes one live market slot. Pending references are walked in place
/// so swap-removal cannot skip a reference.
pub(crate) fn materialize_market(
    subaccount: &mut Subaccount,
    market: &mut MarketState,
    slot_index: usize,
) -> StateResult<()> {
    market.materialize_position_indices(subaccount, slot_index)?;

    let mut ref_index = 0;

    while ref_index < usize::from(subaccount.order_ref_count) {
        if usize::from(subaccount.order_refs[ref_index].slot_index) != slot_index {
            ref_index += 1;
            continue;
        }

        let before_count = subaccount.order_ref_count;
        market.materialize_pending_order(subaccount, ref_index)?;

        if subaccount.order_ref_count == before_count {
            ref_index += 1;
        }
    }

    market.reset_position_index_snapshots(subaccount, slot_index);

    Ok(())
}

#[derive(Accounts)]
pub struct MaterializeSubaccount<'info> {
    /// Materialization is permissionless.
    #[account(mut)]
    pub subaccount: AccountLoader<'info, Subaccount>,
}

/// Materializes each supplied typed market against its matching subaccount slot.
pub fn handle_materialize_subaccount<'info>(
    context: Context<'info, MaterializeSubaccount<'info>>,
) -> Result<()> {
    let mut subaccount = context.accounts.subaccount.load_mut()?;

    for account_info in context.remaining_accounts {
        let loader = AccountLoader::<MarketState>::try_from(account_info)?;

        let mut market = loader.load_mut()?;

        let slot_index = subaccount
            .lookup_market(market.config.market_id)
            .ok_or(StateError::MarketNotEntered)?;

        materialize_market(&mut subaccount, &mut market, slot_index)?;
    }

    Ok(())
}
