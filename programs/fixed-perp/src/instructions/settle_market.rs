// CHECKED
use anchor_lang::prelude::*;

use crate::instructions::materialize_subaccount::materialize_market;
use crate::instructions::oracle::{market_observation_for, read_observation_set, settlement_ratio};
use crate::state::{
    MarginGroup, MarketState, SettlementRatio, StateError, Subaccount, MARKET_ACTIVE,
    MARKET_CLOSE_ONLY, MARKET_MATURED, MAX_MARKETS,
};

#[derive(Accounts)]
pub struct SettleMarketIntoCollateral<'info> {
    pub authority: Signer<'info>,
    #[account(mut, constraint = subaccount.load()?.is_authority(authority.key()) @ StateError::InvalidAuthority)]
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
    pub authority: Signer<'info>,
    #[account(mut, constraint = subaccount.load()?.is_authority(authority.key()) @ StateError::InvalidAuthority)]
    pub subaccount: AccountLoader<'info, Subaccount>,
    #[account(
        constraint = margin_group.load()?.is_active() @ StateError::InvalidStatus,
        constraint = subaccount.load()?.is_same_group_id(margin_group.load()?.group_id) @ StateError::InvalidGroupID
    )]
    pub margin_group: AccountLoader<'info, MarginGroup>,
}

#[derive(Accounts)]
pub struct SettleMaturedMarkets<'info> {
    pub authority: Signer<'info>,
    #[account(mut, constraint = subaccount.load()?.is_authority(authority.key()) @ StateError::InvalidAuthority)]
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

fn settle_one_market(
    subaccount: &mut Subaccount,
    slot_index: usize,
    market: &mut MarketState,
    ratio: SettlementRatio,
) -> Result<()> {
    if !matches!(
        market.status.0,
        MARKET_ACTIVE | MARKET_CLOSE_ONLY | MARKET_MATURED
    ) {
        return Err(StateError::InvalidStatus.into());
    }

    materialize_market(subaccount, market, slot_index)?;

    if subaccount.markets[slot_index].settlement_balance_atoms == 0 {
        return Ok(());
    }

    subaccount.settle_market_into_collateral(slot_index, market, ratio)?;
    Ok(())
}

pub fn handle_settle_market_into_collateral<'info>(
    context: Context<'info, SettleMarketIntoCollateral<'info>>,
) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let group = context.accounts.margin_group.load()?;
    let mut subaccount = context.accounts.subaccount.load_mut()?;
    let mut market = context.accounts.market.load_mut()?;

    let slot_index = subaccount
        .lookup_market(market.config.market_id)
        .ok_or(StateError::InvalidMarket)?;

    let ratio = if market.is_matured() {
        market
            .frozen_settlement_ratio()
            .ok_or(StateError::InvalidStatus)?
    } else {
        let (observations, _) = read_observation_set(
            context.remaining_accounts,
            group.collateral_state,
            group.settlement_state,
            now,
        )?;
        settlement_ratio(&observations.collateral, &observations.settlement)?
    };

    settle_one_market(&mut subaccount, slot_index, &mut market, ratio)?;
    Ok(())
}

pub fn handle_settle_all_markets_into_collateral<'info>(
    context: Context<'info, SettleAllMarketsIntoCollateral<'info>>,
) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let group = context.accounts.margin_group.load()?;
    let mut subaccount = context.accounts.subaccount.load_mut()?;

    if !context.remaining_accounts.is_empty() {
        let (observations, _) = read_observation_set(
            context.remaining_accounts,
            group.collateral_state,
            group.settlement_state,
            now,
        )?;
        let current = settlement_ratio(&observations.collateral, &observations.settlement)?;

        for slot_index in 0..MAX_MARKETS {
            if subaccount.markets[slot_index].market_id == 0 {
                continue;
            }

            let market_id = subaccount.markets[slot_index].market_id;

            let Ok(entry) = market_observation_for(&observations, market_id) else {
                continue;
            };

            let mut market = entry
                .loader
                .load_mut()
                .map_err(|_| StateError::InvalidMarket)?;

            let ratio = if market.is_matured() {
                market
                    .frozen_settlement_ratio()
                    .ok_or(StateError::InvalidStatus)?
            } else {
                current
            };

            settle_one_market(&mut subaccount, slot_index, &mut market, ratio)?;
        }
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
