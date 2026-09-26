use anchor_lang::prelude::*;
use anchor_spl::token::{self, Mint, Token, TokenAccount, TransferChecked};

use crate::state::{CollateralVault, MarginGroup, StateError, Subaccount};

#[derive(Accounts)]
pub struct DepositCollateral<'info> {
    pub depositor: Signer<'info>,
    #[account(
        mut,
        token::mint = collateral_mint,
        token::authority = depositor
    )]
    pub source: Account<'info, TokenAccount>,
    pub collateral_mint: Account<'info, Mint>,
    #[account(
        mut,
        constraint = token_vault.key() == vault_state.load()?.token_account,
        token::mint = collateral_mint
    )]
    pub token_vault: Account<'info, TokenAccount>,
    #[account(
        seeds = [b"collateral_vault_state", collateral_mint.key().as_ref()],
        bump = vault_state.load()?.bump,
        constraint = vault_state.load()?.mint == collateral_mint.key()
    )]
    pub vault_state: AccountLoader<'info, CollateralVault>,
    #[account(
        constraint = margin_group.load()?.collateral_mint == collateral_mint.key(),
        constraint = margin_group.load()?.group_id == beneficiary.load()?.group_id
    )]
    pub margin_group: AccountLoader<'info, MarginGroup>,
    #[account(
        mut,
        seeds = [
            b"subaccount",
            beneficiary.load()?.authority.as_ref(),
            &beneficiary.load()?.subaccount_id.to_le_bytes()
        ],
        bump = beneficiary.load()?.bump
    )]
    pub beneficiary: AccountLoader<'info, Subaccount>,
    pub token_program: Program<'info, Token>,
}

/// Transfers collateral into custody and credits the selected beneficiary.
pub fn handle_deposit_collateral(context: Context<DepositCollateral>, amount: u64) -> Result<()> {
    if amount == 0 {
        return Ok(());
    }
    let mut beneficiary = context.accounts.beneficiary.load_mut()?;
    beneficiary
        .collateral_balance_atoms
        .checked_add(i128::from(amount))
        .ok_or(StateError::ArithmeticOverflow)?;

    token::transfer_checked(
        CpiContext::new(
            token::ID,
            TransferChecked {
                from: context.accounts.source.to_account_info(),
                mint: context.accounts.collateral_mint.to_account_info(),
                to: context.accounts.token_vault.to_account_info(),
                authority: context.accounts.depositor.to_account_info(),
            },
        ),
        amount,
        context.accounts.collateral_mint.decimals,
    )?;
    beneficiary.deposit_collateral(amount)?;
    Ok(())
}
