use anchor_lang::prelude::*;
use anchor_lang::solana_program::clock::Clock;
use crate::{
    error::SrAmmError,
    state::{Pool, Position, TickBitmap},
    utils::math::{calculate_leveraged_swap_outcome, SLOT_WINDOW_SIZE, sqrt_price_to_price},
    utils::token::transfer_tokens,
};
use anchor_spl::token::{TokenAccount, Token};

#[derive(Accounts)]
#[instruction(amount_in: u64, minimum_amount_out: u64, leverage: u16)]
pub struct LeveragedSwap<'info> {
    #[account(mut)]
    pub pool: Account<'info, Pool>,
    #[account(mut)]
    pub tick_bitmap: Account<'info, TickBitmap>,
    #[account(
        init,
        payer = user,
        space = Position::LEN,
        seeds = [b"position", pool.key().as_ref(), user.key().as_ref(), &amount_in.to_le_bytes()],
        bump
    )]
    pub position: Account<'info, Position>,
    #[account(mut)]
    pub token_account_in: Account<'info, TokenAccount>,
    #[account(mut)]
    pub pool_token_account_0: Account<'info, TokenAccount>,
    #[account(mut)]
    pub pool_token_account_1: Account<'info, TokenAccount>,
    #[account(mut)]
    pub token_account_out: Account<'info, TokenAccount>,
    #[account(mut)]
    pub user: Signer<'info>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

pub fn handler(
    ctx: Context<LeveragedSwap>,
    amount_in: u64,
    minimum_amount_out: u64,
    leverage: u16,
) -> Result<()> {
    let pool = &mut ctx.accounts.pool;
    
    // Ensure leverage > 1
    if leverage <= 1 {
        return Err(SrAmmError::MathError.into());
    }
    
    // Update slot window if needed
    let clock = Clock::get()?;
    if clock.slot > pool.slot_window_start + SLOT_WINDOW_SIZE {
        pool.slot_window_start = clock.slot;
        pool.last_slot_price = pool.sqrt_price;
        pool.locked_bid_liquidity = 0;
        pool.locked_ask_liquidity = 0;
    }
    
    let is_token0_in = ctx.accounts.token_account_in.mint == pool.token_0;
    // By convention: if token0 is NOT coming in, the trade is a "buy" (leveraged long)
    let is_buy = !is_token0_in;
    
    let current_sqrt_price = pool.sqrt_price;
    let amount_in_u128 = amount_in as u128;
    let borrow_amount = (leverage as u128 - 1) * amount_in_u128;
    
    let (amount_out, new_sqrt_price) = if is_buy {
        // Leveraged Long:
        pool.borrowed_from_bid = pool.borrowed_from_bid
            .checked_add(borrow_amount)
            .ok_or(SrAmmError::MathError)?;
        let effective_bid = (pool.reserve_1 as u128)
            .checked_sub(pool.borrowed_from_bid)
            .ok_or(SrAmmError::MathError)?;
        calculate_leveraged_swap_outcome(current_sqrt_price, amount_in, effective_bid, true)?
    } else {
        // Leveraged Short:
        pool.borrowed_from_ask = pool.borrowed_from_ask
            .checked_add(borrow_amount)
            .ok_or(SrAmmError::MathError)?;
        let effective_ask = (pool.reserve_0 as u128)
            .checked_sub(pool.borrowed_from_ask)
            .ok_or(SrAmmError::MathError)?;
        calculate_leveraged_swap_outcome(current_sqrt_price, amount_in, effective_ask, false)?
    };
    
    // Check that the output meets the minimum requirement.
    if amount_out < minimum_amount_out {
        return Err(SrAmmError::SlippageExceeded.into());
    }
    
    // Convert executed sqrt price to a regular price and determine ticks.
    let execution_price = sqrt_price_to_price(new_sqrt_price)?;
    let trade_tick = TickBitmap::price_to_tick(execution_price)?;
    const MARGIN_TICK_BUFFER: i32 = 512;
    let liquidation_tick = if is_buy {
        trade_tick - MARGIN_TICK_BUFFER
    } else {
        trade_tick + MARGIN_TICK_BUFFER
    };
    
    // Instead of appending to a Positions array, save the user's position directly.
    let user_position = Position {
        owner: ctx.accounts.user.key(),
        position_id: 0, // Alternatively, derive an ID from the PDA seed if needed
        liquidation_tick,
        collateral: amount_in,
        is_long: is_buy,
        creation_timestamp: clock.unix_timestamp as u64,
    };
    ctx.accounts.position.set_inner(user_position);
    
    // Initialize the tick in the bitmap with the current timestamp and the borrow amount.
    ctx.accounts.tick_bitmap.initialize_tick(liquidation_tick, clock.unix_timestamp as u64, borrow_amount)?;
    
    // Update pool reserves and price.
    if is_token0_in {
        pool.reserve_0 = pool.reserve_0
            .checked_add(amount_in)
            .ok_or(SrAmmError::MathError)?;
        pool.reserve_1 = pool.reserve_1
            .checked_sub(amount_out)
            .ok_or(SrAmmError::MathError)?;
    } else {
        pool.reserve_1 = pool.reserve_1
            .checked_add(amount_in)
            .ok_or(SrAmmError::MathError)?;
        pool.reserve_0 = pool.reserve_0
            .checked_sub(amount_out)
            .ok_or(SrAmmError::MathError)?;
    }
    pool.sqrt_price = new_sqrt_price;
    
    // Execute token transfers.
    transfer_tokens(
        &ctx.accounts.token_program,
        &ctx.accounts.token_account_in,
        if is_token0_in { &ctx.accounts.pool_token_account_0 } else { &ctx.accounts.pool_token_account_1 },
        &ctx.accounts.user,
        amount_in,
    )?;
    
    transfer_tokens(
        &ctx.accounts.token_program,
        if is_token0_in { &ctx.accounts.pool_token_account_1 } else { &ctx.accounts.pool_token_account_0 },
        &ctx.accounts.token_account_out,
        &ctx.accounts.user,
        amount_out,
    )?;
    
    Ok(())
}