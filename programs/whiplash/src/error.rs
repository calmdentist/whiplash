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
    
    #[msg("Mathematical operation underflow")]
    MathUnderflow,
    
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
    
    #[msg("Failed to create metadata account")]
    MetadataCreationFailed,
    
    #[msg("Failed to change token authority")]
    AuthorityChangeFailed,
    
    #[msg("Position cannot be liquidated")]
    PositionNotLiquidatable,
    
    #[msg("Invalid position")]
    InvalidPosition,

    #[msg("Insufficient output amount")]
    InsufficientOutput,
    
    #[msg("Insufficient funds for transaction")]
    InsufficientFunds,

    #[msg("Invalid leverage")]
    InvalidLeverage,

    #[msg("Position cannot be closed (would require liquidation)")]
    PositionNotClosable,

    #[msg("Functionality not yet implemented")]
    NotImplemented,
} 