use anchor_lang::prelude::*;

#[account]
pub struct Pool {
    pub token_0: Pubkey,
    pub token_1: Pubkey,
    pub token_account_0: Pubkey,
    pub token_account_1: Pubkey,
    pub reserve_0: u64,
    pub reserve_1: u64,
    pub fee_tier: u16,
    pub slot_window_start: u64,
    pub last_slot_price: u128,
    pub sqrt_price: u128,
    pub liquidity: u128,
    pub fee_growth_global_0: u128,
    pub fee_growth_global_1: u128,
    pub locked_bid_liquidity: u128,
    pub locked_ask_liquidity: u128,
    pub tick_bitmap: Pubkey,  // Address of the TickBitmap account
    pub borrowed_from_bid: u128, // aggregated borrowed amount from bid (for leveraged longs)
    pub borrowed_from_ask: u128, // aggregated borrowed amount from ask (for leveraged shorts)
    pub bump: u8,
}

impl Pool {
    pub const LEN: usize = 8 +   // discriminator
        32 + // token_0
        32 + // token_1
        32 + // token_account_0
        32 + // token_account_1
        8 +  // reserve_0
        8 +  // reserve_1
        2 +  // fee_tier
        8 +  // slot_window_start
        16 + // last_slot_price
        16 + // sqrt_price
        16 + // liquidity
        16 + // fee_growth_global_0
        16 + // fee_growth_global_1
        16 + // locked_bid_liquidity
        16 + // locked_ask_liquidity
        32 + // tick_bitmap
        16 + // borrowed_from_bid
        16 + // borrowed_from_ask
        1;   // bump
}