use anchor_lang::prelude::*;

#[account]
pub struct Position {
    pub owner: Pubkey,
    pub position_id: u64,
    pub liquidation_tick: i32,
    pub collateral: u64,
    pub is_long: bool,
    pub creation_timestamp: u64,
}

impl Position {
    // 8 bytes for the account discriminator plus:
    // Pubkey: 32, u64: 8, i32: 4, u64: 8, bool: 1, u64: 8
    pub const LEN: usize = 8 + 32 + 8 + 4 + 8 + 1 + 8;
}