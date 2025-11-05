use anchor_lang::prelude::*;

#[account]
#[derive(Default, InitSpace)]
pub struct Position {
    // The owner of the position
    pub authority: Pubkey,
    
    // The pool the position is in
    pub pool: Pubkey,

    // Whether the position is long or short
    pub is_long: bool,

    // The collateral amount
    pub collateral: u64,

    // The leverage multiplier
    pub leverage: u32,

    // The position size (virtual token amount owned by this position)
    pub size: u64,

    // The stored delta_k value needed to restore the pool invariant
    pub delta_k: u128,

    // A snapshot of the cumulative_funding_accumulator from the Pool when this position was opened
    pub entry_funding_accumulator: u128,
    
    // The position nonce (allows for multiple positions in same pool)
    pub nonce: u64,

    // Bump seed for PDA derivation
    pub bump: u8,
}

impl Position {
    pub const LEN: usize = 8 + Position::INIT_SPACE;

    pub fn calculate_fill_amount(&self) -> Result<u64> {
        let fill_amount = self.collateral * self.leverage as u64 - self.collateral;
        Ok(fill_amount)
    }
}