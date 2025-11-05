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
            pool.token_y_mint.as_ref(),
        ],
        bump = pool.bump,
    )]
    pub pool: Account<'info, Pool>,
    
    #[account(
        mut,
        constraint = token_y_vault.key() == pool.token_y_vault @ WhiplashError::InvalidTokenAccounts,
        constraint = token_y_vault.mint == pool.token_y_mint @ WhiplashError::InvalidTokenAccounts,
        constraint = token_y_vault.owner == pool.key() @ WhiplashError::InvalidTokenAccounts,
    )]
    pub token_y_vault: Account<'info, TokenAccount>,
    
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
    let current_timestamp = Clock::get()?.unix_timestamp;
    ctx.accounts.pool.update_funding_accumulators(current_timestamp)?;
    
    let position = &ctx.accounts.position;
    let pool = &ctx.accounts.pool;
    
    // -----------------------------------------------------------------
    // Calculate effective position values using amortization formula
    // f(t) = 1 - (I(t) - I(t_open))
    // y_effective = y_original * f(t)
    // delta_k_effective = delta_k_original * f(t)
    // -----------------------------------------------------------------

    let position_size_original = position.size;
    let delta_k_original: u128 = position.delta_k;
    
    // Calculate the index difference (funding accrued)
    const INDEX_PRECISION_BITS: u32 = 64;
    const INDEX_PRECISION: u128 = 1u128 << INDEX_PRECISION_BITS;
    
    let index_diff = pool.cumulative_funding_rate_index
        .checked_sub(position.entry_funding_rate_index)
        .ok_or(error!(WhiplashError::MathUnderflow))?;
    
    // Calculate effective position size: y_effective = y_original * (1 - index_diff / PRECISION)
    // Rearranged to: y_effective = y_original - (y_original * index_diff / PRECISION)
    let position_size_reduction = (position_size_original as u128)
        .checked_mul(index_diff)
        .ok_or(error!(WhiplashError::MathOverflow))?
        .checked_div(INDEX_PRECISION)
        .ok_or(error!(WhiplashError::MathOverflow))?;
    
    let position_size_u128: u128 = (position_size_original as u128)
        .checked_sub(position_size_reduction)
        .ok_or(error!(WhiplashError::MathUnderflow))?;
    
    // Calculate effective delta_k: delta_k_effective = delta_k_original * (1 - index_diff / PRECISION)
    // Rearranged to: delta_k_effective = delta_k_original - (delta_k_original * index_diff / PRECISION)
    let delta_k_reduction = delta_k_original
        .checked_mul(index_diff)
        .ok_or(error!(WhiplashError::MathOverflow))?
        .checked_div(INDEX_PRECISION)
        .ok_or(error!(WhiplashError::MathOverflow))?;
    
    let delta_k: u128 = delta_k_original
        .checked_sub(delta_k_reduction)
        .ok_or(error!(WhiplashError::MathUnderflow))?;

    // Current total reserves
    let total_x: u128 = pool.lamports as u128;
    let total_y: u128 = pool.token_y_amount as u128;

    // Calculate expected payout and liquidation threshold
    let (expected_payout, liquidation_threshold) = if position.is_long {
        // Long: user returns Y tokens and gets SOL
        // X_out = (x * y_pos - delta_k) / (y + y_pos)
        let product_val = total_x
            .checked_mul(position_size_u128)
            .ok_or(error!(WhiplashError::MathOverflow))?;

        let expected_payout = if product_val <= delta_k {
            0u128
        } else {
            let numerator = product_val
                .checked_sub(delta_k)
                .ok_or(error!(WhiplashError::MathOverflow))?;
            let denominator = total_y
                .checked_add(position_size_u128)
                .ok_or(error!(WhiplashError::MathOverflow))?;
            numerator
                .checked_div(denominator)
                .ok_or(error!(WhiplashError::MathOverflow))?
        };

        // Liquidation threshold: (delta_k / x_current) * 1.05
        let threshold = delta_k
            .checked_mul(105)
            .ok_or(error!(WhiplashError::MathOverflow))?
            .checked_div(100)
            .ok_or(error!(WhiplashError::MathOverflow))?
            .checked_div(total_x)
            .ok_or(error!(WhiplashError::MathOverflow))?;

        (expected_payout, threshold)
    } else {
        // Short: user returns SOL and gets Y tokens
        // Y_out = (x_pos * y - delta_k) / (x + x_pos)
        let product_val = position_size_u128
            .checked_mul(total_y)
            .ok_or(error!(WhiplashError::MathOverflow))?;

        let expected_payout = if product_val <= delta_k {
            0u128
        } else {
            let numerator = product_val
                .checked_sub(delta_k)
                .ok_or(error!(WhiplashError::MathOverflow))?;
            let denominator = total_x
                .checked_add(position_size_u128)
                .ok_or(error!(WhiplashError::MathOverflow))?;
            numerator
                .checked_div(denominator)
                .ok_or(error!(WhiplashError::MathOverflow))?
        };

        // Liquidation threshold: (delta_k / y_current) * 1.05
        let threshold = delta_k
            .checked_mul(105)
            .ok_or(error!(WhiplashError::MathOverflow))?
            .checked_div(100)
            .ok_or(error!(WhiplashError::MathOverflow))?
            .checked_div(total_y)
            .ok_or(error!(WhiplashError::MathOverflow))?;

        (expected_payout, threshold)
    };

    // Check if position is liquidatable: expected_payout <= threshold
    // comment out require block for testing
    require!(
        expected_payout <= liquidation_threshold,
        WhiplashError::PositionNotLiquidatable
    );

    // -----------------------------------------------------------------
    // Execute liquidation
    // -----------------------------------------------------------------

    // Convert effective position size to u64 for liquidation calculations
    let effective_position_size_u64 = if position_size_u128 > u64::MAX as u128 {
        return Err(error!(WhiplashError::MathOverflow));
    } else {
        position_size_u128 as u64
    };
    
    // Calculate how much of the position was paid through funding fees
    let funding_fees_paid = delta_k_original
        .checked_sub(delta_k)
        .ok_or(error!(WhiplashError::MathUnderflow))?;

    // Calculate exact amount needed to restore invariant using ceiling division
    // to ensure we restore enough tokens to fully restore the invariant
    let restore_amount = if position.is_long {
        // For longs: ceil(delta_k / x_current) = (delta_k + x_current - 1) / x_current
        delta_k
            .checked_add(total_x)
            .ok_or(error!(WhiplashError::MathOverflow))?
            .checked_sub(1)
            .ok_or(error!(WhiplashError::MathUnderflow))?
            .checked_div(total_x)
            .ok_or(error!(WhiplashError::MathOverflow))?
    } else {
        // For shorts: ceil(delta_k / y_current) = (delta_k + y_current - 1) / y_current
        delta_k
            .checked_add(total_y)
            .ok_or(error!(WhiplashError::MathOverflow))?
            .checked_sub(1)
            .ok_or(error!(WhiplashError::MathUnderflow))?
            .checked_div(total_y)
            .ok_or(error!(WhiplashError::MathOverflow))?
    };

    // Ensure restore amount fits in u64
    if restore_amount > u64::MAX as u128 {
        return Err(error!(WhiplashError::MathOverflow));
    }
    let restore_amount_u64 = restore_amount as u64;

    // Calculate liquidator reward using effective position size
    let liquidator_reward = effective_position_size_u64
        .checked_sub(restore_amount_u64)
        .ok_or(error!(WhiplashError::MathUnderflow))?;
    
    // Get pool signer seeds for transferring from vault
    let pool_mint = ctx.accounts.pool.token_y_mint;
    let pool_bump = ctx.accounts.pool.bump;

    // Handle based on position type
    // Note: Positions are virtual - tokens were never physically transferred out of the pool
    if position.is_long {
        // LONG POSITION LIQUIDATION
        // Position has virtual claim on Y tokens, liquidator gets Y tokens as reward
        
        // 1. Update pool state
        {
            let pool = &mut ctx.accounts.pool;
            // Return position's virtual tokens to pool (restore amount stays in pool)
            pool.token_y_amount = pool.token_y_amount
                .checked_add(restore_amount_u64)
                .ok_or(error!(WhiplashError::MathOverflow))?;
            
            // Deduct liquidator reward from pool
            pool.token_y_amount = pool.token_y_amount
                .checked_sub(liquidator_reward)
                .ok_or(error!(WhiplashError::MathUnderflow))?;
            
            pool.leveraged_token_y_amount = pool.leveraged_token_y_amount
                .checked_sub(position.leveraged_token_amount)
                .ok_or(error!(WhiplashError::MathUnderflow))?;
            
            // Update funding fee accounting
            // Convert unrealized fees to realized based on what was actually paid
            pool.unrealized_funding_fees = pool.unrealized_funding_fees
                .saturating_sub(funding_fees_paid);
            
            // Remove this position's original delta_k from the total
            pool.total_delta_k = pool.total_delta_k
                .saturating_sub(delta_k_original);
        }
        
        // 2. Transfer liquidator reward (tokens from vault to liquidator)
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
                        from: ctx.accounts.token_y_vault.to_account_info(),
                        to: ctx.accounts.liquidator_reward_account.to_account_info(),
                        authority: ctx.accounts.pool.to_account_info(),
                    },
                    pool_signer,
                ),
                liquidator_reward,
            )?;
        }
    } else {
        // SHORT POSITION LIQUIDATION
        // Position has virtual claim on SOL, liquidator gets SOL as reward
        
        // 1. Update pool state
        {
            let pool = &mut ctx.accounts.pool;
            // Return position's virtual SOL to pool (restore amount stays in pool)
            pool.lamports = pool.lamports
                .checked_add(restore_amount_u64)
                .ok_or(error!(WhiplashError::MathOverflow))?;
            
            // Deduct liquidator reward from pool
            pool.lamports = pool.lamports
                .checked_sub(liquidator_reward)
                .ok_or(error!(WhiplashError::MathUnderflow))?;
            
            pool.leveraged_sol_amount = pool.leveraged_sol_amount
                .checked_sub(position.leveraged_token_amount)
                .ok_or(error!(WhiplashError::MathUnderflow))?;
            
            // Update funding fee accounting
            // Convert unrealized fees to realized based on what was actually paid
            pool.unrealized_funding_fees = pool.unrealized_funding_fees
                .saturating_sub(funding_fees_paid);
            
            // Remove this position's original delta_k from the total
            pool.total_delta_k = pool.total_delta_k
                .saturating_sub(delta_k_original);
        }
        
        // 2. Transfer liquidator reward (SOL from pool to liquidator)
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
    }

    // Emit liquidation event
    emit!(PositionLiquidated {
        liquidator: ctx.accounts.liquidator.key(),
        position_owner: ctx.accounts.position_owner.key(),
        pool: ctx.accounts.pool.key(),
        position: ctx.accounts.position.key(),
        position_size: position_size_original,
        borrowed_amount: position.leveraged_token_amount,
        expected_output: expected_payout as u64,
        liquidator_reward,
        timestamp: Clock::get()?.unix_timestamp,
    });
    
    // Position account is automatically closed due to the close = liquidator constraint
    
    Ok(())
} 