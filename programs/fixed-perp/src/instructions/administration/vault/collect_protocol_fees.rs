use anchor_lang::prelude::*;
use anchor_spl::token::{self, Mint, Token, TokenAccount, TransferChecked};

use crate::state::*;

#[derive(Accounts)]
pub struct CollectProtocolFees<'info> {
    pub authority: Signer<'info>,
    #[account(
        constraint = global_governance.load()?.authority_for(GovernanceCategory::Treasury, authority.key()) @ StateError::InvalidAuthority
    )]
    pub global_governance: AccountLoader<'info, GlobalGovernanceConfig>,
    #[account(
        mut,
        constraint = asset.load()?.is_active() @ StateError::InvalidStatus,
        constraint = asset.load()?.is_valid_custody(token_vault.key()) @ StateError::InvalidTargetCustody
    )]
    pub asset: AccountLoader<'info, AssetState>,
    pub mint: Account<'info, Mint>,
    #[account(
        mut,
        token::mint = mint,
        token::authority = asset
    )]
    pub token_vault: Account<'info, TokenAccount>,
    #[account(
        mut,
        token::mint = mint
    )]
    pub destination: Account<'info, TokenAccount>,
    pub token_program: Program<'info, Token>,
}

pub fn handle_collect_protocol_fees<'info>(
    context: Context<'info, CollectProtocolFees<'info>>,
) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let asset_state = context.accounts.asset.key();

    if context.remaining_accounts.len() > 0 {

        let mut loaders = Vec::with_capacity(context.remaining_accounts.len());

        for account in context.remaining_accounts {
            loaders.push(AccountLoader::<MarketState>::try_from(account)?);
        }

        let mut markets = Vec::with_capacity(loaders.len());

        for loader in &loaders {
            let market = loader.load_mut()?;

            if !market.config.is_same_collateral_state(asset_state) {
                return Err(StateError::MarketAssetMismatch.into());
            }

            markets.push(market);
        }

        let mut market_refs: Vec<&mut MarketState> =
            markets.iter_mut().map(|market| &mut **market).collect();

        let amount =
            MarketState::take_matured_protocol_fees(&mut market_refs)?;

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
    }

    Ok(())
}
