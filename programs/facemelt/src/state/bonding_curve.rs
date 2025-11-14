use anchor_lang::prelude::*;

#[derive(PartialEq, Eq, Clone, Copy)]
#[repr(u8)]
pub enum BondingCurveStatus {
    Active = 0,
    Graduated = 1,
}

impl Default for BondingCurveStatus {
    fn default() -> Self {
        BondingCurveStatus::Active
    }
}

#[account]
#[derive(Default, InitSpace)]
pub struct BondingCurve {
    // The authority that launched the bonding curve
    pub authority: Pubkey,
    
    // Token mint address
    pub token_mint: Pubkey,
    
    // Associated pool address (will be uninitialized until graduation)
    pub pool: Pubkey,
    
    // Token vault (holds the unsold tokens)
    pub token_vault: Pubkey,
    
    // The slope of the linear bonding curve (m in price = m * q)
    // Stored in fixed-point with 18 decimals precision
    pub bonding_curve_slope_m: u128,
    
    // Counter for tokens sold during bonding phase (in token base units)
    pub tokens_sold_on_curve: u64,
    
    // Counter for SOL raised during bonding phase (in lamports)
    pub sol_raised_on_curve: u64,
    
    // The target SOL to be raised (e.g., 200 SOL in lamports)
    pub bonding_target_sol: u64,
    
    // The target tokens to be sold (e.g., 280M tokens)
    pub bonding_target_tokens_sold: u64,
    
    // Status of the bonding curve
    pub status: u8,
    
    // Bump seed for PDA derivation
    pub bump: u8,
}

impl BondingCurve {
    pub const LEN: usize = 8 + BondingCurve::INIT_SPACE;
    
    // Fixed-point precision for slope calculation (10^18)
    pub const SLOPE_PRECISION: u128 = 1_000_000_000_000_000_000;
    
    // Default values
    pub const DEFAULT_TOTAL_SUPPLY: u64 = 420_000_000_000_000; // 420M with 6 decimals
    pub const DEFAULT_TARGET_SOL: u64 = 200_000_000_000; // 200 SOL in lamports
    pub const DEFAULT_TARGET_TOKENS_SOLD: u64 = 280_000_000_000_000; // 280M with 6 decimals
    
    pub fn is_active(&self) -> bool {
        self.status == BondingCurveStatus::Active as u8
    }
    
    pub fn is_graduated(&self) -> bool {
        self.status == BondingCurveStatus::Graduated as u8
    }
    
    // Calculate slope: m = (2 * target_sol) / (target_tokens_sold^2)
    // Returns slope in fixed-point representation
    pub fn calculate_slope(target_sol: u64, target_tokens_sold: u64) -> Result<u128> {
        require!(target_tokens_sold > 0, crate::FacemeltError::InvalidBondingCurveParams);
        
        // m = (2 * target_sol * PRECISION) / target_tokens_sold^2
        let numerator = (2u128)
            .checked_mul(target_sol as u128)
            .ok_or(error!(crate::FacemeltError::MathOverflow))?
            .checked_mul(Self::SLOPE_PRECISION)
            .ok_or(error!(crate::FacemeltError::MathOverflow))?;
        
        let denominator = (target_tokens_sold as u128)
            .checked_mul(target_tokens_sold as u128)
            .ok_or(error!(crate::FacemeltError::MathOverflow))?;
        
        let slope = numerator
            .checked_div(denominator)
            .ok_or(error!(crate::FacemeltError::MathOverflow))?;
        
        Ok(slope)
    }
    
    // Calculate how many tokens can be bought for a given SOL amount
    // q2 = sqrt(q1^2 + (2 * sol_in) / m)
    pub fn calculate_tokens_out_for_sol(&self, sol_in: u64) -> Result<u64> {
        require!(sol_in > 0, crate::FacemeltError::ZeroSwapAmount);
        
        let q1 = self.tokens_sold_on_curve as u128;
        
        // Calculate (2 * sol_in * PRECISION) / m
        let term = (2u128)
            .checked_mul(sol_in as u128)
            .ok_or(error!(crate::FacemeltError::MathOverflow))?
            .checked_mul(Self::SLOPE_PRECISION)
            .ok_or(error!(crate::FacemeltError::MathOverflow))?
            .checked_div(self.bonding_curve_slope_m)
            .ok_or(error!(crate::FacemeltError::MathOverflow))?;
        
        // q1^2 + term
        let q1_squared = q1
            .checked_mul(q1)
            .ok_or(error!(crate::FacemeltError::MathOverflow))?;
        
        let sum = q1_squared
            .checked_add(term)
            .ok_or(error!(crate::FacemeltError::MathOverflow))?;
        
        // sqrt(sum)
        let q2 = integer_sqrt(sum)?;
        
        // tokens_out = q2 - q1
        let tokens_out = q2
            .checked_sub(q1)
            .ok_or(error!(crate::FacemeltError::MathUnderflow))?;
        
        // Note: We don't check target here because swap_on_curve will cap at target
        // This allows the function to calculate the theoretical output
        // and let the caller handle capping/refunding
        
        Ok(tokens_out as u64)
    }
    
    // Calculate how much SOL is needed for tokens back when selling
    // sol_out = (m * (q1^2 - q2^2)) / 2
    pub fn calculate_sol_out_for_tokens(&self, tokens_in: u64) -> Result<u64> {
        require!(tokens_in > 0, crate::FacemeltError::ZeroSwapAmount);
        
        let q1 = self.tokens_sold_on_curve as u128;
        let q2 = q1
            .checked_sub(tokens_in as u128)
            .ok_or(error!(crate::FacemeltError::InsufficientTokensSold))?;
        
        // Calculate q1^2 - q2^2
        let q1_squared = q1
            .checked_mul(q1)
            .ok_or(error!(crate::FacemeltError::MathOverflow))?;
        
        let q2_squared = q2
            .checked_mul(q2)
            .ok_or(error!(crate::FacemeltError::MathOverflow))?;
        
        let diff = q1_squared
            .checked_sub(q2_squared)
            .ok_or(error!(crate::FacemeltError::MathUnderflow))?;
        
        // sol_out = (m * diff) / (2 * PRECISION)
        let sol_out = self.bonding_curve_slope_m
            .checked_mul(diff)
            .ok_or(error!(crate::FacemeltError::MathOverflow))?
            .checked_div(2u128)
            .ok_or(error!(crate::FacemeltError::MathOverflow))?
            .checked_div(Self::SLOPE_PRECISION)
            .ok_or(error!(crate::FacemeltError::MathOverflow))?;
        
        // Check if we have enough SOL in curve to pay out
        require!(
            sol_out <= self.sol_raised_on_curve as u128,
            crate::FacemeltError::InsufficientCurveSol
        );
        
        Ok(sol_out as u64)
    }
}

// Integer square root using Newton's method
fn integer_sqrt(n: u128) -> Result<u128> {
    if n == 0 {
        return Ok(0);
    }
    
    // Initial guess: start with n/2
    let mut x = n / 2 + 1;
    let mut y = (x + n / x) / 2;
    
    // Newton's method: iterate until convergence
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    
    Ok(x)
}

