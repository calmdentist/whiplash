use anchor_lang::prelude::*;

#[error_code]
pub enum WhiplashError {
    #[msg("Invalid token accounts")]
    InvalidTokenAccounts,
    
    #[msg("Insufficient liquidity")]
    InsufficientLiquidity,
    
    #[msg("Invalid mint authority")]
    InvalidMintAuthority,
    
    #[msg("Slippage tolerance exceeded")]
    SlippageToleranceExceeded,
    
    #[msg("Mathematical operation overflow")]
    MathOverflow,
    
    #[msg("Pool already initialized")]
    PoolAlreadyInitialized,
    
    #[msg("Zero liquidity provided")]
    ZeroLiquidity,
    
    #[msg("Zero swap amount")]
    ZeroSwapAmount,
    
    #[msg("Invalid pool state")]
    InvalidPoolState,
    
    #[msg("Unauthorized operation")]
    Unauthorized,
} 