use anchor_lang::prelude::*;
use anchor_spl::token::TokenAccount;

use crate::state::{
    AssetState, GlobalGovernanceConfig, GovernanceCategory, OracleConfig, StateError,
};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug)]
pub struct CreateAssetArgs {
    pub max_age_seconds: u64,
    pub max_confidence_ratio_raw: i128,
    pub max_ema_deviation_raw: i128,
    pub max_peg_deviation_raw: i128,
    pub window_durations: [u64; 3],
    pub short_window_limit_atoms: u128,
    pub medium_window_limit_atoms: u128,
    pub long_window_limit_atoms: u128,
}

#[derive(Accounts)]
#[instruction(args: CreateAssetArgs)]
pub struct CreateAsset<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,
    #[account(
        constraint = global_governance.load()?.authority_for(GovernanceCategory::Resources, authority.key()) @ StateError::InvalidAuthority
    )]
    pub global_governance: AccountLoader<'info, GlobalGovernanceConfig>,
    #[account(
        init,
        payer = authority,
        space = AssetState::SPACE,
        seeds = [crate::state::USDC_SEED],
        bump
    )]
    pub asset: AccountLoader<'info, AssetState>,
    #[account(
        constraint = token_account.mint == crate::state::USDC_MINT @ StateError::InvalidTokenAccount,
        constraint = token_account.owner == asset.key() @ StateError::InvalidTokenAuthority,
        constraint = token_account.delegate.is_none() @ StateError::InvalidTokenAuthority,
        constraint = token_account.close_authority.is_none() @ StateError::InvalidTokenAuthority
    )]
    pub token_account: Account<'info, TokenAccount>,
    pub system_program: Program<'info, System>,
}

pub fn handle_create_asset(context: Context<CreateAsset>, args: CreateAssetArgs) -> Result<()> {
    let mut asset = context.accounts.asset.load_init()?;

    asset.initialize_in_place(
        context.bumps.asset,
        context.accounts.token_account.key(),
        OracleConfig {
            feed_id: crate::state::USDC_USD_FEED_ID,
            max_age_seconds: args.max_age_seconds,
            _pad: [0; 8],
            max_confidence_ratio: crate::state::FixedPoint::from_raw_bits(
                args.max_confidence_ratio_raw,
            ),
            max_ema_deviation: crate::state::FixedPoint::from_raw_bits(args.max_ema_deviation_raw),
        },
        crate::state::FixedPoint::from_raw_bits(args.max_peg_deviation_raw),
        args.window_durations,
        args.short_window_limit_atoms,
        args.medium_window_limit_atoms,
        args.long_window_limit_atoms,
    )?;

    Ok(())
}
