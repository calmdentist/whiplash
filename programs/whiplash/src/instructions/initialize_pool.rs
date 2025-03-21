use anchor_lang::prelude::*;
use anchor_lang::solana_program::clock::Clock;
use anchor_spl::token::{Mint, TokenAccount};
use crate::state::*;
use crate::state::bitmap::TickDataEntry;

#[derive(Accounts)]
pub struct InitializePool<'info> {
    #[account(
        init,
        payer = user,
        space = Pool::LEN,
        seeds = [b"pool", token_0.key().as_ref(), token_1.key().as_ref()],
        bump
    )]
    pub pool: Box<Account<'info, Pool>>,
    
    #[account(init,
        payer = user,
        space = 8 +          // discriminator
               32 +         // pool pubkey
               4 +          // Vec length prefix for bitmap
               256 +        // enough space for bitmap data
               4 +          // Vec length prefix for tick_data
               (32 * std::mem::size_of::<TickDataEntry>()), // initial capacity for tick data
        seeds = [b"tickbitmap", pool.key().as_ref()],
        bump
    )]
    pub tick_bitmap: Account<'info, TickBitmap>,
    
    #[account(mut)]
    pub user: Signer<'info>,
    pub token_0: Account<'info, Mint>,
    pub token_1: Account<'info, Mint>,
    #[account(mut)]
    pub pool_token_account_0: Account<'info, TokenAccount>,
    #[account(mut)]
    pub pool_token_account_1: Account<'info, TokenAccount>,
    pub system_program: Program<'info, System>,
}

pub fn handler(
    ctx: Context<InitializePool>,
    fee_tier: u16,
) -> Result<()> {
    let pool = &mut ctx.accounts.pool;
    let tick_bitmap = &mut ctx.accounts.tick_bitmap;
    
    // Initialize pool
    pool.token_0 = ctx.accounts.token_0.key();
    pool.token_1 = ctx.accounts.token_1.key();
    pool.token_account_0 = ctx.accounts.pool_token_account_0.key();
    pool.token_account_1 = ctx.accounts.pool_token_account_1.key();
    pool.fee_tier = fee_tier;
    pool.sqrt_price = 0;
    pool.liquidity = 0;
    pool.reserve_0 = 0;
    pool.reserve_1 = 0;
    pool.slot_window_start = Clock::get()?.slot;
    pool.last_slot_price = 0;
    pool.tick_bitmap = tick_bitmap.key();
    pool.borrowed_from_bid = 0;
    pool.borrowed_from_ask = 0;
    
    // Initialize additional pool parameters
    pool.fee_growth_global_0 = 0;
    pool.fee_growth_global_1 = 0;
    pool.locked_bid_liquidity = 0;
    pool.locked_ask_liquidity = 0;
    pool.bump = ctx.bumps.pool;

    // Initialize tick bitmap with capacity
    tick_bitmap.pool = pool.key();
    tick_bitmap.bitmap = Vec::with_capacity(32);
    tick_bitmap.tick_data = Vec::with_capacity(32);

    Ok(())
}