use crate::state::*;
use anchor_lang::prelude::*;

#[derive(Accounts)]
pub struct UpdateAuthorities<'info> {
    pub authority: Signer<'info>,
    #[account(mut, constraint = global_governance.load()?.authority_for(GovernanceCategory::AuthorityAdministration, authority.key()) @ StateError::InvalidAuthority)]
    pub global_governance: AccountLoader<'info, GlobalGovernanceConfig>,
}

pub fn handle_update_authorities(
    context: Context<UpdateAuthorities>,
    args: UpdateAuthoritiesArgs,
) -> Result<()> {
    args.apply(&mut *context.accounts.global_governance.load_mut()?)?;

    Ok(())
}
