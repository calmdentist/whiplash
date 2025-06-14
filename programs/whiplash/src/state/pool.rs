use anchor_lang::prelude::*;

#[account]
#[derive(Default, InitSpace)]
pub struct Pool {
    // The authority that initialized the pool
    pub authority: Pubkey,
    
    // Token Y mint address (since token X is native SOL)
    pub token_y_mint: Pubkey,
    
    // Token Y vault (holds the token Y reserves)
    pub token_y_vault: Pubkey,
    
    // Real Token Y reserves (amount held in the vault)
    pub token_y_amount: u64,

    // Virtual Token Y reserves
    pub virtual_token_y_amount: u64,
    
    // Real SOL reserves (in lamports)
    pub lamports: u64,

    // Virtual SOL reserves
    pub virtual_sol_amount: u64,
    
    // Bump seed for PDA derivation
    pub bump: u8,
}

impl Pool {
    pub const LEN: usize = 8 + Pool::INIT_SPACE;
    // Calculates the amount of token Y to receive when swapping SOL
    pub fn calculate_swap_x_to_y(&self, amount_in: u64) -> Result<u64> {
        if amount_in == 0 {
            return Err(error!(crate::WhiplashError::ZeroSwapAmount));
        }
        
        // Check if total reserves (real + virtual) are sufficient
        let total_x = self.lamports.checked_add(self.virtual_sol_amount)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?;
        let total_y = self.token_y_amount.checked_add(self.virtual_token_y_amount)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?;
            
        if total_x == 0 || total_y == 0 {
            return Err(error!(crate::WhiplashError::InsufficientLiquidity));
        }
        
        // Calculate new x after swap
        let x_reserve_after = total_x.checked_add(amount_in)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?;
        
        // Using u128 for intermediate calculations
        let x_after: u128 = x_reserve_after as u128;
        
        let x_before_u128: u128 = total_x as u128;
        let y_before_u128: u128 = total_y as u128;

        // Calculate new y reserves: y_after = ceil((x_before * y_before) / x_after)
        let numerator = x_before_u128
            .checked_mul(y_before_u128)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?;
        let mut y_reserve_after = numerator
            .checked_div(x_after)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?;

        // Round up if remainder exists to avoid k deficit
        if numerator % x_after != 0 {
            y_reserve_after = y_reserve_after
                .checked_add(1u128)
                .ok_or(error!(crate::WhiplashError::MathOverflow))?;
        }
        
        // Ensure fits u64
        if y_reserve_after > u64::MAX as u128 {
            return Err(error!(crate::WhiplashError::MathOverflow));
        }
        
        // Amount out = y_before - y_after (safe since y_after rounded up)
        let amount_out = total_y.checked_sub(y_reserve_after as u64)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?;
            
        Ok(amount_out)
    }
    
    // Calculates the amount of SOL to receive when swapping token Y
    pub fn calculate_swap_y_to_x(&self, amount_in: u64) -> Result<u64> {
        if amount_in == 0 {
            return Err(error!(crate::WhiplashError::ZeroSwapAmount));
        }
        
        // Check if total reserves (real + virtual) are sufficient
        let total_x = self.lamports.checked_add(self.virtual_sol_amount)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?;
        let total_y = self.token_y_amount.checked_add(self.virtual_token_y_amount)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?;
            
        if total_x == 0 || total_y == 0 {
            return Err(error!(crate::WhiplashError::InsufficientLiquidity));
        }
        
        // Using the same pattern as in calculate_swap_x_to_y but for y->x swap
        let y_reserve_after = total_y.checked_add(amount_in)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?;
        
        // Using u128 for intermediate calculations to avoid overflow
        let x_before: u128 = total_x as u128;
        let y_before: u128 = total_y as u128;
        let y_after: u128 = y_reserve_after as u128;
        
        // Calculate new x reserves ensuring no overflow: x_after = ceil((x_before * y_before) / y_after)
        let numerator = x_before
            .checked_mul(y_before)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?;
        let mut x_reserve_after = numerator
            .checked_div(y_after)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?;

        // If the division had a remainder, round UP to avoid k deficit
        if numerator % y_after != 0 {
            x_reserve_after = x_reserve_after
                .checked_add(1u128)
                .ok_or(error!(crate::WhiplashError::MathOverflow))?;
        }
        
        // Ensure the result fits in u64 after potential round-up
        if x_reserve_after > u64::MAX as u128 {
            return Err(error!(crate::WhiplashError::MathOverflow));
        }
        
        // Amount out = x_reserve_before - x_reserve_after (safe because we may have rounded up)
        let amount_out = total_x.checked_sub(x_reserve_after as u64)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?;
            
        Ok(amount_out)
    }
} 