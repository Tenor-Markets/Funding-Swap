use crate::state::{
    governance::GOVERNANCE_CATEGORIES, GlobalGovernanceConfig, GlobalGovernancePolicy,
    GovernanceCategory, StateError,
};
use anchor_lang::prelude::*;

fn is_program_data<T>(program: &Program<'_, T>, program_data: &Account<'_, ProgramData>) -> bool {
    program
        .programdata_address()
        .is_ok_and(|address| address == Some(program_data.key()))
}

fn is_upgrade_authority(program_data: &Account<'_, ProgramData>, authority: Pubkey) -> bool {
    program_data.upgrade_authority_address == Some(authority)
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct GovernanceInitializationArgs {
    pub authorities: [Pubkey; GOVERNANCE_CATEGORIES],
    pub subaccount_creation_fee_lamports: u64,
    pub subaccount_creation_fee_destination: Pubkey,
}

#[derive(Accounts)]
#[instruction(args: GovernanceInitializationArgs)]
pub struct InitializeGovernance<'info> {
    #[account(
        mut,
        constraint = args.authorities[GovernanceCategory::AuthorityAdministration.index()] == authority.key() @ StateError::InvalidAuthority
    )]
    pub authority: Signer<'info>,
    #[account(
        address = crate::ID,
        constraint = is_program_data(&program, &program_data) @ StateError::InvalidKey
    )]
    pub program: Program<'info>,
    #[account(
        constraint = is_upgrade_authority(&program_data, authority.key()) @ StateError::InvalidAuthority
    )]
    pub program_data: Account<'info, ProgramData>,
    #[account(init, payer = authority, space = GlobalGovernanceConfig::SPACE, seeds = [b"governance"], bump)]
    pub governance: AccountLoader<'info, GlobalGovernanceConfig>,
    pub system_program: Program<'info, System>,
}

pub fn handle_initialize_governance(
    context: Context<InitializeGovernance>,
    args: GovernanceInitializationArgs,
) -> Result<()> {
    context
        .accounts
        .governance
        .load_init()?
        .initialize_with_policy(
            context.bumps.governance,
            GlobalGovernancePolicy {
                authorities: args.authorities,
                subaccount_creation_fee_lamports: args.subaccount_creation_fee_lamports,
                subaccount_creation_fee_destination: args.subaccount_creation_fee_destination,
            },
        )?;

    Ok(())
}
