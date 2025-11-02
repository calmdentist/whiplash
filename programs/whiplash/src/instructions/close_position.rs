use anchor_lang::prelude::*;
use anchor_spl::{
    token::{self, Token, TokenAccount, Transfer},
};
use crate::{state::*, events::*, WhiplashError};

#[derive(Accounts)]
pub struct ClosePosition<'info> {
    #[account(mut)]
    pub user: Signer<'info>,
    
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
            user.key().as_ref(),
            position.nonce.to_le_bytes().as_ref(),
        ],
        bump,
        close = user,
        constraint = position.authority == user.key() @ WhiplashError::InvalidPosition,
        constraint = position.pool == pool.key() @ WhiplashError::InvalidPosition,
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
    // Calculate funding fees and effective delta_k
    // -----------------------------------------------------------------
    
    let position_size = position.size;
    let delta_k_original: u128 = position.delta_k;
    
    // Calculate funding fees owed by this position
    let funding_due = pool.calculate_position_funding_fee(
        position.entry_funding_rate_index,
        delta_k_original,
    )?;
    
    // Current total reserves (real + virtual)
    let total_x: u128 = pool.lamports
        .checked_add(pool.virtual_sol_amount)
        .ok_or(error!(WhiplashError::MathOverflow))? as u128;
    let total_y: u128 = pool.token_y_amount
        .checked_add(pool.virtual_token_y_amount)
        .ok_or(error!(WhiplashError::MathOverflow))? as u128;
    
    // Calculate delta_k repaid through funding fees
    // delta_k_repaid = funding_due * y_current
    let delta_k_repaid = funding_due
        .checked_mul(total_y)
        .ok_or(error!(WhiplashError::MathOverflow))?;
    
    // Calculate effective delta_k after accounting for funding fees
    let delta_k: u128 = delta_k_original.saturating_sub(delta_k_repaid);

    let position_size_u128: u128 = position_size as u128;

    // Determine payout depending on position side
    let (payout_u128, is_liquidatable) = if position.is_long {
        // Long: user returns Y tokens and gets SOL
        // X_out = (x * y_pos - delta_k) / (y + y_pos)
        let product_val = total_x
            .checked_mul(position_size_u128)
            .ok_or(error!(WhiplashError::MathOverflow))?;

        let numerator = if product_val <= delta_k {
            0u128
        } else {
            product_val
                .checked_sub(delta_k)
                .ok_or(error!(WhiplashError::MathOverflow))?
        };

        if numerator == 0u128 {
            (0u128, true)
        } else {
            let denominator = total_y
                .checked_add(position_size_u128)
                .ok_or(error!(WhiplashError::MathOverflow))?;
            (
                numerator
                    .checked_div(denominator)
                    .ok_or(error!(WhiplashError::MathOverflow))?,
                false,
            )
        }
    } else {
        // Short: user returns SOL (x_pos) and gets Y tokens
        // Y_out = (x_pos * y - delta_k) / (x + x_pos)
        let product_val = position_size_u128
            .checked_mul(total_y)
            .ok_or(error!(WhiplashError::MathOverflow))?;

        let numerator = if product_val <= delta_k {
            0u128
        } else {
            product_val
                .checked_sub(delta_k)
                .ok_or(error!(WhiplashError::MathOverflow))?
        };

        if numerator == 0u128 {
            (0u128, true)
        } else {
            let denominator = total_x
                .checked_add(position_size_u128)
                .ok_or(error!(WhiplashError::MathOverflow))?;
            (
                numerator
                    .checked_div(denominator)
                    .ok_or(error!(WhiplashError::MathOverflow))?,
                false,
            )
        }
    };

    // If payout is zero, the position should be liquidated instead of closed
    require!(!is_liquidatable && payout_u128 > 0, WhiplashError::PositionNotClosable);

    if payout_u128 > u64::MAX as u128 {
        return Err(error!(WhiplashError::MathOverflow));
    }

    let user_output: u64 = payout_u128 as u64;
    
    // Get PDA info for signing
    let pool_bump = pool.bump;
    let pool_mint = pool.token_y_mint;
    
    // Handle based on position type
    // Note: Positions are virtual - tokens were never physically transferred out of the pool
    if position.is_long {
        // LONG POSITION: User has virtual claim on Y tokens, gets SOL back
        
        // 1. Update pool state
        {
            let pool = &mut ctx.accounts.pool;
            // Return the position's virtual tokens to available pool reserves
            pool.token_y_amount = pool.token_y_amount
                .checked_add(position_size)
                .ok_or(error!(WhiplashError::MathOverflow))?;
            
            // Deduct SOL being paid to the user
            pool.lamports = pool.lamports
                .checked_sub(user_output)
                .ok_or(error!(WhiplashError::MathOverflow))?;
            
            // Remove leveraged amounts
            pool.leveraged_token_y_amount -= position.leveraged_token_amount;
            
            // Update funding fee accounting
            pool.unrealized_funding_fees = pool.unrealized_funding_fees
                .saturating_sub(funding_due);
            
            // Remove this position's delta_k from the total
            pool.total_delta_k = pool.total_delta_k
                .saturating_sub(delta_k_original);
        }
        
        // 2. Transfer SOL payout to user (direct lamport transfer from pool)
        let dest_starting_lamports = ctx.accounts.user.lamports();
        let source_account_info = ctx.accounts.pool.to_account_info();
        
        **source_account_info.try_borrow_mut_lamports()? = source_account_info.lamports()
            .checked_sub(user_output)
            .ok_or(error!(WhiplashError::InsufficientFunds))?;
            
        **ctx.accounts.user.to_account_info().try_borrow_mut_lamports()? = dest_starting_lamports
            .checked_add(user_output)
            .ok_or(error!(WhiplashError::MathOverflow))?;
    } else {
        // SHORT POSITION: User has virtual claim on SOL, gets Y tokens back
        
        // 1. Update pool state
        {
            let pool = &mut ctx.accounts.pool;
            // Return the position's virtual SOL to available pool reserves
            pool.lamports = pool.lamports
                .checked_add(position_size)
                .ok_or(error!(WhiplashError::MathOverflow))?;
                
            // Deduct tokens being sent to the user
            pool.token_y_amount = pool.token_y_amount
                .checked_sub(user_output)
                .ok_or(error!(WhiplashError::MathOverflow))?;

            // Remove leveraged amounts
            pool.leveraged_sol_amount -= position.leveraged_token_amount;
            
            // Update funding fee accounting
            pool.unrealized_funding_fees = pool.unrealized_funding_fees
                .saturating_sub(funding_due);
            
            // Remove this position's delta_k from the total
            pool.total_delta_k = pool.total_delta_k
                .saturating_sub(delta_k_original);
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
                    from: ctx.accounts.token_y_vault.to_account_info(),
                    to: ctx.accounts.user_token_out.to_account_info(),
                    authority: ctx.accounts.pool.to_account_info(),
                },
                pool_signer,
            ),
            user_output,
        )?;
    }
    
    // Emit close position event
    emit!(PositionClosed {
        user: ctx.accounts.user.key(),
        pool: ctx.accounts.pool.key(),
        position: ctx.accounts.position.key(),
        is_long: position.is_long,
        position_size,
        borrowed_amount: 0u64,
        output_amount: payout_u128 as u64,
        user_received: user_output,
        timestamp: Clock::get()?.unix_timestamp,
    });
    
    // Position account is automatically closed due to the close = user constraint
    // No position token account to close since positions are virtual
    
    Ok(())
} 