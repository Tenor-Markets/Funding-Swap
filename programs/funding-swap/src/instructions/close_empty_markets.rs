use anchor_lang::prelude::*;

use crate::state::{MarginGroup, MarketState, StateError, Subaccount, MAX_MARKETS};

#[derive(Accounts)]
pub struct CloseEmptyMarkets<'info> {
    pub authority: Signer<'info>,
    #[account(
        mut,
        constraint = subaccount.load()?.is_authority_or_delegate(authority.key()) @ StateError::InvalidAuthority
    )]
    pub subaccount: AccountLoader<'info, Subaccount>,
    #[account(
        constraint = margin_group.load()?.is_active() @ StateError::InvalidStatus,
        constraint = subaccount.load()?.is_same_group_id(margin_group.load()?.group_id) @ StateError::InvalidGroupID
    )]
    pub margin_group: AccountLoader<'info, MarginGroup>,
}

pub(crate) fn slot_is_empty(subaccount: &Subaccount, slot_index: usize) -> bool {
    let slot = &subaccount.markets[slot_index];

    slot.position_size == 0 && slot.settlement_balance_atoms == 0 && slot.order_risk.is_empty()
}

fn close_market_for_slot<'info>(
    subaccount: &mut Subaccount,
    slot_index: usize,
    remaining_accounts: &'info [AccountInfo<'info>],
) -> Result<()> {
    let market_id = subaccount.markets[slot_index].market_id;

    for account_info in remaining_accounts {
        let loader = AccountLoader::<MarketState>::try_from(account_info)?;

        let mut market = loader.load_mut()?;

        if market.is_paused() {
            return Err(StateError::InvalidStatus.into());
        }

        if market.config.market_id == market_id {
            subaccount.close_market(slot_index, &mut market)?;

            return Ok(());
        }
    }

    Err(StateError::MissingMarketAccount.into())
}

/// Closes only market slots whose position, settlement balance, and order
/// sizes are already zero.  It does not materialize positions or orders.
pub fn close_empty_markets_from_accounts<'info>(
    subaccount: &mut Subaccount,
    remaining_accounts: &'info [AccountInfo<'info>],
) -> Result<()> {
    for slot_index in 0..MAX_MARKETS {
        if subaccount.markets[slot_index].market_id != 0 && slot_is_empty(subaccount, slot_index) {
            close_market_for_slot(subaccount, slot_index, remaining_accounts)?;
        }
    }

    Ok(())
}

/// Cleans every empty market slot using writable market accounts supplied in
/// `remaining_accounts`, returning whether all subaccount slots are empty.
pub fn handle_close_empty_markets<'info>(
    context: Context<'info, CloseEmptyMarkets<'info>>,
) -> Result<()> {
    let mut subaccount = context.accounts.subaccount.load_mut()?;

    close_empty_markets_from_accounts(&mut subaccount, context.remaining_accounts)
}
