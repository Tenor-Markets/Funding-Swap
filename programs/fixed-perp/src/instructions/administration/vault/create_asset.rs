use anchor_lang::prelude::*;
use anchor_spl::token::TokenAccount;

use crate::state::{
    AssetState, GlobalGovernanceConfig, GovernanceCategory, PriceModel, StateError,
};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug)]
pub struct CreateAssetArgs {
    pub asset_id: u32,
    pub price_model: u8,
    pub decimals: u8,
    pub feed_id: [u8; 32],
    pub reference_price_raw: i128,
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
        seeds = [b"asset_state", args.asset_id.to_le_bytes().as_ref()],
        bump
    )]
    pub asset: AccountLoader<'info, AssetState>,
    pub token_account: Option<Account<'info, TokenAccount>>,
    pub system_program: Program<'info, System>,
}

pub fn handle_create_asset(context: Context<CreateAsset>, args: CreateAssetArgs) -> Result<()> {
    let token_account = match &context.accounts.token_account {
        Some(token) => {
            if token.owner != context.accounts.asset.key() {
                return Err(error!(StateError::InvalidTokenAuthority));
            }

            token.key()
        }
        None => Pubkey::default(),
    };

    let mut asset = context.accounts.asset.load_init()?;

    asset.initialize_in_place(
        args.asset_id,
        context.bumps.asset,
        PriceModel(args.price_model),
        args.decimals,
        token_account,
        args.feed_id,
        crate::state::FixedPoint::from_raw_bits(args.reference_price_raw),
        args.max_age_seconds,
        crate::state::FixedPoint::from_raw_bits(args.max_confidence_ratio_raw),
        crate::state::FixedPoint::from_raw_bits(args.max_ema_deviation_raw),
        crate::state::FixedPoint::from_raw_bits(args.max_peg_deviation_raw),
        args.window_durations,
        args.short_window_limit_atoms,
        args.medium_window_limit_atoms,
        args.long_window_limit_atoms,
    )?;

    Ok(())
}
