use std::cell::Ref;

use anchor_lang::prelude::*;
use anchor_spl::token::{self, Mint, Token, TokenAccount, TransferChecked};

use crate::instructions::close_empty_markets::slot_is_empty;
use crate::instructions::materialize_subaccount::materialize_market;
use crate::instructions::oracle::{market_observation_for, read_observation_set, ObservationSet};
use crate::state::{
    AssetState, MarginGroup, MarketState, PortfolioMarketObservation, StateError, Subaccount,
    MAX_MARKETS,
};

#[derive(Accounts)]
pub struct WithdrawAsset<'info> {
    pub authority: Signer<'info>,
    #[account(
        mut,
        constraint = subaccount.load()?.is_authority(authority.key()) @ StateError::InvalidAuthority,
        constraint = subaccount.load()?.is_same_group_id(margin_group.load()?.group_id) @ StateError::InvalidGroupID
    )]
    pub subaccount: AccountLoader<'info, Subaccount>,
    #[account(
        mut,
        constraint = margin_group.load()?.is_active() @ StateError::InvalidStatus,
        constraint = margin_group.load()?.is_same_collateral_state(asset.key()) @ StateError::InvalidKey
    )]
    pub margin_group: AccountLoader<'info, MarginGroup>,
    #[account(
        mut,
        constraint = asset.load()?.is_active() @ StateError::InvalidStatus,
        constraint = asset.load()?.is_valid_custody(token_account.key()) @ StateError::InvalidReference
    )]
    pub asset: AccountLoader<'info, AssetState>,
    pub mint: Account<'info, Mint>,
    #[account(
        mut,
        token::mint = mint,
        token::authority = asset
    )]
    pub token_account: Account<'info, TokenAccount>,
    #[account(mut, token::mint = mint)]
    pub destination: Account<'info, TokenAccount>,
    pub token_program: Program<'info, Token>,
}

/// Materializes supplied markets for health-sensitive actions and settles only
/// those that have reached authenticated maturity.
fn settle_markets_from_observations<'info>(
    subaccount: &mut Subaccount,
    observations: &ObservationSet<'info>,
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
            .map_err(|_| StateError::InvalidReference)?;

        materialize_market(subaccount, &mut market, slot_index)?;

        if market.is_matured() {
            if !subaccount.markets[slot_index].order_risk.is_empty() {
                market.close_all_orders(subaccount, slot_index)?;
            }

            if slot_is_empty(subaccount, slot_index) {
                subaccount.close_market(slot_index, &mut market)?;
            } else {
                subaccount.settle_and_close_market(slot_index, &mut market)?;
            }
        }
    }

    Ok(())
}

#[inline(never)]
fn validate_withdraw_health<'info>(
    group: &MarginGroup,
    subaccount: &mut Subaccount,
    observations: &ObservationSet<'info>,
    amount: u64,
    now: i64,
) -> Result<()> {
    let mut market_guards: [Option<Ref<'_, MarketState>>; MAX_MARKETS] =
        core::array::from_fn(|_| None);
    let mut underlying_guards: [Option<Ref<'_, AssetState>>; MAX_MARKETS] =
        core::array::from_fn(|_| None);
    let collateral_guard = observations.collateral.loader.load()?;
    let settlement_guard = observations.settlement.loader.load()?;

    let mut portfolio: [Option<PortfolioMarketObservation<'_>>; MAX_MARKETS] = [None; MAX_MARKETS];
    for (slot_index, slot) in subaccount.markets.iter().enumerate() {
        if slot.market_id == 0 {
            continue;
        }
        let entry = market_observation_for(observations, slot.market_id)?;
        market_guards[slot_index] = Some(
            entry
                .loader
                .load()
                .map_err(|_| error!(StateError::InvalidReference))?,
        );
        underlying_guards[slot_index] = Some(
            entry
                .underlying
                .loader
                .load()
                .map_err(|_| error!(StateError::InvalidReference))?,
        );
    }
    for (slot_index, slot) in subaccount.markets.iter().enumerate() {
        if slot.market_id == 0 {
            continue;
        }
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
    let amount_i128 = i128::from(amount);

    subaccount.collateral_balance_atoms = subaccount
        .collateral_balance_atoms
        .checked_sub(amount_i128)
        .ok_or_else(|| error!(StateError::ArithmeticOverflow))?;

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

pub fn handle_withdraw_asset<'info>(
    context: Context<'info, WithdrawAsset<'info>>,
    amount: u64,
) -> Result<()> {
    if amount != 0 {
        let now = Clock::get()?.unix_timestamp;
        let (asset_id, bump) = {
            let asset = context.accounts.asset.load()?;
            (asset.asset_id, asset.bump)
        };

        {
            let mut subaccount = context.accounts.subaccount.load_mut()?;
            let group = context.accounts.margin_group.load()?;
            let (observations, _) = read_observation_set::<{crate::state::MAX_MARKETS}>(
                context.remaining_accounts,
                group.collateral_state,
                group.settlement_state,
                now,
            )?;
            settle_markets_from_observations(&mut subaccount, &observations)?;
            validate_withdraw_health(&group, &mut subaccount, &observations, amount, now)?;
        }

        context
            .accounts
            .asset
            .load_mut()?
            .check_and_commit_withdrawal(u128::from(amount), now)?;

        let asset_id_bytes = asset_id.to_le_bytes();
        let signer_seeds: &[&[u8]] = &[b"asset_state", asset_id_bytes.as_ref(), &[bump]];

        token::transfer_checked(
            CpiContext::new_with_signer(
                context.accounts.token_program.key(),
                TransferChecked {
                    from: context.accounts.token_account.to_account_info(),
                    mint: context.accounts.mint.to_account_info(),
                    to: context.accounts.destination.to_account_info(),
                    authority: context.accounts.asset.to_account_info(),
                },
                &[signer_seeds],
            ),
            amount,
            context.accounts.mint.decimals,
        )?;
    }

    Ok(())
}
