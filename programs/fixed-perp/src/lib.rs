use anchor_lang::prelude::*;

pub mod instructions;
pub mod state;

pub use instructions::close_empty_markets::*;
pub use instructions::close_subaccount::*;
pub use instructions::create_subaccount::*;
pub use instructions::deposit_collateral::*;
pub use instructions::initialize_governance::*;

declare_id!("7NcM5j4x6ZW9yGeryVxKp5SVrUtpFdLcUYKYK4SX8i8K");

#[program]
pub mod fixed_perp {
    use super::*;

    pub fn create_subaccount(
        context: Context<CreateSubaccount>,
        subaccount_id: u16,
        group_id: u16,
    ) -> Result<()> {
        instructions::create_subaccount::handle_create_subaccount(context, subaccount_id, group_id)
    }

    pub fn close_empty_markets(context: Context<CloseEmptyMarkets>) -> Result<()> {
        instructions::close_empty_markets::handle_close_empty_markets(context)
    }

    pub fn close_subaccount(context: Context<CloseSubaccount>, group_id: u16) -> Result<()> {
        instructions::close_subaccount::handle_close_subaccount(context, group_id)
    }

    pub fn deposit_collateral(context: Context<DepositCollateral>, amount: u64) -> Result<()> {
        instructions::deposit_collateral::handle_deposit_collateral(context, amount)
    }

    pub fn initialize_governance(
        context: Context<InitializeGovernance>,
        proposal_expiry_seconds: u64,
    ) -> Result<()> {
        instructions::initialize_governance::handle_initialize_governance(
            context,
            proposal_expiry_seconds,
        )
    }

}
