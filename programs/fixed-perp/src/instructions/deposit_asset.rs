// CHECKED
use anchor_lang::prelude::*;
use anchor_spl::token::{self, Mint, Token, TokenAccount, TransferChecked};

use crate::state::{AssetState, MarginGroup, StateError, Subaccount};

#[derive(Accounts)]
pub struct DepositAsset<'info> {
    pub depositor: Signer<'info>,
    #[account(mut, token::mint = mint, token::authority = depositor)]
    pub source: Account<'info, TokenAccount>,
    pub mint: Account<'info, Mint>,
    #[account(mut, token::mint = mint, token::authority = asset)]
    pub token_vault: Account<'info, TokenAccount>,
    #[account(
        constraint = asset.load()?.is_active() @ StateError::InvalidStatus,
        constraint = asset.load()?.is_valid_custody(token_vault.key()) @ StateError::InvalidReference
    )]
    pub asset: AccountLoader<'info, AssetState>,
    #[account(
        constraint = margin_group.load()?.is_active() @ StateError::InvalidStatus,
        constraint = margin_group.load()?.is_same_collateral_state(asset.key()) @ StateError::InvalidKey
    )]
    pub margin_group: AccountLoader<'info, MarginGroup>,
    #[account(mut, constraint = beneficiary.load()?.is_same_group_id(margin_group.load()?.group_id) @ StateError::InvalidGroupID)]
    pub beneficiary: AccountLoader<'info, Subaccount>,
    pub token_program: Program<'info, Token>,
}

/// Deposits collateral into the AssetState custody account and credits the
/// selected beneficiary.
pub fn handle_deposit_asset(context: Context<DepositAsset>, amount: u64) -> Result<()> {
    if amount != 0 {
        context
            .accounts
            .beneficiary
            .load_mut()?
            .deposit_collateral(amount)?;

        token::transfer_checked(
            CpiContext::new(
                context.accounts.token_program.key(),
                TransferChecked {
                    from: context.accounts.source.to_account_info(),
                    mint: context.accounts.mint.to_account_info(),
                    to: context.accounts.token_vault.to_account_info(),
                    authority: context.accounts.depositor.to_account_info(),
                },
            ),
            amount,
            context.accounts.mint.decimals,
        )?;
    }

    Ok(())
}
