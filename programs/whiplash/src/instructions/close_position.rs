use anchor_lang::prelude::*;
use anchor_spl::{
    token::{self, Token, TokenAccount, Transfer},
    associated_token::AssociatedToken,
};
use crate::{state::*, events::*, WhiplashError, utils};

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
    
    #[account(
        mut,
        constraint = position_token_account.key() == position.position_vault @ WhiplashError::InvalidTokenAccounts,
    )]
    pub position_token_account: Account<'info, TokenAccount>,
    
    /// CHECK: This can be either an SPL token account OR a native SOL account (user wallet)
    #[account(mut)]
    pub user_token_out: UncheckedAccount<'info>,
    
    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
}

pub fn handle_close_position(ctx: Context<ClosePosition>) -> Result<()> {
    let position = &ctx.accounts.position;
    let pool = &ctx.accounts.pool;
    
    // Calculate the borrowed amount that needs to be repaid
    let borrowed_amount = position.collateral
        .checked_mul(position.leverage.checked_sub(10).unwrap_or(0) as u64)
        .ok_or(error!(WhiplashError::MathOverflow))?
        .checked_div(10u64)
        .ok_or(error!(WhiplashError::MathOverflow))?;
    
    // Get the current position value
    let position_size = position.size;
    let total_x = pool.lamports.checked_add(pool.virtual_sol_amount)
        .ok_or(error!(WhiplashError::MathOverflow))?;
    let total_y = pool.token_y_amount.checked_add(pool.virtual_token_y_amount)
        .ok_or(error!(WhiplashError::MathOverflow))?;
    
    // Calculate expected output using utility function
    let expected_output = utils::calculate_position_expected_output(
        total_x,
        total_y,
        position_size,
        position.is_long,
        position.leverage,
    )?;
    
    // Check if the output is sufficient to repay the borrowed amount
    require!(
        expected_output >= borrowed_amount,
        WhiplashError::InsufficientOutput
    );
    
    // Calculate user output
    let user_output = expected_output.checked_sub(borrowed_amount)
        .ok_or(error!(WhiplashError::MathOverflow))?;
    
    // Get PDA info for signing
    let pool_bump = pool.bump;
    let pool_mint = pool.token_y_mint;
    
    // Get the position bump from context
    let bump = *ctx.bumps.get("position").unwrap();
    let pool_key = ctx.accounts.pool.key();
    let user_key = ctx.accounts.user.key();
    let position_nonce = position.nonce;
    
    // Handle based on position type
    if position.is_long {
        // LONG POSITION (User holds Y tokens, gets SOL back)
        
        // 1. Transfer tokens from position to vault
        let nonce_bytes = position_nonce.to_le_bytes();
        let position_seeds = &[
            b"position".as_ref(),
            pool_key.as_ref(),
            user_key.as_ref(),
            nonce_bytes.as_ref(),
            &[bump],
        ];
        let position_signer = &[&position_seeds[..]];
        
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.position_token_account.to_account_info(),
                    to: ctx.accounts.token_y_vault.to_account_info(),
                    authority: ctx.accounts.position.to_account_info(),
                },
                position_signer,
            ),
            position_size,
        )?;
        
        // 2. Update pool state
        {
            let pool = &mut ctx.accounts.pool;
            pool.token_y_amount = pool.token_y_amount
                .checked_add(position_size)
                .ok_or(error!(WhiplashError::MathOverflow))?;
                
            pool.lamports = pool.lamports
                .checked_sub(user_output)
                .ok_or(error!(WhiplashError::MathOverflow))?;
        }
        
        // 3. Transfer SOL to user (direct lamport transfer)
        let dest_starting_lamports = ctx.accounts.user.lamports();
        let source_account_info = ctx.accounts.pool.to_account_info();
        
        **source_account_info.try_borrow_mut_lamports()? = source_account_info.lamports()
            .checked_sub(user_output)
            .ok_or(error!(WhiplashError::InsufficientFunds))?;
            
        **ctx.accounts.user.to_account_info().try_borrow_mut_lamports()? = dest_starting_lamports
            .checked_add(user_output)
            .ok_or(error!(WhiplashError::MathOverflow))?;
    } else {
        // SHORT POSITION (User holds SOL, gets Y tokens back)
        
        // For short positions, we only need to handle the accounting in the pool
        // and transfer tokens to the user. The SOL in the position account will be
        // returned to the user when the account is closed.
        
        // 1. Update pool state for accounting
        {
            let pool = &mut ctx.accounts.pool;
            // Record that the SOL has been returned to the pool
            pool.lamports = pool.lamports
                .checked_add(position_size)
                .ok_or(error!(WhiplashError::MathOverflow))?;
                
            // Deduct tokens being sent to the user
            pool.token_y_amount = pool.token_y_amount
                .checked_sub(user_output)
                .ok_or(error!(WhiplashError::MathOverflow))?;
        }
        
        // 2. Transfer tokens from vault to user
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
        borrowed_amount,
        output_amount: expected_output,
        user_received: user_output,
        timestamp: Clock::get()?.unix_timestamp,
    });
    
    // Position account is automatically closed due to the close = user constraint
    
    // Close the position token account if it's a short position
    // For long positions, token::transfer already emptied the account
    // For short positions, we need to manually close the account
    if !position.is_long {
        // Create seeds for position PDA to act as authority
        let nonce_bytes = position_nonce.to_le_bytes();
        let position_seeds = &[
            b"position".as_ref(),
            pool_key.as_ref(),
            user_key.as_ref(),
            nonce_bytes.as_ref(),
            &[bump],
        ];
        let position_signer = &[&position_seeds[..]];
        
        // Close the token account and send rent to user
        token::close_account(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                token::CloseAccount {
                    account: ctx.accounts.position_token_account.to_account_info(),
                    destination: ctx.accounts.user.to_account_info(),
                    authority: ctx.accounts.position.to_account_info(),
                },
                position_signer,
            ),
        )?;
    }
    
    Ok(())
} 