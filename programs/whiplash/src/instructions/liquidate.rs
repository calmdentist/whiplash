use anchor_lang::prelude::*;
use anchor_spl::{
    token::{self, Token, TokenAccount, Transfer},
    associated_token::AssociatedToken,
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
    
    #[account(
        mut,
        constraint = position_token_account.key() == position.position_vault @ WhiplashError::InvalidTokenAccounts,
    )]
    pub position_token_account: Account<'info, TokenAccount>,
    
    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
}

pub fn handle_liquidate(ctx: Context<Liquidate>) -> Result<()> {
    // First check if the position is liquidatable
    let position = &ctx.accounts.position;
    let pool = &ctx.accounts.pool;
    
    // -----------------------------------------------------------------
    // New liquidation condition based on Δk
    // A position is liquidatable when its payout would be zero or negative.
    // For a long: x_current * y_pos <= delta_k
    // For a short: y_current * x_pos <= delta_k
    // -----------------------------------------------------------------

    let position_size_u128: u128 = position.size as u128;
    let position_size = position.size;

    // Current total reserves (real + virtual)
    let total_x: u128 = pool.lamports
        .checked_add(pool.virtual_sol_amount)
        .ok_or(error!(WhiplashError::MathOverflow))? as u128;
    let total_y: u128 = pool.token_y_amount
        .checked_add(pool.virtual_token_y_amount)
        .ok_or(error!(WhiplashError::MathOverflow))? as u128;

    // Compute payout using same formula as close_position
    let payout_u128: u128 = if position.is_long {
        // X_out = (x * y_pos - delta_k) / (y + y_pos)
        let product_val = total_x
            .checked_mul(position_size_u128)
            .ok_or(error!(WhiplashError::MathOverflow))?;

        let numerator = if product_val <= position.delta_k {
            0u128
        } else {
            product_val
                .checked_sub(position.delta_k)
                .ok_or(error!(WhiplashError::MathOverflow))?
        };

        let denominator = total_y
            .checked_add(position_size_u128)
            .ok_or(error!(WhiplashError::MathOverflow))?;

        numerator
            .checked_div(denominator)
            .ok_or(error!(WhiplashError::MathOverflow))?
    } else {
        // Y_out = (x_pos * y - delta_k) / (x + x_pos)
        let product_val = position_size_u128
            .checked_mul(total_y)
            .ok_or(error!(WhiplashError::MathOverflow))?;

        let numerator = if product_val <= position.delta_k {
            0u128
        } else {
            product_val
                .checked_sub(position.delta_k)
                .ok_or(error!(WhiplashError::MathOverflow))?
        };

        let denominator = total_x
            .checked_add(position_size_u128)
            .ok_or(error!(WhiplashError::MathOverflow))?;

        numerator
            .checked_div(denominator)
            .ok_or(error!(WhiplashError::MathOverflow))?
    };

    // A position is only liquidatable when the stored Δk can be fully restored –
    // i.e. when the traderʼs payout is exactly zero (or 1 lamport of rounding).
    // If x_current * y_pos  <  Δk the position is in the "limbo" state and must
    // NOT be liquidated yet.
    require!(payout_u128 == 0u128, WhiplashError::PositionNotLiquidatable);
    
    // If we get here, the position is eligible for liquidation
    // Handle based on position type
    if position.is_long {
        // Long position: transfer tokens from position to vault
        let bump = *ctx.bumps.get("position").unwrap();
        let pool_key = ctx.accounts.pool.key();
        let position_owner_key = ctx.accounts.position_owner.key();
        let position_nonce = position.nonce;
        
        let nonce_bytes = position_nonce.to_le_bytes();
        let position_seeds = &[
            b"position".as_ref(),
            pool_key.as_ref(),
            position_owner_key.as_ref(),
            nonce_bytes.as_ref(),
            &[bump],
        ];
        
        let position_signer = &[&position_seeds[..]];
        
        let cpi_accounts = Transfer {
            from: ctx.accounts.position_token_account.to_account_info(),
            to: ctx.accounts.token_y_vault.to_account_info(),
            authority: ctx.accounts.position.to_account_info(),
        };
        let cpi_ctx = CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            cpi_accounts,
            position_signer,
        );
        token::transfer(cpi_ctx, position_size)?;
        
        // Update pool token reserves
        let pool = &mut ctx.accounts.pool;
        pool.token_y_amount = pool.token_y_amount
            .checked_add(position_size)
            .ok_or(error!(WhiplashError::MathOverflow))?;
    } else {
        // Short position: transfer SOL from position back to pool
        // Store key references to avoid temporary value issues
        let position_token_account_info = ctx.accounts.position_token_account.to_account_info();
        let pool_info = ctx.accounts.pool.to_account_info();
        
        let position_lamports = position_token_account_info.lamports();
        let pool_lamports = pool_info.lamports();
        
        // Calculate new lamport values
        let new_position_lamports = position_lamports.checked_sub(position_size)
            .ok_or(error!(WhiplashError::MathOverflow))?;
        let new_pool_lamports = pool_lamports.checked_add(position_size)
            .ok_or(error!(WhiplashError::MathOverflow))?;
        
        // Update lamports
        **position_token_account_info.try_borrow_mut_lamports()? = new_position_lamports;
        **pool_info.try_borrow_mut_lamports()? = new_pool_lamports;
        
        // Update pool SOL reserves
        let pool = &mut ctx.accounts.pool;
        pool.lamports = pool.lamports
            .checked_add(position_size)
            .ok_or(error!(WhiplashError::MathOverflow))?;
    }
    
    // Emit liquidation event
    emit!(PositionLiquidated {
        liquidator: ctx.accounts.liquidator.key(),
        position_owner: ctx.accounts.position_owner.key(),
        pool: ctx.accounts.pool.key(),
        position: ctx.accounts.position.key(),
        position_size,
        borrowed_amount: 0u64, // No borrowed amount used in the new condition
        expected_output: 0u64, // No expected output used in the new condition
        liquidator_reward: 0, // No reward for now
        timestamp: Clock::get()?.unix_timestamp,
    });
    
    // Position account is automatically closed due to the close = liquidator constraint
    
    Ok(())
} 