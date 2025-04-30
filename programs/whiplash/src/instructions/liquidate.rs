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
        ],
        bump,
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
    
    // Calculate the required output to repay the borrowed amount
    let borrowed_amount = position.collateral.checked_mul((position.leverage as u64) - 1)
        .ok_or(error!(WhiplashError::MathOverflow))?;
    
    // Get the current position value
    let position_size = position.size;
    let total_x = pool.lamports.checked_add(pool.virtual_sol_amount)
        .ok_or(error!(WhiplashError::MathOverflow))?;
    let total_y = pool.token_y_amount.checked_add(pool.virtual_token_y_amount)
        .ok_or(error!(WhiplashError::MathOverflow))?;
        
    // Using u128 for intermediate calculations
    let x_u128: u128 = total_x as u128;
    let y_u128: u128 = total_y as u128;
    let position_size_u128: u128 = position_size as u128;
    
    // Calculate the expected output based on position type
    let expected_output_u128 = if position.is_long {
        // Long position: holding tokens, need to calculate Token->SOL swap
        // Formula: (x * y_position) / (y + y_position)
        (x_u128.checked_mul(position_size_u128)
            .ok_or(error!(WhiplashError::MathOverflow))?)
            .checked_div(y_u128.checked_add(position_size_u128)
                .ok_or(error!(WhiplashError::MathOverflow))?)
            .ok_or(error!(WhiplashError::MathOverflow))?
    } else {
        // Short position: holding SOL, need to calculate SOL->Token swap
        // Formula: (y * x_position) / (x + x_position)
        (y_u128.checked_mul(position_size_u128)
            .ok_or(error!(WhiplashError::MathOverflow))?)
            .checked_div(x_u128.checked_add(position_size_u128)
                .ok_or(error!(WhiplashError::MathOverflow))?)
            .ok_or(error!(WhiplashError::MathOverflow))?
    };
    
    // Ensure the result fits in u64
    if expected_output_u128 > u64::MAX as u128 {
        return Err(error!(WhiplashError::MathOverflow));
    }
    
    let expected_output = expected_output_u128 as u64;
    
    // Liquidation condition check: expected_output < borrowed_amount
    require!(
        expected_output < borrowed_amount,
        WhiplashError::PositionNotLiquidatable
    );
    
    // If we get here, the position is eligible for liquidation
    // Handle based on position type
    if position.is_long {
        // Long position: transfer tokens from position to vault
        // Store key references to avoid temporary value issues
        let pool_key = ctx.accounts.pool.key();
        let position_owner_key = ctx.accounts.position_owner.key();
        let bump = *ctx.bumps.get("position").unwrap();
        
        let position_seeds = &[
            b"position".as_ref(),
            pool_key.as_ref(),
            position_owner_key.as_ref(),
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
        borrowed_amount,
        expected_output,
        liquidator_reward: 0, // No reward for now
        timestamp: Clock::get()?.unix_timestamp,
    });
    
    // Close the position account
    ctx.accounts.position.close(ctx.accounts.liquidator.to_account_info())?;
    
    Ok(())
} 