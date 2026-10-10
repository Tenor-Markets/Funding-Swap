// CHECKED
use anchor_lang::prelude::*;

use crate::instructions::materialize_subaccount::materialize_market;
use crate::instructions::oracle::{market_observation_for, read_observation_set, ObservationSet};
use crate::state::{
    MarginGroup, MarketState, PortfolioMarketObservation, StateError, Subaccount, MAX_MARKETS,
};

#[derive(Accounts)]
pub struct GroupCleanup<'info> {
    #[account(mut)]
    pub subaccount: AccountLoader<'info, Subaccount>,
    #[account(
        constraint = margin_group.load()?.is_active() @ StateError::InvalidStatus,
        constraint = subaccount.load()?.is_same_group_id(margin_group.load()?.group_id) @ StateError::InvalidGroupID
    )]
    pub margin_group: AccountLoader<'info, MarginGroup>,
}

pub fn handle_risky_order_cleanup<'info>(
    context: Context<'info, GroupCleanup<'info>>,
) -> Result<()> {
    if context.remaining_accounts.is_empty() {
        return Ok(());
    }
    let now = Clock::get()?.unix_timestamp;
    let group = context.accounts.margin_group.load()?;

    let (observations, _) = read_observation_set::<{crate::state::MAX_MARKETS}>(
        context.remaining_accounts,
        group.collateral_state,
        group.settlement_state,
        now,
    )?;

    if observations.count != 0 {
        let mut subaccount = context.accounts.subaccount.load_mut()?;

        for slot_index in 0..MAX_MARKETS {
            let market_id = subaccount.markets[slot_index].market_id;

            if market_id == 0 {
                continue;
            }

            let Ok(entry) = market_observation_for(&observations, market_id) else {
                continue;
            };

            let mut market = entry
                .loader
                .load_mut()
                .map_err(|_| StateError::InvalidMarket)?;

            materialize_market(&mut subaccount, &mut market, slot_index)?;
        }

        return cleanup_with_transient_assets(&group, &mut subaccount, &observations, now);
    }

    Ok(())
}

#[inline(never)]
fn cleanup_with_transient_assets<'info>(
    group: &MarginGroup,
    subaccount: &mut Subaccount,
    observations: &ObservationSet<'info>,
    now: i64,
) -> Result<()> {
    let cleanup_eligible = {
        let collateral_guard = observations
            .collateral
            .loader
            .load()
            .map_err(|_| StateError::InvalidAssetState)?;

        let settlement_guard = observations
            .settlement
            .loader
            .load()
            .map_err(|_| StateError::InvalidAssetState)?;

        let mut market_guards: [Option<std::cell::Ref<'_, MarketState>>; MAX_MARKETS] =
            core::array::from_fn(|_| None);

        let mut underlying_guards: [Option<std::cell::Ref<'_, crate::state::AssetState>>;
            MAX_MARKETS] = core::array::from_fn(|_| None);

        for (slot_index, slot) in subaccount.markets.iter().enumerate() {
            if slot.market_id != 0 {
                let entry = market_observation_for(observations, slot.market_id)?;

                market_guards[slot_index] =
                    Some(entry.loader.load().map_err(|_| StateError::InvalidMarket)?);

                underlying_guards[slot_index] = Some(
                    entry
                        .underlying
                        .loader
                        .load()
                        .map_err(|_| StateError::InvalidAssetState)?,
                );
            }
        }

        let mut portfolio: [Option<PortfolioMarketObservation<'_>>; MAX_MARKETS] =
            [None; MAX_MARKETS];

        for (slot_index, slot) in subaccount.markets.iter().enumerate() {
            if slot.market_id != 0 {
                let entry = market_observation_for(observations, slot.market_id)?;

                let market = market_guards[slot_index]
                    .as_ref()
                    .ok_or(StateError::InvalidReference)?;

                let underlying_asset = underlying_guards[slot_index]
                    .as_ref()
                    .ok_or(StateError::InvalidReference)?;

                portfolio[slot_index] = Some(PortfolioMarketObservation {
                    market,
                    underlying_asset,
                    underlying: entry.underlying.observation,
                });
            }
        }

        let health = group.maintenance_portfolio_health_with_assets(
            subaccount,
            &collateral_guard,
            observations.collateral.observation,
            &settlement_guard,
            observations.settlement.observation,
            portfolio.iter().flatten().copied(),
            now,
        )?;

        health.risky_order_cleanup_eligible(group.risky_order_cancellation_ratio)?
    };

    if cleanup_eligible {
        let mut guards: [Option<std::cell::RefMut<'_, MarketState>>; MAX_MARKETS] =
            core::array::from_fn(|_| None);

        let mut count = 0usize;

        for slot_index in 0..MAX_MARKETS {
            let market_id = subaccount.markets[slot_index].market_id;

            if market_id != 0 {
                let Some(entry) = observations.markets[..observations.count]
                    .iter()
                    .flatten()
                    .find(|entry| entry.market_id == market_id)
                else {
                    continue;
                };

                guards[count] = Some(entry.loader.load_mut()?);

                count += 1;
            }
        }

        let mut refs: [std::mem::MaybeUninit<&mut MarketState>; MAX_MARKETS] =
            core::array::from_fn(|_| std::mem::MaybeUninit::uninit());

        for index in 0..count {
            let guard = guards[index].as_mut().ok_or(StateError::InvalidReference)?;

            let pointer = (&mut **guard) as *mut MarketState;
            // SAFETY: each pointer is derived from a live, unique RefMut held
            // in `guards` until the borrowed slice is no longer used.
            refs[index].write(unsafe { &mut *pointer });
        }

        let market_slice = unsafe {
            std::slice::from_raw_parts_mut(refs.as_mut_ptr() as *mut &mut MarketState, count)
        };

        MarginGroup::close_only_cleanup(subaccount, market_slice)?;
    }
    Ok(())
}

pub fn handle_close_only_cleanup<'info>(
    context: Context<'info, GroupCleanup<'info>>,
) -> Result<()> {
    let mut subaccount = context.accounts.subaccount.load_mut()?;

    for info in context.remaining_accounts {
        let loader = AccountLoader::<MarketState>::try_from(info)
            .map_err(|_| error!(StateError::InvalidAccountInfo))?;

        let mut market = loader
            .load_mut()
            .map_err(|_| error!(StateError::InvalidMarket))?;

        if !market.is_close_only() {
            return Err(error!(StateError::InvalidStatus));
        }

        let Some(slot_index) = subaccount.lookup_market(market.config.market_id) else {
            continue;
        };
        materialize_market(&mut subaccount, &mut market, slot_index)?;

        market.close_only_cleanup(&mut subaccount)?;
    }
    Ok(())
}
