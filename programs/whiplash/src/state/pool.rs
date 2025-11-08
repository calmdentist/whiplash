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
    
    // Real Token reserves (amount held in the vault, for auditing)
    pub token_reserve: u64,
    
    // Real SOL reserves (in lamports, for auditing)
    pub sol_reserve: u64,

    // Effective reserves (used for all pricing and swaps)
    pub effective_sol_reserve: u64,
    pub effective_token_reserve: u64,

    // ----- Funding Rate fields -----

    // The sum of original delta_k from all open LONG positions
    pub total_delta_k_longs: u128,
    
    // The sum of original delta_k from all open SHORT positions
    pub total_delta_k_shorts: u128,

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
        
        let total_delta_k = self.total_delta_k_longs
            .checked_add(self.total_delta_k_shorts)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?;
        
        if total_delta_k == 0 {
            self.last_update_timestamp = current_timestamp;
            return Ok(());
        }
        
        // Funding rate is based on the total leverage relative to the current effective k
        // leverage_ratio = total_delta_k / effective_k
        // funding_rate = C * (leverage_ratio)^2
        
        let effective_k = (self.effective_sol_reserve as u128)
            .checked_mul(self.effective_token_reserve as u128)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?;
        
        require!(effective_k > 0, crate::WhiplashError::InsufficientLiquidity);
        
        // Use fixed-point precision for accurate calculation
        // Using 32 bits instead of 64 to avoid overflow when squaring leverage_ratio
        const PRECISION_BITS: u32 = 32;
        const PRECISION: u128 = 1u128 << PRECISION_BITS;
        
        // To avoid overflow, we scale down both total_delta_k and effective_k before calculating the ratio
        // This preserves the ratio while keeping numbers manageable
        const SCALE_FACTOR: u128 = 1_000_000_000; // 1 billion scale factor
        
        let scaled_delta_k = total_delta_k / SCALE_FACTOR;
        let scaled_effective_k = effective_k / SCALE_FACTOR;
        
        require!(scaled_effective_k > 0, crate::WhiplashError::InsufficientLiquidity);
        
        // leverage_ratio = (scaled_delta_k * PRECISION) / scaled_effective_k
        let leverage_ratio = scaled_delta_k
            .checked_mul(PRECISION)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?
            .checked_div(scaled_effective_k)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?;
        
        // leverage_ratio_squared = (leverage_ratio * leverage_ratio) / PRECISION
        let leverage_ratio_squared = leverage_ratio
            .checked_mul(leverage_ratio)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?
            .checked_div(PRECISION)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?;
        
        // Calculate funding rate: C * leverage_ratio_squared
        // We'll use C = 0.0001 per second (represented in fixed-point)
        let c_constant: u128 = PRECISION / 10000; // 0.0001 in fixed-point
        
        // funding_rate = (C * leverage_ratio_squared) / PRECISION
        let funding_rate = c_constant
            .checked_mul(leverage_ratio_squared)
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
        
        // Calculate fees paid by each side, proportional to their share of the total debt
        // fees_paid_by_longs = (funding_rate * total_delta_k_longs * delta_t) / PRECISION
        // funding_rate is in fixed-point, so we divide by PRECISION at the end
        // To avoid overflow, we use scaled values
        let scaled_delta_k_longs = self.total_delta_k_longs / SCALE_FACTOR;
        let scaled_delta_k_shorts = self.total_delta_k_shorts / SCALE_FACTOR;
        
        let fees_temp_longs = funding_rate
            .checked_mul(scaled_delta_k_longs)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?
            .checked_mul(delta_t as u128)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?
            .checked_div(PRECISION)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?;
        
        let fees_temp_shorts = funding_rate
            .checked_mul(scaled_delta_k_shorts)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?
            .checked_mul(delta_t as u128)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?
            .checked_div(PRECISION)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?;
        
        // Now unscale to get the actual fees
        let fees_paid_by_longs = fees_temp_longs
            .checked_mul(SCALE_FACTOR)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?;
        let fees_paid_by_shorts = fees_temp_shorts
            .checked_mul(SCALE_FACTOR)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?;
        
        // Convert k-denominated fees back to the appropriate reserve asset and distribute
        // effective_token_reserve += fees_paid_by_longs / effective_sol_reserve
        if fees_paid_by_longs > 0 {
            let token_fee_increase = fees_paid_by_longs
                .checked_div(self.effective_sol_reserve as u128)
                .ok_or(error!(crate::WhiplashError::MathOverflow))?;
            
            self.effective_token_reserve = self.effective_token_reserve
                .checked_add(token_fee_increase as u64)
                .ok_or(error!(crate::WhiplashError::MathOverflow))?;
        }
        
        // effective_sol_reserve += fees_paid_by_shorts / effective_token_reserve
        if fees_paid_by_shorts > 0 {
            let sol_fee_increase = fees_paid_by_shorts
                .checked_div(self.effective_token_reserve as u128)
                .ok_or(error!(crate::WhiplashError::MathOverflow))?;
            
            self.effective_sol_reserve = self.effective_sol_reserve
                .checked_add(sol_fee_increase as u64)
                .ok_or(error!(crate::WhiplashError::MathOverflow))?;
        }
        
        // Funding payments reduce the outstanding total debt
        self.total_delta_k_longs = self.total_delta_k_longs
            .checked_sub(fees_paid_by_longs)
            .ok_or(error!(crate::WhiplashError::MathUnderflow))?;
        
        self.total_delta_k_shorts = self.total_delta_k_shorts
            .checked_sub(fees_paid_by_shorts)
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
        const PRECISION_BITS: u32 = 32;
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
    
    // Calculate output amount for a swap using the effective reserves
    // input_is_sol: true if input is SOL, false if input is Token
    pub fn calculate_output(&self, input_amount: u64, input_is_sol: bool) -> Result<u64> {
        if input_amount == 0 {
            return Err(error!(crate::WhiplashError::ZeroSwapAmount));
        }
        
        // Check if effective reserves are sufficient
        if self.effective_sol_reserve == 0 || self.effective_token_reserve == 0 {
            return Err(error!(crate::WhiplashError::InsufficientLiquidity));
        }
        
        // Calculate effective k
        let effective_k = (self.effective_sol_reserve as u128)
            .checked_mul(self.effective_token_reserve as u128)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?;
        
        require!(effective_k > 0, crate::WhiplashError::InsufficientLiquidity);
        
        let output = if input_is_sol {
            // Input is SOL, output is TOKEN
            // output = effective_token_reserve - (k / (effective_sol_reserve + input_amount))
            let x = self.effective_sol_reserve as u128;
            let y = self.effective_token_reserve as u128;
            let input = input_amount as u128;
            
            // x_new = x + input_amount
            let x_new = x.checked_add(input)
                .ok_or(error!(crate::WhiplashError::MathOverflow))?;
            
            // y_new = k / x_new (round up to protect the pool)
            let mut y_new = effective_k
                .checked_div(x_new)
                .ok_or(error!(crate::WhiplashError::MathOverflow))?;
            
            // Round up if there's a remainder
            if effective_k % x_new != 0 {
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
            // output = effective_sol_reserve - (k / (effective_token_reserve + input_amount))
            let x = self.effective_sol_reserve as u128;
            let y = self.effective_token_reserve as u128;
            let input = input_amount as u128;
            
            // y_new = y + input_amount
            let y_new = y.checked_add(input)
                .ok_or(error!(crate::WhiplashError::MathOverflow))?;
            
            // x_new = k / y_new (round up to protect the pool)
            let mut x_new = effective_k
                .checked_div(y_new)
                .ok_or(error!(crate::WhiplashError::MathOverflow))?;
            
            // Round up if there's a remainder
            if effective_k % y_new != 0 {
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