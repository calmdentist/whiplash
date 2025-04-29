use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};
use crate::{state::*, events::*, WhiplashError};

#[derive(Accounts)]
pub struct AddLiquidity<'info> {
    #[account(mut)]
    pub provider: Signer<'info>,
    
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
        constraint = provider_token_y.mint == pool.token_y_mint @ WhiplashError::InvalidTokenAccounts,
        constraint = provider_token_y.owner == provider.key() @ WhiplashError::InvalidTokenAccounts,
    )]
    pub provider_token_y: Account<'info, TokenAccount>,
    
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

pub fn handle_add_liquidity(
    ctx: Context<AddLiquidity>, 
    amount_sol_desired: u64, 
    amount_y_desired: u64, 
    amount_sol_min: u64, 
    amount_y_min: u64
) -> Result<()> {
    // Validate amounts
    if amount_sol_desired == 0 || amount_y_desired == 0 {
        return Err(error!(WhiplashError::ZeroLiquidity));
    }
    
    // Check if this is the first time liquidity is being added
    let is_first_liquidity = ctx.accounts.pool.virtual_sol_reserve == 0 && ctx.accounts.pool.token_y_amount == 0;
    
    // Determine optimal amounts to add (similar to Uniswap V2)
    let (amount_sol, amount_y) = if is_first_liquidity {
        // For first liquidity, use the desired amounts directly
        (amount_sol_desired, amount_y_desired)
    } else {
        // Calculate optimal amounts based on the existing ratio
        let amount_y_optimal = calculate_optimal_amount(
            amount_sol_desired,
            ctx.accounts.pool.token_y_amount,
            ctx.accounts.pool.virtual_sol_reserve,
        )?;
        
        if amount_y_optimal <= amount_y_desired {
            // The optimal amount of Y is less than desired, so we'll use all of SOL and that amount of Y
            require!(
                amount_y_optimal >= amount_y_min,
                WhiplashError::SlippageToleranceExceeded
            );
            (amount_sol_desired, amount_y_optimal)
        } else {
            // The optimal amount of Y is more than desired, so calculate optimal SOL based on desired Y
            let amount_sol_optimal = calculate_optimal_amount(
                amount_y_desired,
                ctx.accounts.pool.virtual_sol_reserve,
                ctx.accounts.pool.token_y_amount,
            )?;
            require!(
                amount_sol_optimal <= amount_sol_desired,
                WhiplashError::MathOverflow
            );
            require!(
                amount_sol_optimal >= amount_sol_min,
                WhiplashError::SlippageToleranceExceeded
            );
            (amount_sol_optimal, amount_y_desired)
        }
    };
    
    // Transfer token Y from provider to vault
    let cpi_accounts_y = Transfer {
        from: ctx.accounts.provider_token_y.to_account_info(),
        to: ctx.accounts.token_y_vault.to_account_info(),
        authority: ctx.accounts.provider.to_account_info(),
    };
    let cpi_program = ctx.accounts.token_program.to_account_info();
    let cpi_ctx_y = CpiContext::new(cpi_program, cpi_accounts_y);
    token::transfer(cpi_ctx_y, amount_y)?;
    
    // Update pool reserves
    let pool = &mut ctx.accounts.pool;
    pool.virtual_sol_reserve = pool.virtual_sol_reserve.checked_add(amount_sol)
        .ok_or(error!(WhiplashError::MathOverflow))?;
    pool.token_y_amount = pool.token_y_amount.checked_add(amount_y)
        .ok_or(error!(WhiplashError::MathOverflow))?;
    
    // Emit liquidity added event
    emit!(LiquidityAdded {
        provider: ctx.accounts.provider.key(),
        pool: ctx.accounts.pool.key(),
        amount_x: amount_sol,
        amount_y,
        timestamp: Clock::get()?.unix_timestamp,
    });
    
    Ok(())
}

// Helper function to calculate the optimal amount of the second token
// based on the amount of the first token and the current reserves
fn calculate_optimal_amount(
    amount_a: u64,
    reserve_b: u64,
    reserve_a: u64,
) -> Result<u64> {
    if reserve_a == 0 || reserve_b == 0 {
        return Ok(amount_a);
    }
    
    let amount_b = (amount_a as u128)
        .checked_mul(reserve_b as u128)
        .ok_or(error!(WhiplashError::MathOverflow))?
        .checked_div(reserve_a as u128)
        .ok_or(error!(WhiplashError::MathOverflow))?;
        
    // Check for overflow before returning
    if amount_b > u64::MAX as u128 {
        return Err(error!(WhiplashError::MathOverflow));
    }
    
    Ok(amount_b as u64)
} 