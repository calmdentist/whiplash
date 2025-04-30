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

    pub fn launch(
        ctx: Context<Launch>, 
        virtual_sol_reserve: u64,
        token_name: String,
        token_ticker: String,
        metadata_uri: String,
    ) -> Result<()> {
        instructions::launch::handle_launch(ctx, virtual_sol_reserve, token_name, token_ticker, metadata_uri)
    }

    pub fn swap(ctx: Context<Swap>, amount_in: u64, min_amount_out: u64) -> Result<()> {
        instructions::swap::handle_swap(ctx, amount_in, min_amount_out)
    }

    pub fn leverage_swap(
        ctx: Context<LeverageSwap>,
        amount_in: u64,
        min_amount_out: u64,
        leverage: u8,
    ) -> Result<()> {
        instructions::leverage_swap::handle_leverage_swap(ctx, amount_in, min_amount_out, leverage)
    }
    
    pub fn liquidate(ctx: Context<Liquidate>) -> Result<()> {
        instructions::liquidate::handle_liquidate(ctx)
    }
}