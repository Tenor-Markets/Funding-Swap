use crate::state::*;
use anchor_lang::prelude::*;

#[derive(Accounts)]
#[instruction(market_id: u32)]
pub struct AllocateMarket<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,
    #[account(constraint = global_governance.load()?.authority_for(GovernanceCategory::Creation, authority.key()) @ StateError::InvalidAuthority)]
    pub global_governance: AccountLoader<'info, GlobalGovernanceConfig>,
    /// CHECK: The account is validated as the canonical, empty system-owned market PDA.
    #[account(
        mut,
        constraint = market_id != 0 @ StateError::InvalidConfiguration,
        constraint = market.key() == Pubkey::find_program_address(
            &[b"market_state", &market_id.to_le_bytes()],
            &crate::ID
        ).0 @ StateError::InvalidKey,
        constraint = market.owner == &anchor_lang::system_program::ID @ StateError::InvalidKey,
        constraint = market.data_is_empty() @ StateError::InvalidKey
    )]
    pub market: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
}

pub fn handle_allocate_market(context: Context<AllocateMarket>, market_id: u32) -> Result<()> {
    // Context binds governance and the signer; the handler derives and creates
    // the oversized market PDA because Anchor cannot allocate it via `init`.
    let (_, bump) =
        Pubkey::find_program_address(&[b"market_state", &market_id.to_le_bytes()], &crate::ID);
    let required_lamports = Rent::get()?.minimum_balance(MarketState::SPACE);
    let current_lamports = context.accounts.market.to_account_info().lamports();
    if current_lamports < required_lamports {
        let top_up = required_lamports
            .checked_sub(current_lamports)
            .ok_or(StateError::ArithmeticOverflow)?;
        anchor_lang::system_program::transfer(
            CpiContext::new(
                anchor_lang::system_program::ID,
                anchor_lang::system_program::Transfer {
                    from: context.accounts.authority.to_account_info(),
                    to: context.accounts.market.to_account_info(),
                },
            ),
            top_up,
        )?;
    }
    let market_id_bytes = market_id.to_le_bytes();
    let signer_seeds: &[&[u8]] = &[b"market_state", &market_id_bytes, &[bump]];
    let space = u64::try_from(MarketState::SPACE).map_err(|_| StateError::ArithmeticOverflow)?;
    anchor_lang::system_program::allocate(
        CpiContext::new_with_signer(
            anchor_lang::system_program::ID,
            anchor_lang::system_program::Allocate {
                account_to_allocate: context.accounts.market.to_account_info(),
            },
            &[signer_seeds],
        ),
        space,
    )?;
    anchor_lang::system_program::assign(
        CpiContext::new_with_signer(
            anchor_lang::system_program::ID,
            anchor_lang::system_program::Assign {
                account_to_assign: context.accounts.market.to_account_info(),
            },
            &[signer_seeds],
        ),
        &crate::ID,
    )?;
    Ok(())
}
