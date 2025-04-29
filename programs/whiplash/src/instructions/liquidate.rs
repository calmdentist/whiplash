use anchor_lang::prelude::*;
use anchor_spl::{
    token::{self, Token, TokenAccount, Transfer},
};
use crate::{state::*, events::*, WhiplashError};

#[derive(Accounts)]
pub struct Liquidate<'info> {
    #[account(mut)]
    pub liquidator: Signer<'info>,
    
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
        constraint = position.pool == pool.key() @ WhiplashError::InvalidPosition,
    )]
    pub position: Account<'info, Position>,
    
    #[account(
        mut,
        constraint = position_token_account.key() == position.position_vault @ WhiplashError::InvalidTokenAccounts,
        constraint = position_token_account.owner == position.key() @ WhiplashError::InvalidTokenAccounts,
    )]
    pub position_token_account: Account<'info, TokenAccount>,
    
    #[account(
        mut,
        constraint = liquidator_token_account.owner == liquidator.key() @ WhiplashError::InvalidTokenAccounts,
    )]
    pub liquidator_token_account: Account<'info, TokenAccount>,
    
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

pub fn handle_liquidate(ctx: Context<Liquidate>) -> Result<()> {
    let position = &ctx.accounts.position;
    let pool = &mut ctx.accounts.pool;
    
    // Calculate the liquidation price
    let borrowed_amount = position.collateral.checked_mul((position.leverage - 1) as u64)
        .ok_or(error!(WhiplashError::MathOverflow))?;
    
    // Calculate the current value of the position
    let position_value = pool.calculate_swap_y_to_sol(position.size)?;
    
    // Check if position is liquidatable
    require!(
        position_value < borrowed_amount,
        WhiplashError::PositionNotLiquidatable
    );
    
    // Transfer position tokens to the pool
    let position_signer_seeds = &[
        b"position".as_ref(),
        position.pool.as_ref(),
        position.authority.as_ref(),
        &[position.bump],
    ];
    let position_signer = &[&position_signer_seeds[..]];
    
    let cpi_accounts = Transfer {
        from: ctx.accounts.position_token_account.to_account_info(),
        to: ctx.accounts.token_y_vault.to_account_info(),
        authority: ctx.accounts.position.to_account_info(),
    };
    let cpi_program = ctx.accounts.token_program.to_account_info();
    let cpi_ctx = CpiContext::new_with_signer(cpi_program, cpi_accounts, position_signer);
    token::transfer(cpi_ctx, position.size)?;
    
    // Update pool reserves
    pool.token_y_amount = pool.token_y_amount.checked_add(position.size)
        .ok_or(error!(WhiplashError::MathOverflow))?;
    
    // Emit liquidation event
    emit!(PositionLiquidated {
        position: position.key(),
        pool: pool.key(),
        liquidator: ctx.accounts.liquidator.key(),
        position_size: position.size,
        timestamp: Clock::get()?.unix_timestamp,
    });
    
    Ok(())
} 