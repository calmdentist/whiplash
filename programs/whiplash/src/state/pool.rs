use anchor_lang::prelude::*;

#[account]
#[derive(Default)]
pub struct Pool {
    // The authority that initialized the pool
    pub authority: Pubkey,
    
    // Token X mint address
    pub token_x_mint: Pubkey,
    
    // Token Y mint address
    pub token_y_mint: Pubkey,
    
    // Token X vault (holds the token X reserves)
    pub token_x_vault: Pubkey,
    
    // Token Y vault (holds the token Y reserves)
    pub token_y_vault: Pubkey,
    
    // Token X reserves (amount held in the vault)
    pub token_x_amount: u64,
    
    // Token Y reserves (amount held in the vault)
    pub token_y_amount: u64,
    
    // Bump seed for PDA derivation
    pub bump: u8,
}

impl Pool {
    pub const LEN: usize = 8 + // discriminator
                           32 + // authority
                           32 + // token_x_mint
                           32 + // token_y_mint
                           32 + // token_x_vault
                           32 + // token_y_vault
                           8 +  // token_x_amount
                           8 +  // token_y_amount
                           1;   // bump
    
    // Calculates the amount of token Y to receive when swapping token X
    pub fn calculate_swap_x_to_y(&self, amount_in: u64) -> Result<u64> {
        if amount_in == 0 {
            return Err(error!(crate::WhiplashError::ZeroSwapAmount));
        }
        
        if self.token_x_amount == 0 || self.token_y_amount == 0 {
            return Err(error!(crate::WhiplashError::InsufficientLiquidity));
        }
        
        // x * y = k formula
        // Calculate the new reserves
        let x_reserve_after = self.token_x_amount.checked_add(amount_in)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?;
            
        // k = x * y
        let k = self.token_x_amount.checked_mul(self.token_y_amount)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?;
            
        // k / x_after = y_after
        let y_reserve_after = k.checked_div(x_reserve_after)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?;
            
        // Amount out = y_reserve_before - y_reserve_after
        let amount_out = self.token_y_amount.checked_sub(y_reserve_after)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?;
            
        Ok(amount_out)
    }
    
    // Calculates the amount of token X to receive when swapping token Y
    pub fn calculate_swap_y_to_x(&self, amount_in: u64) -> Result<u64> {
        if amount_in == 0 {
            return Err(error!(crate::WhiplashError::ZeroSwapAmount));
        }
        
        if self.token_x_amount == 0 || self.token_y_amount == 0 {
            return Err(error!(crate::WhiplashError::InsufficientLiquidity));
        }
        
        // x * y = k formula
        // Calculate the new reserves
        let y_reserve_after = self.token_y_amount.checked_add(amount_in)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?;
            
        // k = x * y
        let k = self.token_x_amount.checked_mul(self.token_y_amount)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?;
            
        // k / y_after = x_after
        let x_reserve_after = k.checked_div(y_reserve_after)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?;
            
        // Amount out = x_reserve_before - x_reserve_after
        let amount_out = self.token_x_amount.checked_sub(x_reserve_after)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?;
            
        Ok(amount_out)
    }
} 