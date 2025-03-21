use anchor_lang::prelude::*;
use anchor_spl::token::{Token, TokenAccount};
use crate::{
    error::SrAmmError,
    state::*,
    utils::{math::*, token::transfer_tokens},
};

#[derive(Accounts)]
pub struct AddLiquidity<'info> {
    #[account(mut)]
    pub pool: Account<'info, Pool>,
    #[account(mut)]
    pub user: Signer<'info>,
    #[account(mut)]
    pub pool_authority: Signer<'info>,
    #[account(mut,
        constraint = token_account_0.mint == pool.token_0,
        constraint = token_account_0.owner == user.key()
    )]
    pub token_account_0: Account<'info, TokenAccount>,
    #[account(mut,
        constraint = pool_token_account_0.key() == pool.token_account_0
    )]
    pub pool_token_account_0: Account<'info, TokenAccount>,
    #[account(mut,
        constraint = token_account_1.mint == pool.token_1,
        constraint = token_account_1.owner == user.key()
    )]
    pub token_account_1: Account<'info, TokenAccount>,
    #[account(mut)]
    pub pool_token_account_1: Account<'info, TokenAccount>,
    pub token_program: Program<'info, Token>,
}

pub fn handler(
    ctx: Context<AddLiquidity>,
    amount_0: u64,
    amount_1: u64,
) -> Result<()> {
    let pool = &mut ctx.accounts.pool;

    // If this is the first liquidity addition, calculate and set initial sqrt_price
    if pool.liquidity == 0 {
        // Calculate initial sqrt price as sqrt(amount1/amount0) * 2^64
        // This represents the geometric mean price scaled by 2^64
        let initial_sqrt_price = calculate_initial_sqrt_price(amount_0, amount_1)?;
        pool.sqrt_price = initial_sqrt_price;
        pool.last_slot_price = initial_sqrt_price;
    }

    // Calculate liquidity amount based on geometric mean
    let liquidity_amount = calculate_liquidity_amount(
        amount_0,
        amount_1,
        pool.sqrt_price,
    )?;

    // Update pool state
    pool.reserve_0 = pool.reserve_0.checked_add(amount_0)
        .ok_or(SrAmmError::MathError)?;
    pool.reserve_1 = pool.reserve_1.checked_add(amount_1)
        .ok_or(SrAmmError::MathError)?;
    pool.liquidity = pool.liquidity.checked_add(liquidity_amount)
        .ok_or(SrAmmError::MathError)?;

    // Transfer tokens to pool
    transfer_tokens(
        &ctx.accounts.token_program,
        &ctx.accounts.token_account_0,
        &ctx.accounts.pool_token_account_0,
        &ctx.accounts.user,
        amount_0,
    )?;

    transfer_tokens(
        &ctx.accounts.token_program,
        &ctx.accounts.token_account_1,
        &ctx.accounts.pool_token_account_1,
        &ctx.accounts.user,
        amount_1,
    )?;

    Ok(())
}

// Helper function to calculate initial sqrt price
fn calculate_initial_sqrt_price(amount_0: u64, amount_1: u64) -> Result<u128> {
    if amount_0 == 0 || amount_1 == 0 {
        return Err(SrAmmError::InvalidInitialLiquidity.into());
    }

    // Convert amounts to u128 for precision
    let amount_0 = amount_0 as u128;
    let amount_1 = amount_1 as u128;

    // Calculate sqrt(amount1/amount0) * 2^64
    // First multiply amount1 by 2^64 to maintain precision during division
    let numerator = amount_1.checked_mul(1u128 << 64)
        .ok_or(SrAmmError::MathError)?;
    
    let ratio = numerator.checked_div(amount_0)
        .ok_or(SrAmmError::MathError)?;

    // Calculate square root
    let sqrt_price = sqrt(ratio);
    Ok(sqrt_price) // Already scaled to 2^64
} 