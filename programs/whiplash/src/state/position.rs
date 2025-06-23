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

    // The collateral amount supplied by the trader (denominated in the input asset)
    pub collateral: u64,

    // Spot leg size that was swapped during opening.
    //  • long  → quantity of token-Y purchased
    //  • short → quantity of SOL (lamports) purchased
    pub spot_size: u64,

    // Borrowed amount that was removed from the real reserves and mirrored into virtual reserves.
    //  • long  → quantity of token-Y borrowed (added to virtual_token_y_amount)
    //  • short → quantity of SOL (lamports) borrowed (added to virtual_sol_amount)
    pub debt_size: u64,

    // The position nonce (allows for multiple positions in same pool)
    pub nonce: u64,

    // Bump seed for PDA derivation
    pub bump: u8,
}

impl Position {
    pub const LEN: usize = 8 + Position::INIT_SPACE;
}