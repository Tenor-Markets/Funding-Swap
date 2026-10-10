// CHECKED
use anchor_lang::prelude::*;

use crate::instructions::liquidate::{maintenance_health, materialize_entered};
use crate::instructions::oracle::{
    market_observation_for, read_observation_set, settlement_ratio, underlying_settlement_price,
    ObservationSet,
};
use crate::state::{
    FixedPoint, GlobalGovernanceConfig, GovernanceCategory, MarginGroup, StateError, Subaccount,
    MAX_MARKETS,
};

#[derive(Accounts)]
pub struct Deleverage<'info> {
    pub authority: Signer<'info>,
    #[account(
        constraint = global_governance.load()?.authority_for(
            GovernanceCategory::Deleveraging,
            authority.key(),
        ) @ StateError::InvalidAuthority
    )]
    pub global_governance: AccountLoader<'info, GlobalGovernanceConfig>,
    #[account(
        mut,
        constraint = winner.key() != loser.key() @ StateError::Conflict,
        constraint = winner.load()?.is_same_group_id(margin_group.load()?.group_id) @ StateError::InvalidGroupID
    )]
    pub winner: AccountLoader<'info, Subaccount>,
    #[account(
        mut,
        constraint = loser.load()?.is_same_group_id(margin_group.load()?.group_id) @ StateError::InvalidGroupID
    )]
    pub loser: AccountLoader<'info, Subaccount>,
    #[account(constraint = margin_group.load()?.is_active() @ StateError::InvalidStatus)]
    pub margin_group: AccountLoader<'info, MarginGroup>,
}

#[event]
pub struct ForcedDeleverage {
    pub winner: Pubkey,
    pub loser: Pubkey,
    pub market_id: u32,
    pub size_to_winner: i64,
    pub alpha_raw: i128,
    pub loss_atoms: i128,
}

pub fn handle_deleverage<'info>(
    context: Context<'info, Deleverage<'info>>,
    market_id: u32,
    size_to_winner: i64,
    alpha_raw: i128,
) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let group = context.accounts.margin_group.load()?;
    // Both users can hold a full, disjoint set of markets.
    let (observations, _) = read_observation_set::<{ MAX_MARKETS * 2 }>(
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
        let mut winner = context.accounts.winner.load_mut()?;
        let mut loser = context.accounts.loser.load_mut()?;
        materialize_entered(&mut winner, &observations)?;
        materialize_entered(&mut loser, &observations)?;
    }

    {
        let winner = context.accounts.winner.load()?;
        let loser = context.accounts.loser.load()?;
        let winner_ratio =
            maintenance_health(&group, &winner, &observations, now)?.maintenance_health_ratio()?;
        let loser_ratio =
            maintenance_health(&group, &loser, &observations, now)?.maintenance_health_ratio()?;
        // The paper requires the winner to be strictly healthier. Boros checks
        // that outside the market, and this instruction is that caller.
        if winner_ratio.to_raw_bits() <= loser_ratio.to_raw_bits() {
            return Err(StateError::Conflict.into());
        }
    }

    let loser_value = {
        let loser = context.accounts.loser.load()?;
        portfolio_settlement_value(&loser, &observations, now)?
    };
    let alpha = FixedPoint::from_raw_bits(alpha_raw);
    let loss = {
        let mut winner = context.accounts.winner.load_mut()?;
        let mut loser = context.accounts.loser.load_mut()?;
        let mut market = entry
            .loader
            .load_mut()
            .map_err(|_| StateError::InvalidMarket)?;
        market.deleverage(
            &mut winner,
            &mut loser,
            size_to_winner,
            loser_value,
            alpha,
            now,
        )?
    };

    emit!(ForcedDeleverage {
        winner: context.accounts.winner.key(),
        loser: context.accounts.loser.key(),
        market_id,
        size_to_winner,
        alpha_raw,
        loss_atoms: loss,
    });
    Ok(())
}

/// Raw portfolio value in settlement atoms: collateral converted once, plus
/// each settlement balance and mark-to-market position. Haircuts stay in the
/// health ratio and do not change this debt figure.
fn portfolio_settlement_value<'info, const N: usize>(
    subaccount: &Subaccount,
    observations: &ObservationSet<'info, N>,
    now: i64,
) -> Result<i128> {
    let ratio = settlement_ratio(&observations.collateral, &observations.settlement)?;
    let mut total = ratio.apply_inverse(subaccount.collateral_balance_atoms)?;
    for slot in &subaccount.markets {
        if slot.market_id == 0 {
            continue;
        }
        total = total
            .checked_add(slot.settlement_balance_atoms)
            .ok_or_else(|| error!(StateError::ArithmeticOverflow))?;
        let entry = market_observation_for(observations, slot.market_id)?;
        let position_value = {
            let market = entry.loader.load().map_err(|_| StateError::InvalidMarket)?;
            let time_to_maturity = market
                .config
                .maturity_timestamp
                .checked_sub(now)
                .ok_or(StateError::InvalidClock)?;
            if time_to_maturity < 0 {
                return Err(error!(StateError::InvalidClock));
            }
            let mark = market.current_mark_rate(now)?;
            if market.config.settlement_state != observations.settlement.loader.key() {
                return Err(error!(StateError::Conflict));
            }
            market.position_value(slot.position_size, mark, time_to_maturity)?
        };
        let cross = underlying_settlement_price(&entry.underlying, &observations.settlement)?;
        let scaled = cross
            .checked_mul_int(position_value)
            .map_err(|_| StateError::ArithmeticOverflow)?
            .floor_atoms()
            .map_err(|_| StateError::ArithmeticOverflow)?;
        total = total
            .checked_add(scaled)
            .ok_or_else(|| error!(StateError::ArithmeticOverflow))?;
    }
    Ok(total)
}
