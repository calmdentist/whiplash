use anchor_lang::prelude::*;

mod instructions;
mod state;
mod error;
mod events;
mod utils;

pub use instructions::*;
pub use state::*;
pub use error::*;
pub use events::*;
pub use utils::*;

declare_id!("DjSx4kWjgjUQ2QDjYcfJooCNhisSC2Rk3uzGkK9fJRbb");

#[program]
pub mod whiplash {
    use super::*;

    pub fn launch(
        ctx: Context<Launch>, 
        sol_amount: u64,
        token_name: String,
        token_ticker: String,
        metadata_uri: String,
        funding_constant_c: Option<u128>,
        liquidation_divergence_threshold: Option<u128>,
    ) -> Result<()> {
        instructions::launch::handle_launch(
            ctx, 
            sol_amount, 
            token_name, 
            token_ticker, 
            metadata_uri,
            funding_constant_c,
            liquidation_divergence_threshold
        )
    }

    pub fn swap(ctx: Context<Swap>, amount_in: u64, min_amount_out: u64) -> Result<()> {
        instructions::swap::handle_swap(ctx, amount_in, min_amount_out)
    }

    pub fn leverage_swap(
        ctx: Context<LeverageSwap>,
        amount_in: u64,
        min_amount_out: u64,
        leverage: u32,
        nonce: u64,
    ) -> Result<()> {
        instructions::leverage_swap::handle_leverage_swap(ctx, amount_in, min_amount_out, leverage, nonce)
    }
    
    pub fn liquidate(ctx: Context<Liquidate>) -> Result<()> {
        instructions::liquidate::handle_liquidate(ctx)
    }

    pub fn close_position(ctx: Context<ClosePosition>) -> Result<()> {
        instructions::close_position::handle_close_position(ctx)
    }
}