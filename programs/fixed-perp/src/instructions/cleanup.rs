use anchor_lang::prelude::*;

use crate::instructions::materialize_subaccount::materialize_market;
use crate::instructions::oracle::{market_observation_for, read_observation_set, ObservationSet};
use crate::state::{MarginGroup, MarketState, StateError, Subaccount, MAX_MARKETS};

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

    let (observations, _) =
        read_observation_set::<{ crate::state::MAX_MARKETS }>(context.remaining_accounts, now)?;

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

            let mut market = entry.loader.load_mut()?;

            materialize_market(&mut subaccount, &mut market, slot_index)?;
        }

        return cleanup_with_observations(&group, &mut subaccount, &observations, now);
    }

    Ok(())
}

#[inline(never)]
fn cleanup_with_observations<'info>(
    group: &MarginGroup,
    subaccount: &mut Subaccount,
    observations: &ObservationSet<'info>,
    now: i64,
) -> Result<()> {
    let cleanup_eligible = {
        let mut market_guards: [Option<std::cell::Ref<'_, MarketState>>; MAX_MARKETS] =
            core::array::from_fn(|_| None);

        for (slot_index, slot) in subaccount.markets.iter().enumerate() {
            if slot.market_id != 0 {
                let entry = market_observation_for(observations, slot.market_id)?;

                market_guards[slot_index] = Some(entry.loader.load()?);
            }
        }

        let mut portfolio: [Option<&MarketState>; MAX_MARKETS] = [None; MAX_MARKETS];

        for (slot_index, slot) in subaccount.markets.iter().enumerate() {
            if slot.market_id != 0 {
                let market = market_guards[slot_index]
                    .as_ref()
                    .ok_or(StateError::MissingMarketObservation)?;

                portfolio[slot_index] = Some(market);
            }
        }

        let health = group.maintenance_portfolio_health(
            subaccount,
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

        MarginGroup::close_only_cleanup(
            subaccount,
            guards.iter_mut().flatten().map(|guard| &mut **guard),
        )?;
    }
    Ok(())
}

pub fn handle_close_only_cleanup<'info>(
    context: Context<'info, GroupCleanup<'info>>,
) -> Result<()> {
    let mut subaccount = context.accounts.subaccount.load_mut()?;

    for info in context.remaining_accounts {
        let loader = AccountLoader::<MarketState>::try_from(info)?;

        let mut market = loader.load_mut()?;

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
