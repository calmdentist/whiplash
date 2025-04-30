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
        
        // x * y = k formula with virtual reserves
        // Calculate the new reserves
        let x_reserve_after = total_x.checked_add(amount_in)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?;
            
        // k = (x + virtual_x) * (y + virtual_y)
        let k = total_x.checked_mul(total_y)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?;
            
        // k / x_after = y_after
        let y_reserve_after = k.checked_div(x_reserve_after)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?;
            
        // Amount out = y_reserve_before - y_reserve_after
        let amount_out = total_y.checked_sub(y_reserve_after)
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
        
        // x * y = k formula with virtual reserves
        // Calculate the new reserves
        let y_reserve_after = total_y.checked_add(amount_in)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?;
            
        // k = (x + virtual_x) * (y + virtual_y)
        let k = total_x.checked_mul(total_y)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?;
            
        // k / y_after = x_after
        let x_reserve_after = k.checked_div(y_reserve_after)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?;
            
        // Amount out = x_reserve_before - x_reserve_after
        let amount_out = total_x.checked_sub(x_reserve_after)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?;
            
        Ok(amount_out)
    }
} 