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

    // Leveraged token Y reserves
    pub leveraged_token_y_amount: u64,

    // Leveraged SOL reserves
    pub leveraged_sol_amount: u64,

    // Creation timestamp
    pub creation_timestamp: u64,

    // ----- Funding Rate fields -----
    // The original constant product of the pool, set at launch
    pub original_k: u128,

    // The sum of delta_k from all open leveraged positions
    pub total_delta_k: u128,

    // The last time the funding rate was updated
    pub last_funding_timestamp: i64,

    // A continuously increasing value representing the total funding rate accrued per unit of delta_k
    pub cumulative_funding_rate_index: u128,

    // The total accrued fees owed to the pool from all leveraged positions, denominated in the base asset
    pub unrealized_funding_fees: u128,
    
    // Bump seed for PDA derivation
    pub bump: u8,
}

impl Pool {
    pub const LEN: usize = 8 + Pool::INIT_SPACE;
    
    // Update the funding rate accumulators based on time elapsed
    pub fn update_funding_accumulators(&mut self, current_timestamp: i64) -> Result<()> {
        if self.total_delta_k == 0 || self.original_k == 0 {
            // No positions open, just update timestamp
            self.last_funding_timestamp = current_timestamp;
            return Ok(());
        }
        
        let time_elapsed = current_timestamp
            .checked_sub(self.last_funding_timestamp)
            .ok_or(error!(crate::WhiplashError::MathOverflow))? as u128;
        
        if time_elapsed == 0 {
            return Ok(());
        }
        
        // Calculate funding rate: C * (r / (1 - r))^2
        // Since r = total_delta_k / original_k, we can simplify:
        // r / (1 - r) = total_delta_k / (original_k - total_delta_k)
        
        // Ensure total_delta_k < original_k (otherwise the system is over-leveraged)
        require!(
            self.total_delta_k < self.original_k,
            crate::WhiplashError::ExcessiveLeverage
        );
        
        let k_minus_delta_k = self.original_k
            .checked_sub(self.total_delta_k)
            .ok_or(error!(crate::WhiplashError::MathUnderflow))?;
        
        // Use fixed-point precision for accurate calculation
        // Using 32 bits to avoid overflow when multiplying with large delta_k values
        const PRECISION_BITS: u32 = 32;
        const PRECISION: u128 = 1u128 << PRECISION_BITS;
        
        // ratio = (total_delta_k << PRECISION_BITS) / (original_k - total_delta_k)
        let ratio = self.total_delta_k
            .checked_mul(PRECISION)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?
            .checked_div(k_minus_delta_k)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?;
        
        // ratio_squared = (ratio * ratio) >> PRECISION_BITS
        let ratio_squared = ratio
            .checked_mul(ratio)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?
            .checked_div(PRECISION)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?;
        
        // Calculate funding rate: C * ratio_squared
        // We'll use C = 0.0001 per second (represented in fixed-point)
        let c_numerator: u128 = PRECISION / 10000; // 0.0001 in fixed-point
        
        // funding_rate = C * ratio_squared / PRECISION
        let funding_rate = c_numerator
            .checked_mul(ratio_squared)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?
            .checked_div(PRECISION)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?;
        
        // Get current y reserve for dimensional correctness
        let total_y: u128 = self.token_y_amount
            .checked_add(self.virtual_token_y_amount)
            .ok_or(error!(crate::WhiplashError::MathOverflow))? as u128;
        
        require!(total_y > 0, crate::WhiplashError::InsufficientLiquidity);
        
        // Update cumulative funding rate index
        // I(t) has dimensions 1/[y], so delta_index = (funding_rate * time_elapsed) / y_current
        let delta_index = funding_rate
            .checked_mul(time_elapsed)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?
            .checked_div(total_y)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?;
        
        self.cumulative_funding_rate_index = self.cumulative_funding_rate_index
            .checked_add(delta_index)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?;
        
        // Update unrealized funding fees (in base asset [x])
        // delta_fees = (funding_rate * total_delta_k * time_elapsed) / (y_current * PRECISION)
        let delta_fees = funding_rate
            .checked_mul(self.total_delta_k)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?
            .checked_mul(time_elapsed)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?
            .checked_div(total_y)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?
            .checked_div(PRECISION)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?;
        
        self.unrealized_funding_fees = self.unrealized_funding_fees
            .checked_add(delta_fees)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?;
        
        self.last_funding_timestamp = current_timestamp;
        
        Ok(())
    }
    
    // Calculate funding fees for a position
    pub fn calculate_position_funding_fee(
        &self,
        entry_funding_rate_index: u128,
        position_delta_k: u128,
    ) -> Result<u128> {
        // funding_due = (current_index - entry_index) * delta_k
        let index_diff = self.cumulative_funding_rate_index
            .checked_sub(entry_funding_rate_index)
            .ok_or(error!(crate::WhiplashError::MathUnderflow))?;
        
        let funding_due = index_diff
            .checked_mul(position_delta_k)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?;
        
        Ok(funding_due)
    }
    
    // Calculates the amount of token Y to receive when swapping SOL
    pub fn calculate_swap_x_to_y(&self, amount_in: u64) -> Result<u64> {
        if amount_in == 0 {
            return Err(error!(crate::WhiplashError::ZeroSwapAmount));
        }
        
        // Check if total reserves (real + virtual) are sufficient
        let total_x = self.lamports.checked_add(self.virtual_sol_amount)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?;
            // .checked_add(self.leveraged_sol_amount)
            // .ok_or(error!(crate::WhiplashError::MathOverflow))?;
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
    
    // Calculates the amount of SOL (token X) to receive when swapping in token Y.
    // `apply_soft_boundary` decides whether we apply the dynamic soft–boundary logic
    // (quadratic interpolation that partially offsets `leveraged_token_y_amount`).
    // Passing `false` produces a plain constant-product quote that includes the
    // *entire* `leveraged_token_y_amount` – this is useful for measuring the
    // premium that the soft boundary charges so we can retire virtual SOL.
    pub fn calculate_swap_y_to_x(&self, amount_in: u64, apply_soft_boundary: bool) -> Result<u64> {
        if amount_in == 0 {
            return Err(error!(crate::WhiplashError::ZeroSwapAmount));
        }

        // Short-circuit to the simple constant-product path when either:
        //   1. `apply_soft_boundary` is false (used for premium calculation), or
        //   2. `virtual_sol_amount` is zero – once the virtual reserve is gone we
        //      no longer need the soft boundary.
        if !apply_soft_boundary || self.virtual_sol_amount == 0 {
            return Self::calculate_swap_y_to_x_plain(
                amount_in,
                self.lamports,
                self.virtual_sol_amount,
                self.token_y_amount,
                self.virtual_token_y_amount
            );
        }

        // -----------------------------
        // Soft-boundary logic (original)
        // -----------------------------

        // Calculate threshold amount_in s.t. output equals the real SOL reserve.
        let threshold_amount_in = (self.token_y_amount as u128)
            .checked_mul(self.lamports as u128)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?
            .checked_div(self.virtual_sol_amount as u128)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?;

        if threshold_amount_in > u64::MAX as u128 {
            return Err(error!(crate::WhiplashError::MathOverflow));
        }

        // Quadratic interpolation of leveraged debt.
        let effective_leveraged_token_y_amount: u128 = if amount_in as u128 >= threshold_amount_in {
            self.leveraged_token_y_amount as u128
        } else {
            // amount_in^2 * leveraged_token_y_amount / threshold_amount_in^2
            let amount_in_u128 = amount_in as u128;
            let amount_in_squared = amount_in_u128 * amount_in_u128;
            let threshold_squared = threshold_amount_in * threshold_amount_in;
            let (num_low, num_high) = Self::mul_u128_to_u256(amount_in_squared, self.leveraged_token_y_amount as u128);
            Self::div_u256_by_u128(num_high, num_low, threshold_squared)
        };

        // Assemble reserves with the soft boundary logic
        let total_x = self
            .lamports
            .checked_add(self.virtual_sol_amount)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?;

        let total_y = self.token_y_amount
            .checked_add(self.virtual_token_y_amount)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?
            .checked_add(effective_leveraged_token_y_amount as u64)
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

    // Helper function for 128-bit × 128-bit → 256-bit multiplication
    // Returns (low_128_bits, high_128_bits)
    fn mul_u128_to_u256(a: u128, b: u128) -> (u128, u128) {
        // Split each u128 into two u64s
        let a_low = a as u64;
        let a_high = (a >> 64) as u64;
        let b_low = b as u64;
        let b_high = (b >> 64) as u64;
        
        // Perform partial multiplications
        let ll = (a_low as u128) * (b_low as u128);
        let lh = (a_low as u128) * (b_high as u128);
        let hl = (a_high as u128) * (b_low as u128);
        let hh = (a_high as u128) * (b_high as u128);
        
        // Combine results
        let mid = lh + hl;
        let carry = if mid < lh { 1u128 << 64 } else { 0 };
        
        let low = ll + (mid << 64);
        let high = hh + (mid >> 64) + carry + if low < ll { 1 } else { 0 };
        
        (low, high)
    }
    
    // Helper function for 256-bit ÷ 128-bit → 128-bit division
    fn div_u256_by_u128(high: u128, low: u128, divisor: u128) -> u128 {
        if high == 0 {
            // Simple case: just divide low by divisor
            return low / divisor;
        }
        
        if high >= divisor {
            // Result would overflow u128, but this shouldn't happen in our use case
            return u128::MAX;
        }
        
        // Long division algorithm for 256-bit ÷ 128-bit
        let mut remainder = high;
        let mut quotient = 0u128;
        
        // Process bit by bit from high to low
        for i in (0..128).rev() {
            remainder = (remainder << 1) | ((low >> i) & 1);
            quotient <<= 1;
            
            if remainder >= divisor {
                remainder -= divisor;
                quotient |= 1;
            }
        }
        
        quotient
    }

    // Helper that returns the plain constant-product quote (no soft boundary).
    #[inline(always)]
    fn calculate_swap_y_to_x_plain(
        amount_in: u64,
        lamports: u64,
        virtual_sol: u64,
        token_y_amount: u64,
        virtual_token_y_amount: u64
    ) -> Result<u64> {
        if amount_in == 0 {
            return Err(error!(crate::WhiplashError::ZeroSwapAmount));
        }

        let total_x = lamports.checked_add(virtual_sol)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?;
        let total_y = token_y_amount
            .checked_add(virtual_token_y_amount)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?;

        if total_x == 0 || total_y == 0 {
            return Err(error!(crate::WhiplashError::InsufficientLiquidity));
        }

        let y_after = (total_y as u128)
            .checked_add(amount_in as u128)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?;

        let numerator = (total_x as u128)
            .checked_mul(total_y as u128)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?;

        let mut x_reserve_after = numerator
            .checked_div(y_after)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?;

        // Round up to avoid k deficit
        if numerator % y_after != 0 {
            x_reserve_after = x_reserve_after
                .checked_add(1u128)
                .ok_or(error!(crate::WhiplashError::MathOverflow))?;
        }

        if x_reserve_after > u64::MAX as u128 {
            return Err(error!(crate::WhiplashError::MathOverflow));
        }

        let amount_out = total_x.checked_sub(x_reserve_after as u64)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?;

        Ok(amount_out)
    }
} 