use anchor_lang::prelude::*;

mod instructions;
mod state;
mod error;
mod events;

pub use instructions::*;
pub use state::*;
pub use error::*;
pub use events::*;

declare_id!("GHjAHPHGZocJKtxUhe3Eom5B73AF4XGXYukV4QMMDNhZ");

#[program]
pub mod whiplash {
    use super::*;

    pub fn launch(ctx: Context<Launch>, bump: u8, initial_virtual_sol: u64) -> Result<()> {
        instructions::initialize_pool::handle_launch(ctx, bump, initial_virtual_sol)
    }

    pub fn add_liquidity(
        ctx: Context<AddLiquidity>, 
        amount_x_desired: u64, 
        amount_y_desired: u64, 
        amount_x_min: u64, 
        amount_y_min: u64
    ) -> Result<()> {
        instructions::add_liquidity::handle_add_liquidity(
            ctx, 
            amount_x_desired, 
            amount_y_desired, 
            amount_x_min, 
            amount_y_min
        )
    }

    pub fn swap(ctx: Context<Swap>, amount_in: u64, min_amount_out: u64) -> Result<()> {
        instructions::swap::handle_swap(ctx, amount_in, min_amount_out)
    }

    pub fn liquidate(ctx: Context<Liquidate>) -> Result<()> {
        instructions::liquidate::handle_liquidate(ctx)
    }
}