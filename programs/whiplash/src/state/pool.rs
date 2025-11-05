use anchor_lang::prelude::*;

#[account]
#[derive(Default, InitSpace)]
pub struct Pool {
    // The authority that initialized the pool
    pub authority: Pubkey,
    
    // Token mint address
    pub token_mint: Pubkey,
    
    // Token vault (holds the Token reserves)
    pub token_vault: Pubkey,
    
    // Real Token reserves (amount held in the vault)
    pub token_reserve: u64,
    
    // Real SOL reserves (in lamports)
    pub sol_reserve: u64,

    // The current constant product of the pool
    pub k: u128,

    // ----- Funding Rate fields -----

    // The sum of delta_k from all open leveraged positions
    pub total_delta_k: u128,

    // A continuously increasing value representing the total funding rate accrued per unit of delta_k
    pub cumulative_funding_accumulator: u128,

    // The last time the funding accumulator was updated
    pub last_update_timestamp: i64,
    
    // Bump seed for PDA derivation
    pub bump: u8,
}

impl Pool {
    pub const LEN: usize = 8 + Pool::INIT_SPACE;
    
    // Update the funding rate accumulators based on time elapsed
    pub fn update_funding_accumulators(&mut self, current_timestamp: i64) -> Result<()> {
        let delta_t = current_timestamp
            .checked_sub(self.last_update_timestamp)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?;
        
        if delta_t <= 0 {
            return Ok(());
        }
        
        if self.total_delta_k == 0 {
            self.last_update_timestamp = current_timestamp;
            return Ok(());
        }
        
        // Funding rate is based on the current effective total_delta_k relative to the current effective k.
        // leverage_term = total_delta_k / k
        // funding_rate = C * (leverage_term)^2
        
        require!(self.k > 0, crate::WhiplashError::InsufficientLiquidity);
        
        // Use fixed-point precision for accurate calculation
        const PRECISION_BITS: u32 = 64;
        const PRECISION: u128 = 1u128 << PRECISION_BITS;
        
        // leverage_term = (total_delta_k * PRECISION) / k
        let leverage_term = self.total_delta_k
            .checked_mul(PRECISION)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?
            .checked_div(self.k)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?;
        
        // leverage_term_squared = (leverage_term * leverage_term) / PRECISION
        let leverage_term_squared = leverage_term
            .checked_mul(leverage_term)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?
            .checked_div(PRECISION)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?;
        
        // Calculate funding rate: C * leverage_term_squared
        // We'll use C = 0.0001 per second (represented in fixed-point)
        let c_constant: u128 = PRECISION / 10000; // 0.0001 in fixed-point
        
        // funding_rate = (C * leverage_term_squared) / PRECISION
        let funding_rate = c_constant
            .checked_mul(leverage_term_squared)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?
            .checked_div(PRECISION)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?;
        
        // Update the cumulative index for new positions to use
        // delta_funding_index = funding_rate * delta_t
        let delta_funding_index = funding_rate
            .checked_mul(delta_t as u128)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?;
        
        self.cumulative_funding_accumulator = self.cumulative_funding_accumulator
            .checked_add(delta_funding_index)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?;
        
        // Calculate the total fees accrued across all positions during this period
        // accrued_fees = funding_rate * total_delta_k * delta_t
        let accrued_fees = funding_rate
            .checked_mul(self.total_delta_k)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?
            .checked_mul(delta_t as u128)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?;
        
        // Realize the fees directly into the pool's state
        self.k = self.k
            .checked_add(accrued_fees)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?;
        
        self.total_delta_k = self.total_delta_k
            .checked_sub(accrued_fees)
            .ok_or(error!(crate::WhiplashError::MathUnderflow))?;
        
        self.last_update_timestamp = current_timestamp;
        
        Ok(())
    }
    
    // Calculate the remaining factor for a position based on funding accrued
    // Returns the factor in fixed-point with PRECISION bits
    // f(t) = 1 - (I(t) - I(t_open))
    pub fn calculate_position_remaining_factor(
        &self,
        entry_funding_accumulator: u128,
    ) -> Result<u128> {
        // Use the same fixed-point precision as in update_funding_accumulators
        const PRECISION_BITS: u32 = 64;
        const PRECISION: u128 = 1u128 << PRECISION_BITS;
        
        // Calculate the funding index difference
        let index_diff = self.cumulative_funding_accumulator
            .checked_sub(entry_funding_accumulator)
            .ok_or(error!(crate::WhiplashError::MathUnderflow))?;
        
        // Remaining factor = 1 - index_diff
        // Clamp to ensure it's between 0 and 1
        if index_diff >= PRECISION {
            // Position has been fully amortized
            return Ok(0);
        }
        
        let remaining_factor = PRECISION
            .checked_sub(index_diff)
            .ok_or(error!(crate::WhiplashError::MathUnderflow))?;
        
        Ok(remaining_factor)
    }
    
    // Calculate output amount for a swap against the live, effective k
    // input_is_sol: true if input is SOL, false if input is Token
    pub fn calculate_output(&self, input_amount: u64, input_is_sol: bool) -> Result<u64> {
        if input_amount == 0 {
            return Err(error!(crate::WhiplashError::ZeroSwapAmount));
        }
        
        // Check if reserves are sufficient
        if self.sol_reserve == 0 || self.token_reserve == 0 {
            return Err(error!(crate::WhiplashError::InsufficientLiquidity));
        }
        
        require!(self.k > 0, crate::WhiplashError::InsufficientLiquidity);
        
        // Swaps are always against the live, effective k
        let k_to_use = self.k;
        
        let output = if input_is_sol {
            // Input is SOL, output is TOKEN
            let x = self.sol_reserve as u128;
            let y = self.token_reserve as u128;
            let input = input_amount as u128;
            
            // x_new = x + input_amount
            let x_new = x.checked_add(input)
                .ok_or(error!(crate::WhiplashError::MathOverflow))?;
            
            // y_new = k / x_new (round up to protect the pool)
            let mut y_new = k_to_use
                .checked_div(x_new)
                .ok_or(error!(crate::WhiplashError::MathOverflow))?;
            
            // Round up if there's a remainder
            if k_to_use % x_new != 0 {
                y_new = y_new.checked_add(1)
                    .ok_or(error!(crate::WhiplashError::MathOverflow))?;
            }
            
            // output = y - y_new
            let output_amount = y.checked_sub(y_new)
                .ok_or(error!(crate::WhiplashError::InsufficientLiquidity))?;
            
            // Ensure output fits in u64
            if output_amount > u64::MAX as u128 {
                return Err(error!(crate::WhiplashError::MathOverflow));
            }
            
            output_amount as u64
        } else {
            // Input is TOKEN, output is SOL
            let x = self.sol_reserve as u128;
            let y = self.token_reserve as u128;
            let input = input_amount as u128;
            
            // y_new = y + input_amount
            let y_new = y.checked_add(input)
                .ok_or(error!(crate::WhiplashError::MathOverflow))?;
            
            // x_new = k / y_new (round up to protect the pool)
            let mut x_new = k_to_use
                .checked_div(y_new)
                .ok_or(error!(crate::WhiplashError::MathOverflow))?;
            
            // Round up if there's a remainder
            if k_to_use % y_new != 0 {
                x_new = x_new.checked_add(1)
                    .ok_or(error!(crate::WhiplashError::MathOverflow))?;
            }
            
            // output = x - x_new
            let output_amount = x.checked_sub(x_new)
                .ok_or(error!(crate::WhiplashError::InsufficientLiquidity))?;
            
            // Ensure output fits in u64
            if output_amount > u64::MAX as u128 {
                return Err(error!(crate::WhiplashError::MathOverflow));
            }
            
            output_amount as u64
        };
        
        Ok(output)
    }
} 