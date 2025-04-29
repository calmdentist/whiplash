use anchor_lang::prelude::*;

#[event]
pub struct PoolLaunched {
    pub token_y_mint: Pubkey,
    pub pool: Pubkey,
    pub initial_virtual_sol: u64,
    pub timestamp: i64,
}

#[event]
pub struct LiquidityAdded {
    pub provider: Pubkey,
    pub pool: Pubkey,
    pub amount_x: u64,
    pub amount_y: u64,
    pub timestamp: i64,
}

#[event]
pub struct Swapped {
    pub user: Pubkey,
    pub pool: Pubkey,
    pub token_in_mint: Pubkey,
    pub token_out_mint: Pubkey,
    pub amount_in: u64,
    pub amount_out: u64,
    pub timestamp: i64,
}

#[event]
pub struct LiquidityRemoved {
    pub provider: Pubkey,
    pub pool: Pubkey,
    pub amount_x: u64,
    pub amount_y: u64,
    pub timestamp: i64,
}

#[event]
pub struct PositionLiquidated {
    pub position: Pubkey,
    pub pool: Pubkey,
    pub liquidator: Pubkey,
    pub position_size: u64,
    pub timestamp: i64,
} 