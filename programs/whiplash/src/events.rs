use anchor_lang::prelude::*;

#[event]
pub struct PoolLaunched {
    pub token_mint: Pubkey,
    pub pool: Pubkey,
    pub virtual_sol_reserve: u64,
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
pub struct PositionOpened {
    pub user: Pubkey,
    pub pool: Pubkey,
    pub position: Pubkey,
    pub is_long: bool,
    pub collateral: u64,
    pub leverage: u8,
    pub size: u64,
    pub entry_price: u128,
    pub timestamp: i64,
}

#[event]
pub struct PositionClosed {
    pub user: Pubkey,
    pub pool: Pubkey,
    pub position: Pubkey,
    pub is_long: bool,
    pub collateral: u64,
    pub leverage: u8,
    pub size: u64,
    pub exit_price: u128,
    pub pnl: i64,
    pub timestamp: i64,
}

#[event]
pub struct PositionLiquidated {
    pub liquidator: Pubkey,
    pub position_owner: Pubkey,
    pub pool: Pubkey,
    pub position: Pubkey,
    pub position_size: u64,
    pub borrowed_amount: u64,
    pub expected_output: u64,
    pub liquidator_reward: u64,
    pub timestamp: i64,
} 