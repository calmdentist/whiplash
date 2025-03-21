use anchor_lang::prelude::*;
use anchor_lang::solana_program::clock::Clock;
use anchor_spl::token::{Token, TokenAccount};
use crate::{
    error::SrAmmError,
    state::{Pool, TickBitmap},
    utils::{
        math::{SLOT_WINDOW_SIZE, calculate_swap_outcome},
        token::transfer_tokens,
    },
};

#[derive(Accounts)]
pub struct Swap<'info> {
    #[account(mut)]
    pub pool: Account<'info, Pool>,
    #[account(mut)]
    pub tick_bitmap: Account<'info, TickBitmap>,
    #[account(mut,
        constraint = token_account_in.mint == pool.token_0 || token_account_in.mint == pool.token_1,
        constraint = token_account_in.owner == user.key()
    )]
    pub token_account_in: Account<'info, TokenAccount>,
    #[account(mut,
        constraint = token_account_out.mint == pool.token_0 || token_account_out.mint == pool.token_1,
        constraint = token_account_out.owner == user.key(),
        constraint = token_account_in.mint != token_account_out.mint
    )]
    pub token_account_out: Account<'info, TokenAccount>,
    #[account(mut,
        constraint = pool_token_account_0.key() == pool.token_account_0
    )]
    pub pool_token_account_0: Account<'info, TokenAccount>,
    #[account(mut,
        constraint = pool_token_account_1.key() == pool.token_account_1
    )]
    pub pool_token_account_1: Account<'info, TokenAccount>,
    #[account(mut)]
    pub user: Signer<'info>,
    #[account(mut)]
    pub pool_authority: Signer<'info>,
    pub token_program: Program<'info, Token>,
}

pub fn handler(
    ctx: Context<Swap>,
    amount_in: u64,
    minimum_amount_out: u64,
) -> Result<()> {
    let pool = &mut ctx.accounts.pool;
    
    // Log current swap parameters and pool state.
    msg!("Starting swap with amount_in: {}", amount_in);
    msg!("Current pool state:");
    msg!("  sqrt_price: {}", pool.sqrt_price);
    msg!("  liquidity: {}", pool.liquidity);
    msg!("  locked_bid_liquidity: {}", pool.locked_bid_liquidity);
    msg!("  locked_ask_liquidity: {}", pool.locked_ask_liquidity);
    
    // Identify whether token0 or token1 is coming in.
    let is_token_0_in = ctx.accounts.token_account_in.mint == pool.token_0;
    let is_buy = !is_token_0_in;
    
    // Update slot window if needed.
    let clock = Clock::get()?;
    if clock.slot > pool.slot_window_start + SLOT_WINDOW_SIZE {
        pool.slot_window_start = clock.slot;
        pool.last_slot_price = pool.sqrt_price;
        pool.locked_bid_liquidity = 0;
        pool.locked_ask_liquidity = 0;
    }
    
    // Lock liquidity to maintain a constant price on one side:
    if is_buy {
        pool.locked_bid_liquidity = pool.liquidity;
        pool.locked_ask_liquidity = 0;
    } else {
        pool.locked_ask_liquidity = pool.liquidity;
        pool.locked_bid_liquidity = 0;
    }
    
    // Calculate swap outcome.
    let (amount_out, new_sqrt_price) = calculate_swap_outcome(
        pool.sqrt_price,
        pool.last_slot_price,
        amount_in,
        pool.liquidity,
        pool.locked_bid_liquidity,
        pool.locked_ask_liquidity,
        is_buy,
    )?;
    
    msg!("Swap outcome:");
    msg!("  amount_out: {}", amount_out);
    msg!("  new_sqrt_price: {}", new_sqrt_price);
    
    // Enforce the minimum amount to output.
    if amount_out < minimum_amount_out {
        return Err(SrAmmError::SlippageExceeded.into());
    }
    
    // Update pool reserves and price depending on input token.
    if is_token_0_in {
        pool.reserve_0 = pool.reserve_0.checked_add(amount_in)
            .ok_or(SrAmmError::MathError)?;
        pool.reserve_1 = pool.reserve_1.checked_sub(amount_out)
            .ok_or(SrAmmError::MathError)?;
    } else {
        pool.reserve_1 = pool.reserve_1.checked_add(amount_in)
            .ok_or(SrAmmError::MathError)?;
        pool.reserve_0 = pool.reserve_0.checked_sub(amount_out)
            .ok_or(SrAmmError::MathError)?;
    }
    pool.sqrt_price = new_sqrt_price;
    
    // Transfer tokens from user (input token) to pool.
    transfer_tokens(
        &ctx.accounts.token_program,
        &ctx.accounts.token_account_in,
        if is_token_0_in { &ctx.accounts.pool_token_account_0 } else { &ctx.accounts.pool_token_account_1 },
        &ctx.accounts.user,
        amount_in,
    )?;
    
    // Transfer tokens from pool to user (output token).
    transfer_tokens(
        &ctx.accounts.token_program,
        if is_token_0_in { &ctx.accounts.pool_token_account_1 } else { &ctx.accounts.pool_token_account_0 },
        &ctx.accounts.token_account_out,
        &ctx.accounts.pool_authority,
        amount_out,
    )?;
    
    Ok(())
} 