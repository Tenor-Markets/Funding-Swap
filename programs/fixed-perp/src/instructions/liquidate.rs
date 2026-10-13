use std::cell::Ref;

use anchor_lang::prelude::*;

use crate::instructions::materialize_subaccount::materialize_market;
use crate::instructions::oracle::{market_observation_for, read_observation_set, ObservationSet};
use crate::state::{
    AssetState, MarginGroup, MarketState, PortfolioHealth, PortfolioMarketObservation, StateError,
    Subaccount, MAX_MARKETS,
};

#[derive(Accounts)]
pub struct Liquidate<'info> {
    pub liquidator_authority: Signer<'info>,
    #[account(
        mut,
        constraint = liquidator.load()?.is_authority(liquidator_authority.key()) @ StateError::InvalidAuthority,
        constraint = liquidator.load()?.is_same_group_id(margin_group.load()?.group_id) @ StateError::InvalidGroupID
    )]
    pub liquidator: AccountLoader<'info, Subaccount>,
    #[account(
        mut,
        constraint = victim.key() != liquidator.key() @ StateError::Conflict,
        constraint = victim.load()?.is_same_group_id(margin_group.load()?.group_id) @ StateError::InvalidGroupID
    )]
    pub victim: AccountLoader<'info, Subaccount>,
    #[account(constraint = margin_group.load()?.is_active() @ StateError::InvalidStatus)]
    pub margin_group: AccountLoader<'info, MarginGroup>,
}

#[event]
pub struct Liquidation {
    pub liquidator: Pubkey,
    pub victim: Pubkey,
    pub market_id: u64,
    pub size_to_liquidator: i64,
    pub incentive_atoms: i128,
    pub fee_atoms: u128,
}

pub fn handle_liquidate<'info>(
    context: Context<'info, Liquidate<'info>>,
    market_id: u64,
    size_to_liquidator: i64,
) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let group = context.accounts.margin_group.load()?;
    let (observations, _) = read_observation_set::<{crate::state::MAX_MARKETS}>(
        context.remaining_accounts,
        group.collateral_state,
        group.settlement_state,
        now,
    )?;
    let entry = market_observation_for(&observations, market_id)?;
    {
        let market = entry.loader.load()?;
        group.ensure_market_assets_match(&market)?;
    }

    {
        let mut victim = context.accounts.victim.load_mut()?;
        let mut liquidator = context.accounts.liquidator.load_mut()?;
        materialize_entered(&mut victim, &observations)?;
        materialize_entered(&mut liquidator, &observations)?;
    }

    let health_ratio = {
        let victim = context.accounts.victim.load()?;
        maintenance_health(&group, &victim, &observations, now)?.liquidation_health_ratio()?
    };
    let pre_liquidator_position = {
        let liquidator = context.accounts.liquidator.load()?;
        liquidator
            .lookup_market(market_id)
            .map(|slot| liquidator.markets[slot].position_size)
            .unwrap_or(0)
    };

    let (incentive, fee, liquidator_cash) = {
        let mut victim = context.accounts.victim.load_mut()?;
        let mut liquidator = context.accounts.liquidator.load_mut()?;
        let mut market = entry
            .loader
            .load_mut()
            .map_err(|_| StateError::InvalidMarket)?;
        market.liquidate(
            &mut victim,
            &mut liquidator,
            size_to_liquidator,
            health_ratio,
            now,
        )?
    };

    {
        let liquidator = context.accounts.liquidator.load()?;
        let health = initial_health(&group, &liquidator, &observations, now)?;
        if !health.strict_im_met()? {
            let market = entry.loader.load()?;
            let slot = liquidator
                .lookup_market(market_id)
                .ok_or(StateError::NotFound)?;
            let post = &liquidator.markets[slot];
            let action = market.closing_action(
                pre_liquidator_position,
                &post.order_risk,
                post.position_size,
                &post.order_risk,
                None,
                post.close_only_exempt != 0,
                group.critical_health_ratio,
                liquidator_cash,
                None,
                now,
            )?;
            if action.requires_strict_im {
                return Err(StateError::InsufficientInitialMargin.into());
            }
        }
    }

    emit!(Liquidation {
        liquidator: context.accounts.liquidator.key(),
        victim: context.accounts.victim.key(),
        market_id,
        size_to_liquidator,
        incentive_atoms: incentive,
        fee_atoms: fee,
    });
    Ok(())
}

pub(crate) fn materialize_entered<'info, const N: usize>(
    subaccount: &mut Subaccount,
    observations: &ObservationSet<'info, N>,
) -> Result<()> {
    for slot_index in 0..MAX_MARKETS {
        let market_id = subaccount.markets[slot_index].market_id;
        if market_id == 0 {
            continue;
        }
        let entry = market_observation_for(observations, market_id)?;
        let mut market = entry
            .loader
            .load_mut()
            .map_err(|_| StateError::InvalidMarket)?;
        materialize_market(subaccount, &mut market, slot_index)?;
    }
    Ok(())
}

pub(crate) fn maintenance_health<'info, const N: usize>(
    group: &MarginGroup,
    subaccount: &Subaccount,
    observations: &ObservationSet<'info, N>,
    now: i64,
) -> Result<PortfolioHealth> {
    portfolio_health(group, subaccount, observations, now, true)
}

fn initial_health<'info, const N: usize>(
    group: &MarginGroup,
    subaccount: &Subaccount,
    observations: &ObservationSet<'info, N>,
    now: i64,
) -> Result<PortfolioHealth> {
    portfolio_health(group, subaccount, observations, now, false)
}

fn portfolio_health<'info, const N: usize>(
    group: &MarginGroup,
    subaccount: &Subaccount,
    observations: &ObservationSet<'info, N>,
    now: i64,
    maintenance: bool,
) -> Result<PortfolioHealth> {
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
    let mut market_guards: [Option<Ref<'_, MarketState>>; MAX_MARKETS] =
        core::array::from_fn(|_| None);
    let mut underlying_guards: [Option<Ref<'_, AssetState>>; MAX_MARKETS] =
        core::array::from_fn(|_| None);
    for (slot_index, slot) in subaccount.markets.iter().enumerate() {
        if slot.market_id == 0 {
            continue;
        }
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
    let mut portfolio: [Option<PortfolioMarketObservation<'_>>; MAX_MARKETS] = [None; MAX_MARKETS];
    for (slot_index, slot) in subaccount.markets.iter().enumerate() {
        if slot.market_id == 0 {
            continue;
        }
        let entry = market_observation_for(observations, slot.market_id)?;
        portfolio[slot_index] = Some(PortfolioMarketObservation {
            market: market_guards[slot_index]
                .as_ref()
                .ok_or(StateError::InvalidReference)?,
            underlying_asset: underlying_guards[slot_index]
                .as_ref()
                .ok_or(StateError::InvalidReference)?,
            underlying: entry.underlying.observation,
        });
    }
    let markets = portfolio.iter().flatten().copied();
    if maintenance {
        Ok(group.maintenance_portfolio_health_with_assets(
            subaccount,
            &collateral_guard,
            observations.collateral.observation,
            &settlement_guard,
            observations.settlement.observation,
            markets,
            now,
        )?)
    } else {
        Ok(group.initial_portfolio_health_with_assets(
            subaccount,
            &collateral_guard,
            observations.collateral.observation,
            &settlement_guard,
            observations.settlement.observation,
            markets,
            now,
        )?)
    }
}
