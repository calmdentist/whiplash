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

declare_id!("5cZM87xG3opyuDjBedCpxJ6mhDyztVXLEB18tcULCmmW");

#[program]
pub mod facemelt {
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

    pub fn launch_on_curve(
        ctx: Context<LaunchOnCurve>,
        token_name: String,
        token_ticker: String,
        metadata_uri: String,
        total_supply: Option<u64>,
        target_sol: Option<u64>,
        target_tokens_sold: Option<u64>,
    ) -> Result<()> {
        instructions::launch_on_curve::handle_launch_on_curve(
            ctx,
            token_name,
            token_ticker,
            metadata_uri,
            total_supply,
            target_sol,
            target_tokens_sold,
        )
    }

    pub fn swap_on_curve(
        ctx: Context<SwapOnCurve>,
        amount_in: u64,
        min_amount_out: u64,
        input_is_sol: bool,
    ) -> Result<()> {
        instructions::swap_on_curve::handle_swap_on_curve(ctx, amount_in, min_amount_out, input_is_sol)
    }
}