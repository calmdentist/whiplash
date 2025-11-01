use anchor_lang::prelude::*;

#[account]
#[derive(Default, InitSpace)]
pub struct Position {
    // The owner of the position
    pub authority: Pubkey,
    
    // The pool the position is in
    pub pool: Pubkey,

    // The vault that holds position tokens
    pub position_vault: Pubkey,

    // Whether the position is long or short
    pub is_long: bool,

    // The collateral amount
    pub collateral: u64,

    // The leverage multiplier
    pub leverage: u32,

    // The entry price of the position
    pub entry_price: u128,

    // The position size (output token amount)
    pub size: u64,

    // The stored delta_k value needed to restore the pool invariant
    pub delta_k: u128,

    // A snapshot of the cumulative_funding_rate_index from the Pool when this position was opened
    pub entry_funding_rate_index: u128,

    // The leveraged token amount (output amount - collateral output amount)
    pub leveraged_token_amount: u64,

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