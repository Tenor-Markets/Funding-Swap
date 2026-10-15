use anchor_lang::prelude::*;

use crate::instructions::materialize_subaccount::materialize_market;
use crate::state::{
    MarginGroup, MarketState, SettlementRatio, StateError, StateResult, Subaccount, MARKET_ACTIVE,
    MARKET_CLOSE_ONLY, MARKET_MATURED, MAX_MARKETS,
};

#[derive(Accounts)]
pub struct SettleMarketIntoCollateral<'info> {
    #[account(mut)]
    pub subaccount: AccountLoader<'info, Subaccount>,
    #[account(
        constraint = margin_group.load()?.is_active() @ StateError::InvalidStatus,
        constraint = subaccount.load()?.is_same_group_id(margin_group.load()?.group_id) @ StateError::InvalidGroupID
    )]
    pub margin_group: AccountLoader<'info, MarginGroup>,
    #[account(mut)]
    pub market: AccountLoader<'info, MarketState>,
}

#[derive(Accounts)]
pub struct SettleAllMarketsIntoCollateral<'info> {
    #[account(mut)]
    pub subaccount: AccountLoader<'info, Subaccount>,
    #[account(
        constraint = margin_group.load()?.is_active() @ StateError::InvalidStatus,
        constraint = subaccount.load()?.is_same_group_id(margin_group.load()?.group_id) @ StateError::InvalidGroupID
    )]
    pub margin_group: AccountLoader<'info, MarginGroup>,
}

#[derive(Accounts)]
pub struct SettleMaturedMarkets<'info> {
    #[account(mut)]
    pub subaccount: AccountLoader<'info, Subaccount>,
    #[account(
        constraint = margin_group.load()?.is_active() @ StateError::InvalidStatus,
        constraint = subaccount.load()?.is_same_group_id(margin_group.load()?.group_id) @ StateError::InvalidGroupID
    )]
    pub margin_group: AccountLoader<'info, MarginGroup>,
}

fn slot_is_empty(subaccount: &Subaccount, slot_index: usize) -> bool {
    let slot = &subaccount.markets[slot_index];

    slot.position_size == 0 && slot.settlement_balance_atoms == 0 && slot.order_risk.is_empty()
}

fn market_account_for_slot<'info>(
    subaccount: &Subaccount,
    slot_index: usize,
    remaining_accounts: &'info [AccountInfo<'info>],
) -> Result<&'info AccountInfo<'info>> {
    let market_id = subaccount.markets[slot_index].market_id;

    for account_info in remaining_accounts {
        let loader = AccountLoader::<MarketState>::try_from(account_info)
            .map_err(|_| StateError::InvalidAccountInfo)?;

        let market = loader.load().map_err(|_| StateError::InvalidMarket)?;

        if market.config.market_id == market_id {
            return Ok(account_info);
        }
    }

    Err(StateError::InvalidMarket.into())
}

fn prepare_market_settlement(
    subaccount: &mut Subaccount,
    slot_index: usize,
    market: &mut MarketState,
    collateral_state: Pubkey,
    settlement_state: Pubkey,
) -> StateResult<()> {
    if !matches!(
        market.status.0,
        MARKET_ACTIVE | MARKET_CLOSE_ONLY | MARKET_MATURED
    ) {
        return Err(StateError::InvalidStatus);
    }

    // The group's price and collateral ledger are only meaningful for a market
    // that uses those same assets. This is the `ensure_market_assets_match` rule.
    if market.config.collateral_state != collateral_state
        || market.config.settlement_state != settlement_state
    {
        return Err(StateError::Conflict);
    }

    materialize_market(subaccount, market, slot_index)?;

    if subaccount.markets[slot_index].settlement_balance_atoms == 0 {
        return Ok(());
    }

    // A live oracle price would let the two sides of one payment convert at
    // different rates. The same asset converts one-for-one. Different assets
    // wait for the single frozen maturity ratio.
    let ratio = if market.is_matured() {
        market
            .frozen_settlement_ratio()
            .ok_or(StateError::InvalidStatus)?
    } else if collateral_state == settlement_state {
        SettlementRatio::new(1, 1)?
    } else {
        return Ok(());
    };

    subaccount.settle_market_into_collateral(slot_index, market, ratio)?;

    Ok(())
}

fn reject_unused_price_accounts(accounts: &[AccountInfo<'_>]) -> Result<()> {
    if accounts.is_empty() {
        Ok(())
    } else {
        Err(StateError::InvalidRemainingAccounts.into())
    }
}

struct SuppliedMarkets<'info> {
    loaders: [Option<AccountLoader<'info, MarketState>>; MAX_MARKETS],
    count: usize,
}

/// Market accounts come first. Parsing stops at the first account that is not
/// a `MarketState`, which is where the current-price pair begins when needed.
fn read_supplied_markets<'info>(
    mut accounts: &'info [AccountInfo<'info>],
) -> Result<(SuppliedMarkets<'info>, &'info [AccountInfo<'info>])> {
    let mut supplied = SuppliedMarkets {
        loaders: core::array::from_fn(|_| None),
        count: 0,
    };

    loop {
        let Some(account) = accounts.first() else {
            break;
        };

        let Ok(loader) = AccountLoader::<MarketState>::try_from(account) else {
            break;
        };

        if supplied.count == MAX_MARKETS {
            return Err(StateError::InvalidRemainingAccounts.into());
        }

        accounts = &accounts[1..];
        supplied.loaders[supplied.count] = Some(loader);
        supplied.count += 1;
    }

    Ok((supplied, accounts))
}

fn reject_duplicate_markets(supplied: &SuppliedMarkets<'_>) -> Result<()> {
    let mut market_ids = [0u64; MAX_MARKETS];

    for (index, loader) in supplied.loaders.iter().enumerate().take(supplied.count) {
        let loader = loader.as_ref().ok_or(StateError::InvalidMarket)?;
        let market = loader.load().map_err(|_| StateError::InvalidMarket)?;
        let market_id = market.config.market_id;

        if market_ids[..index].contains(&market_id) {
            return Err(StateError::InvalidRemainingAccounts.into());
        }

        market_ids[index] = market_id;
    }

    Ok(())
}

/// Materializes one slot and converts its settlement balance into collateral.
///
/// The same collateral and settlement asset converts one-for-one. A matured
/// market uses its frozen ratio. A different settlement asset stays in the
/// slot until that ratio is frozen. No oracle accounts are read.
pub fn handle_settle_market_into_collateral<'info>(
    context: Context<'info, SettleMarketIntoCollateral<'info>>,
) -> Result<()> {
    let (collateral_state, settlement_state) = {
        let group = context.accounts.margin_group.load()?;

        (group.collateral_state, group.settlement_state)
    };

    let slot_index = {
        let subaccount = context.accounts.subaccount.load()?;
        let market = context.accounts.market.load()?;

        subaccount
            .lookup_market(market.config.market_id)
            .ok_or(StateError::InvalidMarket)?
    };

    {
        let mut subaccount = context.accounts.subaccount.load_mut()?;
        let mut market = context.accounts.market.load_mut()?;

        prepare_market_settlement(
            &mut subaccount,
            slot_index,
            &mut market,
            collateral_state,
            settlement_state,
        )?;
    }

    reject_unused_price_accounts(context.remaining_accounts)
}

/// Materializes every supplied market. Market accounts are the whole
/// remaining-account list. Settlement converts under the same rules as the
/// single-market instruction.
pub fn handle_settle_all_markets_into_collateral<'info>(
    context: Context<'info, SettleAllMarketsIntoCollateral<'info>>,
) -> Result<()> {
    let (collateral_state, settlement_state) = {
        let group = context.accounts.margin_group.load()?;

        (group.collateral_state, group.settlement_state)
    };

    let (supplied, extra_accounts) = read_supplied_markets(context.remaining_accounts)?;

    reject_duplicate_markets(&supplied)?;
    reject_unused_price_accounts(extra_accounts)?;

    let mut subaccount = context.accounts.subaccount.load_mut()?;

    for loader in supplied.loaders.iter().take(supplied.count) {
        let loader = loader.as_ref().ok_or(StateError::InvalidMarket)?;
        let mut market = loader.load_mut().map_err(|_| StateError::InvalidMarket)?;

        let slot_index = subaccount
            .lookup_market(market.config.market_id)
            .ok_or(StateError::InvalidMarket)?;

        prepare_market_settlement(
            &mut subaccount,
            slot_index,
            &mut market,
            collateral_state,
            settlement_state,
        )?;
    }

    Ok(())
}

pub fn handle_settle_matured_markets<'info>(
    context: Context<'info, SettleMaturedMarkets<'info>>,
) -> Result<()> {
    let mut subaccount = context.accounts.subaccount.load_mut()?;
    for slot_index in 0..MAX_MARKETS {
        if subaccount.markets[slot_index].market_id == 0 {
            continue;
        }

        let market_info =
            market_account_for_slot(&subaccount, slot_index, context.remaining_accounts)?;

        let loader = AccountLoader::<MarketState>::try_from(market_info)
            .map_err(|_| StateError::InvalidAccountInfo)?;

        let mut market = loader.load_mut().map_err(|_| StateError::InvalidMarket)?;

        if market.is_matured() {
            materialize_market(&mut subaccount, &mut market, slot_index)?;

            if !subaccount.markets[slot_index].order_risk.is_empty() {
                market.close_all_orders(&mut subaccount, slot_index)?;
            }

            if slot_is_empty(&subaccount, slot_index) {
                subaccount.close_market(slot_index, &mut market)?;
            } else {
                subaccount.settle_and_close_market(slot_index, &mut market)?;
            }
        }
    }

    Ok(())
}
