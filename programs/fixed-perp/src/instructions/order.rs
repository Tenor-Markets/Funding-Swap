use std::cell::Ref;

use anchor_lang::prelude::*;

use crate::instructions::materialize_subaccount::materialize_market;
use crate::instructions::oracle::{market_observation_for, read_observation_set, ObservationSet};
use crate::state::{
    MakerSubaccount, MarginGroup, MarketState, PlaceOrderIntent, PortfolioMarketObservation,
    StateError, Subaccount, TimeInForce, MAX_MARKETS, MAX_ORDERS,
};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug)]
pub struct PlaceOrderArgs {
    pub size: i64,
    pub limit_tick: i16,
    pub time_in_force: u8,
    pub expiry_timestamp: i64,
    pub maximum_payment_atoms: u128,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug)]
pub struct ModifyOrderArgs {
    pub sequence: u64,
    pub slab_index: u64,
    pub new_size: i64,
}

#[derive(Accounts)]
pub struct OrderAccounts<'info> {
    pub authority: Signer<'info>,
    #[account(mut, constraint = subaccount.load()?.is_authority_or_delegate(authority.key()) @ StateError::InvalidAuthority)]
    pub subaccount: AccountLoader<'info, Subaccount>,
    #[account(mut, constraint = market.load()?.may_use_order() @ StateError::InvalidStatus)]
    pub market: AccountLoader<'info, MarketState>,
    #[account(
        constraint = margin_group.load()?.is_active() @ StateError::InvalidStatus,
        constraint = subaccount.load()?.is_same_group_id(margin_group.load()?.group_id) @ StateError::InvalidGroupID
    )]
    pub margin_group: AccountLoader<'info, MarginGroup>,
}

#[inline(never)]
fn maker_accounts<'info>(
    remaining: &'info [AccountInfo<'info>],
) -> Result<(
    [Option<AccountLoader<'info, Subaccount>>; MAX_ORDERS],
    usize,
)> {
    if remaining.len() > MAX_ORDERS {
        return Err(error!(StateError::TooManyMakerAccounts));
    }

    let mut loaders: [Option<AccountLoader<'info, Subaccount>>; MAX_ORDERS] =
        core::array::from_fn(|_| None);

    for (index, info) in remaining.iter().enumerate() {
        let loader = AccountLoader::<Subaccount>::try_from(info)?;

        loaders[index] = Some(loader);
    }

    Ok((loaders, remaining.len()))
}

#[inline(never)]
fn check_initial_health<'info>(
    group: &MarginGroup,
    subaccount: &Subaccount,
    observations: &ObservationSet<'info>,
    now: i64,
) -> Result<()> {
    let collateral_guard = observations.collateral.loader.load()?;

    let settlement_guard = observations.settlement.loader.load()?;

    let mut market_guards: [Option<Ref<'_, MarketState>>; MAX_MARKETS] =
        core::array::from_fn(|_| None);

    let mut underlying_guards: [Option<Ref<'_, crate::state::AssetState>>; MAX_MARKETS] =
        core::array::from_fn(|_| None);

    for (slot_index, slot) in subaccount.markets.iter().enumerate() {
        if slot.market_id != 0 {
            let entry = market_observation_for(observations, slot.market_id)?;

            market_guards[slot_index] = Some(entry.loader.load()?);

            underlying_guards[slot_index] = Some(entry.underlying.loader.load()?);
        }
    }

    let mut portfolio: [Option<PortfolioMarketObservation<'_>>; MAX_MARKETS] = [None; MAX_MARKETS];

    for (slot_index, slot) in subaccount.markets.iter().enumerate() {
        if slot.market_id != 0 {
            let entry = market_observation_for(observations, slot.market_id)?;

            let market = market_guards[slot_index]
                .as_ref()
                .ok_or(StateError::MissingMarketObservation)?;

            let underlying_asset = underlying_guards[slot_index]
                .as_ref()
                .ok_or(StateError::MissingUnderlyingObservation)?;

            portfolio[slot_index] = Some(PortfolioMarketObservation {
                market,
                underlying_asset,
                underlying: entry.underlying.observation,
            });
        }
    }

    let health = group.initial_portfolio_health_with_assets(
        subaccount,
        &collateral_guard,
        observations.collateral.observation,
        &settlement_guard,
        observations.settlement.observation,
        portfolio.iter().flatten().copied(),
        now,
    )?;

    if !health.strict_im_met()? {
        return Err(error!(StateError::InsufficientInitialMargin));
    }

    Ok(())
}

#[inline(never)]
fn materialize_other_markets<'info>(
    subaccount: &mut Subaccount,
    observations: &ObservationSet<'info>,
    primary_market_key: Pubkey,
) -> Result<()> {
    for slot_index in 0..MAX_MARKETS {
        let market_id = subaccount.markets[slot_index].market_id;

        if market_id == 0 {
            continue;
        }

        let entry = market_observation_for(observations, market_id)?;

        if entry.loader.key() == primary_market_key {
            continue;
        }

        let mut market = entry.loader.load_mut()?;

        materialize_market(subaccount, &mut market, slot_index)?;
    }

    Ok(())
}

pub fn handle_place_order<'info>(
    context: Context<'info, OrderAccounts<'info>>,
    args: PlaceOrderArgs,
) -> Result<()> {
    let tif = TimeInForce(args.time_in_force);

    tif.validate()?;

    let now = Clock::get()?.unix_timestamp;

    let group = context.accounts.margin_group.load()?;

    let (observations, maker_account_infos) = read_observation_set::<{crate::state::MAX_MARKETS}>(
        context.remaining_accounts,
        group.collateral_state,
        group.settlement_state,
        now,
    )?;

    let (maker_loaders, maker_count) = maker_accounts(maker_account_infos)?;

    let intent = PlaceOrderIntent {
        size: args.size,
        limit_tick: args.limit_tick,
        time_in_force: tif,
        expiry_timestamp: args.expiry_timestamp,
        maximum_payment_atoms: args.maximum_payment_atoms,
    };

    execute_place_order(
        context.accounts,
        &group,
        &observations,
        &maker_loaders,
        maker_count,
        maker_account_infos,
        now,
        intent,
    )
}

#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn execute_place_order<'info>(
    accounts: &OrderAccounts<'info>,
    group: &MarginGroup,
    observations: &ObservationSet<'info>,
    maker_loaders: &[Option<AccountLoader<'info, Subaccount>>; MAX_ORDERS],
    maker_count: usize,
    maker_accounts: &'info [AccountInfo<'info>],
    now: i64,
    intent: PlaceOrderIntent,
) -> Result<()> {
    let mut maker_guards: [Option<Ref<'_, Subaccount>>; MAX_ORDERS] =
        core::array::from_fn(|_| None);

    for index in 0..maker_count {
        maker_guards[index] = Some(
            maker_loaders[index]
                .as_ref()
                .ok_or(StateError::MissingMakerSubaccount)?
                .load()?,
        );
    }

    let mut subaccount = accounts.subaccount.load_mut()?;

    {
        let mut market = accounts.market.load_mut()?;

        // The first order creates the slot inside `Subaccount::ensure_market`; only
        // an already-entered market has index or pending-fill state to materialize.

        if let Some(slot_index) = subaccount.lookup_market(market.config.market_id) {
            materialize_market(&mut subaccount, &mut market, slot_index)?;
        }

        if maker_count == 0 {
            market.place_order(&mut subaccount, accounts.subaccount.key(), now, intent, &[])?;
        } else {

            let first_key = maker_accounts[0].key();
            let first_account = maker_guards[0]
                .as_ref()
                .ok_or(StateError::MissingMakerSubaccount)?;

            let mut makers: [MakerSubaccount<'_>; MAX_ORDERS] =
                core::array::from_fn(|_| MakerSubaccount {
                    key: first_key,
                    subaccount: &first_account,
                });

            for index in 0..maker_count {
                let guard = maker_guards[index]
                    .as_ref()
                    .ok_or(StateError::MissingMakerSubaccount)?;

                makers[index] = MakerSubaccount {
                    key: maker_accounts[index].key(),
                    subaccount: guard,
                };
            }

            market.place_order(
                &mut subaccount,
                accounts.subaccount.key(),
                now,
                intent,
                &makers[..maker_count],
            )?;
        }
    }

    materialize_other_markets(&mut subaccount, observations, accounts.market.key())?;

    check_initial_health(group, &subaccount, observations, now)?;

    Ok(())
}

pub fn handle_cancel_order<'info>(
    context: Context<'info, OrderAccounts<'info>>,
    sequence: u64,
    slab_index: u64,
) -> Result<()> {
    let mut subaccount = context.accounts.subaccount.load_mut()?;

    let mut market = context.accounts.market.load_mut()?;

    let slot_index = subaccount
        .lookup_market(market.config.market_id)
        .ok_or(StateError::MarketNotEntered)?;

    materialize_market(&mut subaccount, &mut market, slot_index)?;

    market.close_order(
        &mut subaccount,
        crate::state::OrderId {
            sequence,
            slab_index,
        },
    )?;

    Ok(())
}

pub fn handle_modify_order<'info>(
    context: Context<'info, OrderAccounts<'info>>,
    args: ModifyOrderArgs,
) -> Result<()> {
    // A zero-sized modification is the explicit cancellation form.  Keep it
    // on the oracle-free cancellation path.

    if args.new_size == 0 {
        return handle_cancel_order(context, args.sequence, args.slab_index);
    }

    let now = Clock::get()?.unix_timestamp;

    let group = context.accounts.margin_group.load()?;

    let (observations, _) = read_observation_set::<{crate::state::MAX_MARKETS}>(
        context.remaining_accounts,
        group.collateral_state,
        group.settlement_state,
        now,
    )?;

    execute_modify_order(context.accounts, &group, &observations, now, args)
}

#[inline(never)]
fn execute_modify_order<'info>(
    accounts: &OrderAccounts<'info>,
    group: &MarginGroup,
    observations: &ObservationSet<'info>,
    now: i64,
    args: ModifyOrderArgs,
) -> Result<()> {
    let mut subaccount = accounts.subaccount.load_mut()?;

    {
        let mut market = accounts.market.load_mut()?;

        let slot_index = subaccount
            .lookup_market(market.config.market_id)
            .ok_or(StateError::MarketNotEntered)?;

        materialize_market(&mut subaccount, &mut market, slot_index)?;

        market.modify_order(
            &mut subaccount,
            crate::state::OrderId {
                sequence: args.sequence,
                slab_index: args.slab_index,
            },
            args.new_size,
        )?;
    }

    materialize_other_markets(&mut subaccount, observations, accounts.market.key())?;

    check_initial_health(group, &subaccount, observations, now)?;

    Ok(())
}

pub fn handle_close_all_orders<'info>(context: Context<'info, OrderAccounts<'info>>) -> Result<()> {
    let mut subaccount = context.accounts.subaccount.load_mut()?;

    let mut market = context.accounts.market.load_mut()?;

    if let Some(slot) = subaccount.lookup_market(market.config.market_id) {
        materialize_market(&mut subaccount, &mut market, slot)?;

        if !subaccount.markets[slot].order_risk.is_empty() {
            market.close_all_orders(&mut subaccount, slot)?;
        }
    }

    Ok(())
}
