use crate::state::GlobalGovernanceConfig;
use anchor_lang::prelude::*;

#[derive(Accounts)]
pub struct InitializeGovernance<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,
    #[account(init, payer = authority, space = GlobalGovernanceConfig::SPACE, seeds = [b"governance"], bump)]
    pub governance: AccountLoader<'info, GlobalGovernanceConfig>,
    pub system_program: Program<'info, System>,
}

pub fn handle_initialize_governance(
    context: Context<InitializeGovernance>,
    proposal_expiry_seconds: u64,
) -> Result<()> {
    context
        .accounts
        .governance
        .load_init()?
        .initialize_in_place(
            context.bumps.governance,
            context.accounts.authority.key(),
            proposal_expiry_seconds,
        )?;
    Ok(())
}
