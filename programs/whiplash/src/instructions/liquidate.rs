use anchor_lang::prelude::*;
use anchor_spl::{
    token::{self, Token, TokenAccount, Transfer},
};
use crate::{state::*, events::*, WhiplashError};

#[derive(Accounts)]
pub struct Liquidate<'info> {
    #[account(mut)]
    pub liquidator: Signer<'info>,
    
    /// The owner of the position being liquidated
    /// CHECK: Account is not written to, just a key
    pub position_owner: UncheckedAccount<'info>,
    
    #[account(
        mut,
        seeds = [
            b"pool".as_ref(),
            pool.token_mint.as_ref(),
        ],
        bump = pool.bump,
    )]
    pub pool: Account<'info, Pool>,
    
    #[account(
        mut,
        constraint = token_vault.key() == pool.token_vault @ WhiplashError::InvalidTokenAccounts,
        constraint = token_vault.mint == pool.token_mint @ WhiplashError::InvalidTokenAccounts,
        constraint = token_vault.owner == pool.key() @ WhiplashError::InvalidTokenAccounts,
    )]
    pub token_vault: Account<'info, TokenAccount>,
    
    #[account(
        mut,
        seeds = [
            b"position".as_ref(),
            pool.key().as_ref(),
            position_owner.key().as_ref(),
            position.nonce.to_le_bytes().as_ref(),
        ],
        bump,
        close = liquidator,
        constraint = position.authority == position_owner.key() @ WhiplashError::InvalidPosition,
        constraint = position.pool == pool.key() @ WhiplashError::InvalidPosition,
    )]
    pub position: Account<'info, Position>,
    
    /// CHECK: This can be either an SPL token account OR a native SOL account (liquidator wallet)
    #[account(mut)]
    pub liquidator_reward_account: UncheckedAccount<'info>,
    
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

pub fn handle_liquidate(ctx: Context<Liquidate>) -> Result<()> {
    // Update funding rate accumulators before any position operations
    // This also updates the EMA price
    let current_timestamp = Clock::get()?.unix_timestamp;
    ctx.accounts.pool.update_funding_accumulators(current_timestamp)?;
    
    // Check price divergence to prevent manipulation-based liquidations
    let price_safe = ctx.accounts.pool.check_liquidation_price_safety()?;
    require!(
        price_safe,
        WhiplashError::LiquidationPriceManipulation
    );
    
    let position = &ctx.accounts.position;
    let pool = &ctx.accounts.pool;
    
    // -----------------------------------------------------------------
    // Calculate effective position values using amortization formula
    // f(t) = 1 - (I(t) - I(t_open))
    // effective_size = size * f(t)
    // effective_delta_k = delta_k * f(t)
    // -----------------------------------------------------------------

    let position_size_original = position.size;
    let delta_k_original: u128 = position.delta_k;
    
    // Use pool's method to calculate remaining factor
    const PRECISION_BITS: u32 = 32;
    const PRECISION: u128 = 1u128 << PRECISION_BITS;
    
    let remaining_factor = pool.calculate_position_remaining_factor(position.entry_funding_accumulator)?;
    
    // Calculate effective position size: effective_size = original_size * remaining_factor / PRECISION
    let effective_size_u128: u128 = (position_size_original as u128)
        .checked_mul(remaining_factor)
        .ok_or(error!(WhiplashError::MathOverflow))?
        .checked_div(PRECISION)
        .ok_or(error!(WhiplashError::MathOverflow))?;
    
    // Calculate effective delta_k: effective_delta_k = original_delta_k * remaining_factor / PRECISION
    let effective_delta_k: u128 = delta_k_original
        .checked_mul(remaining_factor)
        .ok_or(error!(WhiplashError::MathOverflow))?
        .checked_div(PRECISION)
        .ok_or(error!(WhiplashError::MathOverflow))?;

    // Current effective reserves
    let x_e: u128 = pool.effective_sol_reserve as u128;
    let y_e: u128 = pool.effective_token_reserve as u128;

    // Convert effective_size to u64 for calculate_output
    let effective_size_u64 = if effective_size_u128 > u64::MAX as u128 {
        return Err(error!(WhiplashError::MathOverflow));
    } else {
        effective_size_u128 as u64
    };

    // 1. Calculate the gross value of the position's effective size
    // This is what the position would be worth if swapped without debt repayment
    let position_value_in_collateral = pool.calculate_output(
        effective_size_u64,
        !position.is_long // Swap direction is opposite of position direction
    )? as u128;

    // 2. Calculate the net payout after repaying debt (same formula as close_position)
    let payout_u128 = if position.is_long {
        // Long: returns tokens and gets SOL
        // payout = (x_e * effective_size - effective_delta_k) / (y_e + effective_size)
        let product_val = x_e
            .checked_mul(effective_size_u128)
            .ok_or(error!(WhiplashError::MathOverflow))?;

        if product_val <= effective_delta_k {
            // Underwater: closing would require taking from pool (bad debt)
            // Don't liquidate - let funding fees amortize the position to zero
            return Err(error!(WhiplashError::PositionNotLiquidatable));
        }
        
        let numerator = product_val
            .checked_sub(effective_delta_k)
            .ok_or(error!(WhiplashError::MathOverflow))?;
        let denominator = y_e
            .checked_add(effective_size_u128)
            .ok_or(error!(WhiplashError::MathOverflow))?;
        numerator
            .checked_div(denominator)
            .ok_or(error!(WhiplashError::MathOverflow))?
    } else {
        // Short: returns SOL and gets tokens
        // payout = (y_e * effective_size - effective_delta_k) / (x_e + effective_size)
        let product_val = effective_size_u128
            .checked_mul(y_e)
            .ok_or(error!(WhiplashError::MathOverflow))?;

        if product_val <= effective_delta_k {
            // Underwater: closing would require taking from pool (bad debt)
            // Don't liquidate - let funding fees amortize the position to zero
            return Err(error!(WhiplashError::PositionNotLiquidatable));
        }
        
        let numerator = product_val
            .checked_sub(effective_delta_k)
            .ok_or(error!(WhiplashError::MathOverflow))?;
        let denominator = x_e
            .checked_add(effective_size_u128)
            .ok_or(error!(WhiplashError::MathOverflow))?;
        numerator
            .checked_div(denominator)
            .ok_or(error!(WhiplashError::MathOverflow))?
    };

    // 3. Check if the net payout is AT MOST 5% of the gross value
    // Position is liquidatable when: payout <= 5% of position_value
    let liquidation_threshold = position_value_in_collateral
        .checked_mul(5)
        .ok_or(error!(WhiplashError::MathOverflow))?
        .checked_div(100)
        .ok_or(error!(WhiplashError::MathOverflow))?;

    require!(
        payout_u128 <= liquidation_threshold,
        WhiplashError::PositionNotLiquidatable
    );

    // -----------------------------------------------------------------
    // Execute liquidation
    // -----------------------------------------------------------------

    // 4. The liquidator's reward is the entire remaining payout
    let liquidator_reward = if payout_u128 > u64::MAX as u128 {
        return Err(error!(WhiplashError::MathOverflow));
    } else {
        payout_u128 as u64
    };
    
    // Get pool signer seeds for transferring from vault
    let pool_mint = ctx.accounts.pool.token_mint;
    let pool_bump = ctx.accounts.pool.bump;

    // 5. Settle the position against the pool (same logic as close_position)
    // Note: Positions are virtual - tokens were never physically transferred out of the pool
    if position.is_long {
        // LONG POSITION LIQUIDATION
        // Position has virtual claim on tokens, liquidator gets SOL reward
        
        // Update pool state
        {
            let pool = &mut ctx.accounts.pool;
            // Return the position's effective virtual tokens to effective reserves
            pool.effective_token_reserve = pool.effective_token_reserve
                .checked_add(effective_size_u64)
                .ok_or(error!(WhiplashError::MathOverflow))?;
            
            // Deduct liquidator reward (SOL) from effective reserves
            pool.effective_sol_reserve = pool.effective_sol_reserve
                .checked_sub(liquidator_reward)
                .ok_or(error!(WhiplashError::MathUnderflow))?;
            
            // Also deduct from real SOL reserves (actual payout)
            pool.sol_reserve = pool.sol_reserve
                .checked_sub(liquidator_reward)
                .ok_or(error!(WhiplashError::MathUnderflow))?;
            
            // Remove this position's EFFECTIVE delta_k from the longs pool
            // Funding fees reduce total_delta_k proportionally across all positions
            // So we subtract the effective delta_k (original * remaining_factor)
            pool.total_delta_k_longs = pool.total_delta_k_longs
                .checked_sub(effective_delta_k)
                .ok_or(error!(WhiplashError::MathUnderflow))?;
        }
        
        // Transfer liquidator reward (SOL from pool to liquidator)
        if liquidator_reward > 0 {
            let pool_lamports = ctx.accounts.pool.to_account_info().lamports();
            let liquidator_lamports = ctx.accounts.liquidator_reward_account.to_account_info().lamports();
            
            **ctx.accounts.pool.to_account_info().try_borrow_mut_lamports()? = pool_lamports
                .checked_sub(liquidator_reward)
                .ok_or(error!(WhiplashError::MathUnderflow))?;
                
            **ctx.accounts.liquidator_reward_account.to_account_info().try_borrow_mut_lamports()? = liquidator_lamports
                .checked_add(liquidator_reward)
                .ok_or(error!(WhiplashError::MathOverflow))?;
        }
    } else {
        // SHORT POSITION LIQUIDATION
        // Position has virtual claim on SOL, liquidator gets tokens as reward
        
        // Update pool state
        {
            let pool = &mut ctx.accounts.pool;
            // Return the position's effective virtual SOL to effective reserves
            pool.effective_sol_reserve = pool.effective_sol_reserve
                .checked_add(effective_size_u64)
                .ok_or(error!(WhiplashError::MathOverflow))?;
                
            // Deduct liquidator reward (tokens) from effective reserves
            pool.effective_token_reserve = pool.effective_token_reserve
                .checked_sub(liquidator_reward)
                .ok_or(error!(WhiplashError::MathUnderflow))?;
            
            // Also deduct from real token reserves (actual payout)
            pool.token_reserve = pool.token_reserve
                .checked_sub(liquidator_reward)
                .ok_or(error!(WhiplashError::MathUnderflow))?;
            
            // Remove this position's EFFECTIVE delta_k from the shorts pool
            // Funding fees reduce total_delta_k proportionally across all positions
            // So we subtract the effective delta_k (original * remaining_factor)
            pool.total_delta_k_shorts = pool.total_delta_k_shorts
                .checked_sub(effective_delta_k)
                .ok_or(error!(WhiplashError::MathUnderflow))?;
        }
        
        // Transfer liquidator reward (tokens from vault to liquidator)
        if liquidator_reward > 0 {
            let pool_seeds = &[
                b"pool".as_ref(),
                pool_mint.as_ref(),
                &[pool_bump],
            ];
            let pool_signer = &[&pool_seeds[..]];
            
            token::transfer(
                CpiContext::new_with_signer(
                    ctx.accounts.token_program.to_account_info(),
                    Transfer {
                        from: ctx.accounts.token_vault.to_account_info(),
                        to: ctx.accounts.liquidator_reward_account.to_account_info(),
                        authority: ctx.accounts.pool.to_account_info(),
                    },
                    pool_signer,
                ),
                liquidator_reward,
            )?;
        }
    }

    // Emit liquidation event
    emit!(PositionLiquidated {
        liquidator: ctx.accounts.liquidator.key(),
        position_owner: ctx.accounts.position_owner.key(),
        pool: ctx.accounts.pool.key(),
        position: ctx.accounts.position.key(),
        position_size: position_size_original,
        borrowed_amount: position.delta_k as u64, // Report original delta_k
        expected_output: payout_u128 as u64,
        liquidator_reward,
        timestamp: Clock::get()?.unix_timestamp,
    });
    
    // Position account is automatically closed due to the close = liquidator constraint
    
    Ok(())
} 