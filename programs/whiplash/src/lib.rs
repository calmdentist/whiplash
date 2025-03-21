use anchor_lang::prelude::*;

pub mod error;
pub mod instructions;
pub mod state;
pub mod utils;

declare_id!("GHjAHPHGZocJKtxUhe3Eom5B73AF4XGXYukV4QMMDNhZ");

use instructions::*;

#[program]
pub mod whiplash {
    use super::*;

    pub fn initialize_pool(
        ctx: Context<InitializePool>,
        fee_tier: u16,
    ) -> Result<()> {
        instructions::initialize_pool::handler(ctx, fee_tier)
    }

    pub fn add_liquidity(
        ctx: Context<AddLiquidity>,
        amount_0: u64,
        amount_1: u64,
    ) -> Result<()> {
        instructions::add_liquidity::handler(ctx, amount_0, amount_1)
    }

    pub fn remove_liquidity(
        ctx: Context<RemoveLiquidity>,
        liquidity: u128
    ) -> Result<()> {
        instructions::remove_liquidity::handler(ctx, liquidity)
    }

    pub fn swap(
        ctx: Context<Swap>,
        amount_in: u64,
        minimum_amount_out: u64,
    ) -> Result<()> {
        instructions::swap::handler(ctx, amount_in, minimum_amount_out)
    }

    pub fn swap_leveraged(
        ctx: Context<LeveragedSwap>,
        amount_in: u64,
        minimum_amount_out: u64,
        leverage: u16,
    ) -> Result<()> {
        instructions::leveraged_swap::handler(ctx, amount_in, minimum_amount_out, leverage)
    }
} 