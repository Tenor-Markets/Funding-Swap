use anchor_lang::prelude::*;
use anchor_spl::token::{self, Mint, Token, TokenAccount, TransferChecked};

use crate::state::*;

#[derive(Accounts)]
pub struct ExecuteProtocolFeeCollection<'info> {
    #[account(
        mut,
        constraint = asset.load()?.is_active() @ StateError::InvalidStatus,
        constraint = asset.load()?.is_valid_custody(token_vault.key()) @ StateError::InvalidReference
    )]
    pub asset: AccountLoader<'info, AssetState>,
    pub mint: Account<'info, Mint>,
    #[account(mut, token::mint = mint, token::authority = asset)]
    pub token_vault: Account<'info, TokenAccount>,
    #[account(
        mut,
        token::mint = mint,
        constraint = pending_collection.is_same_destination(destination.key()) @ StateError::InvalidKey
    )]
    pub destination: Account<'info, TokenAccount>,
    #[account(
        mut,
        close = rent_recipient,
        constraint = pending_collection.is_same_asset(asset.key()) @ StateError::InvalidKey,
        constraint = pending_collection.is_same_rent_recipient(rent_recipient.key()) @ StateError::InvalidKey
    )]
    pub pending_collection: Account<'info, PendingProtocolFeeCollection>,
    /// CHECK: Address is fixed by the pending proposal; it only receives rent.
    #[account(mut)]
    pub rent_recipient: UncheckedAccount<'info>,
    pub token_program: Program<'info, Token>,
}

pub fn handle_execute_protocol_fee_collection<'info>(
    context: Context<'info, ExecuteProtocolFeeCollection<'info>>,
) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    if context.accounts.destination.key() == context.accounts.token_vault.key() {
        return Err(StateError::InvalidConfiguration.into());
    }
    context.accounts.pending_collection.window.accepts(now)?;

    let mut loaders = Vec::with_capacity(context.remaining_accounts.len());
    for account in context.remaining_accounts {
        if loaders
            .iter()
            .any(|earlier: &AccountLoader<MarketState>| earlier.key() == account.key())
        {
            return Err(StateError::InvalidRemainingAccounts.into());
        }
        loaders.push(
            AccountLoader::<MarketState>::try_from(account)
                .map_err(|_| StateError::InvalidMarket)?,
        );
    }
    let mut markets = Vec::with_capacity(loaders.len());
    for loader in &loaders {
        markets.push(loader.load_mut().map_err(|_| StateError::InvalidMarket)?);
    }
    let mut market_refs: Vec<&mut MarketState> =
        markets.iter_mut().map(|market| &mut **market).collect();
    let amount =
        MarketState::take_matured_protocol_fees(context.accounts.asset.key(), &mut market_refs)?;
    drop(market_refs);
    drop(markets);
    drop(loaders);

    let (asset_id, bump) = {
        let mut asset = context.accounts.asset.load_mut()?;
        asset.check_and_commit_withdrawal(u128::from(amount), now)?;
        (asset.asset_id, asset.bump)
    };
    let asset_id_bytes = asset_id.to_le_bytes();
    let signer_seeds: &[&[u8]] = &[b"asset_state", asset_id_bytes.as_ref(), &[bump]];
    token::transfer_checked(
        CpiContext::new_with_signer(
            context.accounts.token_program.key(),
            TransferChecked {
                from: context.accounts.token_vault.to_account_info(),
                mint: context.accounts.mint.to_account_info(),
                to: context.accounts.destination.to_account_info(),
                authority: context.accounts.asset.to_account_info(),
            },
            &[signer_seeds],
        ),
        amount,
        context.accounts.mint.decimals,
    )?;
    Ok(())
}
