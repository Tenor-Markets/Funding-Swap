// CHECKED
use anchor_lang::prelude::*;
use anchor_spl::token::{self, Mint, Token, TokenAccount, TransferChecked};

use crate::instructions::materialize_subaccount::materialize_market;
use crate::state::{AssetState, MarginGroup, MarketState, StateError, Subaccount, MAX_MARKETS};

#[derive(Accounts)]
pub struct CloseSubaccount<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,
    #[account(
        mut,
        close = authority,
        constraint = subaccount.load()?.is_authority(authority.key()) @ StateError::InvalidAuthority,
        constraint = subaccount.load()?.is_same_group_id(margin_group.load()?.group_id) @ StateError::InvalidGroupID,

    )]
    pub subaccount: AccountLoader<'info, Subaccount>,
    #[account(
        mut,
        constraint = margin_group.load()?.is_active() @ StateError::InvalidStatus,
        constraint = margin_group.load()?.is_same_collateral_state(asset.key()) @ StateError::InvalidKey
    )]
    pub margin_group: AccountLoader<'info, MarginGroup>,
    #[account(constraint = asset.load()?.is_active() @ StateError::InvalidStatus)]
    pub asset: AccountLoader<'info, AssetState>,
    pub mint: Account<'info, Mint>,
    #[account(mut, token::mint = mint, token::authority = asset)]
    pub token_account: Account<'info, TokenAccount>,
    #[account(mut, token::mint = mint)]
    pub destination: Account<'info, TokenAccount>,
    pub token_program: Program<'info, Token>,
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

/// Materializes, closes orders, settles matured markets, and closes every
/// market slot before the subaccount itself is closed.
fn close_subaccount_markets_from_accounts<'info>(
    subaccount: &mut Subaccount,
    remaining_accounts: &'info [AccountInfo<'info>],
) -> Result<bool> {
    let mut closed_count = 0;
    for slot_index in 0..MAX_MARKETS {
        if subaccount.markets[slot_index].market_id == 0 {
            closed_count += 1;
            continue;
        }

        let market_info = market_account_for_slot(subaccount, slot_index, remaining_accounts)?;
        let loader = AccountLoader::<MarketState>::try_from(market_info)
            .map_err(|_| StateError::InvalidAccountInfo)?;
        let mut market = loader.load_mut().map_err(|_| StateError::InvalidMarket)?;

        materialize_market(subaccount, &mut market, slot_index)?;

        if !subaccount.markets[slot_index].order_risk.is_empty() {
            market.close_all_orders(subaccount, slot_index)?;
        }

        if slot_is_empty(subaccount, slot_index) {
            subaccount.close_market(slot_index, &mut market)?;
            closed_count += 1;
        } else if market.is_matured() {
            subaccount.settle_and_close_market(slot_index, &mut market)?;
            closed_count += 1;
        }
    }

    Ok(closed_count == MAX_MARKETS)
}

/// Withdraws nonnegative remaining collateral, closes the subaccount, and
/// removes it from its group's live count.
pub fn handle_close_subaccount<'info>(
    context: Context<'info, CloseSubaccount<'info>>,
) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let remaining_collateral = {
        let mut subaccount = context.accounts.subaccount.load_mut()?;
        let all_markets_empty =
            close_subaccount_markets_from_accounts(&mut subaccount, context.remaining_accounts)?;
        if !all_markets_empty {
            return Err(StateError::MarketsNotEmpty.into());
        }
        if subaccount.collateral_balance_atoms < 0 {
            return Err(StateError::NegativeCollateralBalance.into());
        }
        u64::try_from(subaccount.collateral_balance_atoms)
            .map_err(|_| StateError::ArithmeticOverflow)?
    };

    let (asset_id, bump) = {
        let asset = context.accounts.asset.load()?;
        if !asset.is_valid_custody(context.accounts.token_account.key()) {
            return Err(StateError::InvalidTargetCustody.into());
        }
        (asset.asset_id, asset.bump)
    };

    context
        .accounts
        .asset
        .load_mut()?
        .check_and_commit_withdrawal(u128::from(remaining_collateral), now)?;

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
        remaining_collateral,
        context.accounts.mint.decimals,
    )?;

    context
        .accounts
        .margin_group
        .load_mut()?
        .remove_subaccount()?;

    Ok(())
}
