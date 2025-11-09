use anchor_lang::prelude::*;
use anchor_spl::{
    token::{self, Token, TokenAccount, Transfer},
};
use crate::{state::*, events::*, FacemeltError};

#[derive(Accounts)]
pub struct ClosePosition<'info> {
    #[account(mut)]
    pub user: Signer<'info>,
    
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
        constraint = token_vault.key() == pool.token_vault @ FacemeltError::InvalidTokenAccounts,
        constraint = token_vault.mint == pool.token_mint @ FacemeltError::InvalidTokenAccounts,
        constraint = token_vault.owner == pool.key() @ FacemeltError::InvalidTokenAccounts,
    )]
    pub token_vault: Account<'info, TokenAccount>,
    
    #[account(
        mut,
        seeds = [
            b"position".as_ref(),
            pool.key().as_ref(),
            user.key().as_ref(),
            position.nonce.to_le_bytes().as_ref(),
        ],
        bump,
        close = user,
        constraint = position.authority == user.key() @ FacemeltError::InvalidPosition,
        constraint = position.pool == pool.key() @ FacemeltError::InvalidPosition,
    )]
    pub position: Account<'info, Position>,
    
    /// CHECK: This can be either an SPL token account OR a native SOL account (user wallet)
    #[account(mut)]
    pub user_token_out: UncheckedAccount<'info>,
    
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

pub fn handle_close_position(ctx: Context<ClosePosition>) -> Result<()> {
    // Update funding rate accumulators before any position operations
    let current_timestamp = Clock::get()?.unix_timestamp;
    ctx.accounts.pool.update_funding_accumulators(current_timestamp)?;
    
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
    let position_size_u128: u128 = (position_size_original as u128)
        .checked_mul(remaining_factor)
        .ok_or(error!(FacemeltError::MathOverflow))?
        .checked_div(PRECISION)
        .ok_or(error!(FacemeltError::MathOverflow))?;
    
    // Calculate effective delta_k: effective_delta_k = original_delta_k * remaining_factor / PRECISION
    let delta_k: u128 = delta_k_original
        .checked_mul(remaining_factor)
        .ok_or(error!(FacemeltError::MathOverflow))?
        .checked_div(PRECISION)
        .ok_or(error!(FacemeltError::MathOverflow))?;
    
    // Current effective reserves
    let x_e: u128 = pool.effective_sol_reserve as u128;
    let y_e: u128 = pool.effective_token_reserve as u128;

    // Determine payout depending on position side (using architecture formulas)
    let (payout_u128, is_liquidatable) = if position.is_long {
        // Long: user returns tokens and gets SOL
        // payout = (x_e * effective_size - effective_delta_k) / (y_e + effective_size)
        let product_val = x_e
            .checked_mul(position_size_u128)
            .ok_or(error!(FacemeltError::MathOverflow))?;

        let numerator = if product_val <= delta_k {
            0u128
        } else {
            product_val
                .checked_sub(delta_k)
                .ok_or(error!(FacemeltError::MathOverflow))?
        };

        if numerator == 0u128 {
            (0u128, true)
        } else {
            let denominator = y_e
                .checked_add(position_size_u128)
                .ok_or(error!(FacemeltError::MathOverflow))?;
            (
                numerator
                    .checked_div(denominator)
                    .ok_or(error!(FacemeltError::MathOverflow))?,
                false,
            )
        }
    } else {
        // Short: user returns SOL and gets tokens
        // payout = (y_e * effective_size - effective_delta_k) / (x_e + effective_size)
        let product_val = position_size_u128
            .checked_mul(y_e)
            .ok_or(error!(FacemeltError::MathOverflow))?;

        let numerator = if product_val <= delta_k {
            0u128
        } else {
            product_val
                .checked_sub(delta_k)
                .ok_or(error!(FacemeltError::MathOverflow))?
        };

        if numerator == 0u128 {
            (0u128, true)
        } else {
            let denominator = x_e
                .checked_add(position_size_u128)
                .ok_or(error!(FacemeltError::MathOverflow))?;
            (
                numerator
                    .checked_div(denominator)
                    .ok_or(error!(FacemeltError::MathOverflow))?,
                false,
            )
        }
    };

    // If payout is zero, the position should be liquidated instead of closed
    require!(!is_liquidatable && payout_u128 > 0, FacemeltError::PositionNotClosable);

    if payout_u128 > u64::MAX as u128 {
        return Err(error!(FacemeltError::MathOverflow));
    }

    let user_output: u64 = payout_u128 as u64;
    
    // Convert effective position sizes to u64 for pool updates
    let effective_position_size_u64 = if position_size_u128 > u64::MAX as u128 {
        return Err(error!(FacemeltError::MathOverflow));
    } else {
        position_size_u128 as u64
    };
    
    // Get PDA info for signing
    let pool_bump = pool.bump;
    let pool_mint = pool.token_mint;
    
    // Handle based on position type
    // Note: Positions are virtual - tokens were never physically transferred out of the pool
    if position.is_long {
        // LONG POSITION: User has virtual claim on tokens, gets SOL back
        
        // 1. Update pool state
        {
            let pool = &mut ctx.accounts.pool;
            // Return the position's effective virtual tokens to effective reserves
            pool.effective_token_reserve = pool.effective_token_reserve
                .checked_add(effective_position_size_u64)
                .ok_or(error!(FacemeltError::MathOverflow))?;
            
            // Deduct SOL being paid to the user from effective reserves
            pool.effective_sol_reserve = pool.effective_sol_reserve
                .checked_sub(user_output)
                .ok_or(error!(FacemeltError::MathOverflow))?;
            
            // Also deduct from real SOL reserves (actual payout)
            pool.sol_reserve = pool.sol_reserve
                .checked_sub(user_output)
                .ok_or(error!(FacemeltError::MathOverflow))?;
            
            // Remove this position's EFFECTIVE delta_k from the longs pool
            // Funding fees reduce total_delta_k proportionally across all positions
            // So we subtract the effective delta_k (original * remaining_factor)
            pool.total_delta_k_longs = pool.total_delta_k_longs
                .checked_sub(delta_k)
                .ok_or(error!(FacemeltError::MathUnderflow))?;
            
            // Handle rounding errors: if remaining delta_k is very small (< 0.01% of effective_k), round to zero
            let effective_k = (pool.effective_sol_reserve as u128)
                .checked_mul(pool.effective_token_reserve as u128)
                .ok_or(error!(FacemeltError::MathOverflow))?;
            let threshold = effective_k / 10000; // 0.01% threshold
            if pool.total_delta_k_longs < threshold {
                pool.total_delta_k_longs = 0;
            }
        }
        
        // 2. Transfer SOL payout to user (direct lamport transfer from pool)
        let dest_starting_lamports = ctx.accounts.user.lamports();
        let source_account_info = ctx.accounts.pool.to_account_info();
        
        **source_account_info.try_borrow_mut_lamports()? = source_account_info.lamports()
            .checked_sub(user_output)
            .ok_or(error!(FacemeltError::InsufficientFunds))?;
            
        **ctx.accounts.user.to_account_info().try_borrow_mut_lamports()? = dest_starting_lamports
            .checked_add(user_output)
            .ok_or(error!(FacemeltError::MathOverflow))?;
    } else {
        // SHORT POSITION: User has virtual claim on SOL, gets tokens back
        
        // 1. Update pool state
        {
            let pool = &mut ctx.accounts.pool;
            // Return the position's effective virtual SOL to effective reserves
            pool.effective_sol_reserve = pool.effective_sol_reserve
                .checked_add(effective_position_size_u64)
                .ok_or(error!(FacemeltError::MathOverflow))?;
                
            // Deduct tokens being sent to the user from effective reserves
            pool.effective_token_reserve = pool.effective_token_reserve
                .checked_sub(user_output)
                .ok_or(error!(FacemeltError::MathOverflow))?;
            
            // Also deduct from real token reserves (actual payout)
            pool.token_reserve = pool.token_reserve
                .checked_sub(user_output)
                .ok_or(error!(FacemeltError::MathOverflow))?;
            
            // Remove this position's EFFECTIVE delta_k from the shorts pool
            // Funding fees reduce total_delta_k proportionally across all positions
            // So we subtract the effective delta_k (original * remaining_factor)
            pool.total_delta_k_shorts = pool.total_delta_k_shorts
                .checked_sub(delta_k)
                .ok_or(error!(FacemeltError::MathUnderflow))?;
            
            // Handle rounding errors: if remaining delta_k is very small (< 0.01% of effective_k), round to zero
            let effective_k = (pool.effective_sol_reserve as u128)
                .checked_mul(pool.effective_token_reserve as u128)
                .ok_or(error!(FacemeltError::MathOverflow))?;
            let threshold = effective_k / 10000; // 0.01% threshold
            if pool.total_delta_k_shorts < threshold {
                pool.total_delta_k_shorts = 0;
            }
        }
        
        // 2. Transfer token payout to user (from vault)
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
                    to: ctx.accounts.user_token_out.to_account_info(),
                    authority: ctx.accounts.pool.to_account_info(),
                },
                pool_signer,
            ),
            user_output,
        )?;
    }
    
    // If there are no remaining effective debts, snap effective reserves to real reserves.
    // This prevents small rounding-dust discrepancies from lingering when the book is flat.
    {
        let pool = &mut ctx.accounts.pool;
        if pool.total_delta_k_longs == 0 && pool.total_delta_k_shorts == 0 {
            pool.effective_sol_reserve = pool.sol_reserve;
            pool.effective_token_reserve = pool.token_reserve;
        }
    }
    
    // Emit close position event
    emit!(PositionClosed {
        user: ctx.accounts.user.key(),
        pool: ctx.accounts.pool.key(),
        position: ctx.accounts.position.key(),
        is_long: position.is_long,
        position_size: position_size_original,
        borrowed_amount: 0u64,
        output_amount: payout_u128 as u64,
        user_received: user_output,
        timestamp: Clock::get()?.unix_timestamp,
    });
    
    // Position account is automatically closed due to the close = user constraint
    // No position token account to close since positions are virtual
    
    Ok(())
} 