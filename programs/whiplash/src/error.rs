use anchor_lang::prelude::*;

#[error_code]
pub enum SrAmmError {
    #[msg("Pool already initialized")]
    PoolAlreadyInitialized,
    #[msg("Invalid slot window")]
    InvalidSlotWindow,
    #[msg("Insufficient liquidity")]
    InsufficientLiquidity,
    #[msg("Price impact too high")]
    PriceImpactTooHigh,
    #[msg("Slippage tolerance exceeded")]
    SlippageExceeded,
    #[msg("Invalid fee tier")]
    InvalidFeeTier,
    #[msg("Liquidity activation period not elapsed")]
    LiquidityActivationPeriodNotElapsed,
    #[msg("Invalid token account")]
    InvalidTokenAccount,
    #[msg("Math error")]
    MathError,  
    #[msg("Price out of bounds")]
    PriceOutOfBounds,
    #[msg("Initial price required")]
    InitialPriceRequired,
    #[msg("Invalid initial liquidity")]
    InvalidInitialLiquidity,
} 