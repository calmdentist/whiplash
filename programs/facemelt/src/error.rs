use anchor_lang::prelude::*;

#[error_code]
pub enum FacemeltError {
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

    #[msg("Delta K overload")]
    DeltaKOverload,

    #[msg("Position cannot be closed (would require liquidation)")]
    PositionNotClosable,

    #[msg("Excessive leverage - total delta_k ratio too high")]
    ExcessiveLeverage,
    
    #[msg("Liquidation blocked: spot price diverged too far from EMA (possible manipulation)")]
    LiquidationPriceManipulation,
    
    #[msg("Invalid bonding curve parameters")]
    InvalidBondingCurveParams,
    
    #[msg("Not enough tokens left on bonding curve")]
    InsufficientCurveTokens,
    
    #[msg("Not enough SOL in bonding curve to pay out")]
    InsufficientCurveSol,
    
    #[msg("Cannot sell more tokens than have been sold on curve")]
    InsufficientTokensSold,
    
    #[msg("Bonding curve has already graduated")]
    BondingCurveAlreadyGraduated,
    
    #[msg("Bonding curve is not active")]
    BondingCurveNotActive,
} 