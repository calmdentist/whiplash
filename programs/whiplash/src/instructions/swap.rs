use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};
use crate::{state::*, events::*, WhiplashError};

#[derive(Accounts)]
pub struct Swap<'info> {
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
    
    /// CHECK: This can be either an SPL token account OR a native SOL account (user wallet)
    #[account(mut)]
    pub user_token_in: UncheckedAccount<'info>,
    
    #[account(
        mut,
        constraint = user_token_out.owner == user.key() @ WhiplashError::InvalidTokenAccounts,
    )]
    pub user_token_out: Account<'info, TokenAccount>,
    
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

pub fn handle_swap(ctx: Context<Swap>, amount_in: u64, min_amount_out: u64) -> Result<()> {
    // Validate input amount
    if amount_in == 0 {
        return Err(error!(WhiplashError::ZeroSwapAmount));
    }
    
    // Check if token in is SOL based on the owner of the account
    // If the owner is the System Program, it's a native SOL account
    let is_sol_to_y = ctx.accounts.user_token_in.owner == &anchor_lang::solana_program::system_program::ID;
    
    // For SOL to token Y, we need to verify user_token_out is a token Y account
    if is_sol_to_y {
        require!(
            ctx.accounts.user_token_out.mint == ctx.accounts.pool.token_y_mint,
            WhiplashError::InvalidTokenAccounts
        );
    } else {
        // For token Y to SOL, we need to verify user_token_in is a token account that holds token Y
        // We need to get the mint from the token account
        let user_token_in_account = Account::<TokenAccount>::try_from(&ctx.accounts.user_token_in)?;
        require!(
            user_token_in_account.mint == ctx.accounts.pool.token_y_mint,
            WhiplashError::InvalidTokenAccounts
        );
    }
    
    // Calculate the output amount based on the constant product formula
    let amount_out = if is_sol_to_y {
        ctx.accounts.pool.calculate_swap_x_to_y(amount_in)?
    } else {
        ctx.accounts.pool.calculate_swap_y_to_x(amount_in)?
    };
    
    // Check minimum output amount
    require!(
        amount_out >= min_amount_out,
        WhiplashError::SlippageToleranceExceeded
    );
    
    // Handle token transfers
    if is_sol_to_y {
        // Transfer SOL from user to pool
        let ix = anchor_lang::solana_program::system_instruction::transfer(
            &ctx.accounts.user.key(),
            &ctx.accounts.pool.key(),
            amount_in,
        );
        anchor_lang::solana_program::program::invoke(
            &ix,
            &[
                ctx.accounts.user.to_account_info(),
                ctx.accounts.pool.to_account_info(),
            ],
        )?;

        // Transfer token Y from vault to user
        let pool_signer_seeds = &[
            b"pool".as_ref(),
            ctx.accounts.pool.token_y_mint.as_ref(),
            &[ctx.accounts.pool.bump],
        ];
        let pool_signer = &[&pool_signer_seeds[..]];
        
        let cpi_accounts_out = Transfer {
            from: ctx.accounts.token_y_vault.to_account_info(),
            to: ctx.accounts.user_token_out.to_account_info(),
            authority: ctx.accounts.pool.to_account_info(),
        };
        let cpi_ctx_out = CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            cpi_accounts_out,
            pool_signer,
        );
        token::transfer(cpi_ctx_out, amount_out)?;
    } else {
        // This is a token Y to SOL swap
        // Transfer token Y from user to vault
        let cpi_accounts_in = Transfer {
            from: ctx.accounts.user_token_in.to_account_info(),
            to: ctx.accounts.token_y_vault.to_account_info(),
            authority: ctx.accounts.user.to_account_info(),
        };
        let cpi_ctx_in = CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            cpi_accounts_in,
        );
        token::transfer(cpi_ctx_in, amount_in)?;

        // Transfer SOL from pool to user
        let pool_signer_seeds = &[
            b"pool".as_ref(),
            ctx.accounts.pool.token_y_mint.as_ref(),
            &[ctx.accounts.pool.bump],
        ];
        let pool_signer = &[&pool_signer_seeds[..]];
        
        let ix = anchor_lang::solana_program::system_instruction::transfer(
            &ctx.accounts.pool.key(),
            &ctx.accounts.user.key(),
            amount_out,
        );
        anchor_lang::solana_program::program::invoke_signed(
            &ix,
            &[
                ctx.accounts.pool.to_account_info(),
                ctx.accounts.user.to_account_info(),
            ],
            pool_signer,
        )?;
    }
    
    // Update pool reserves
    let pool = &mut ctx.accounts.pool;
    if is_sol_to_y {
        pool.lamports = pool.lamports.checked_add(amount_in)
            .ok_or(error!(WhiplashError::MathOverflow))?;
        pool.token_y_amount = pool.token_y_amount.checked_sub(amount_out)
            .ok_or(error!(WhiplashError::MathOverflow))?;
    } else {
        pool.token_y_amount = pool.token_y_amount.checked_add(amount_in)
            .ok_or(error!(WhiplashError::MathOverflow))?;
        pool.lamports = pool.lamports.checked_sub(amount_out)
            .ok_or(error!(WhiplashError::MathOverflow))?;
    }
    
    // Emit swap event
    emit!(Swapped {
        user: ctx.accounts.user.key(),
        pool: ctx.accounts.pool.key(),
        token_in_mint: if is_sol_to_y { 
            anchor_lang::solana_program::system_program::ID // Use System Program ID for SOL
        } else {
            let user_token_in_account = Account::<TokenAccount>::try_from(&ctx.accounts.user_token_in)?;
            user_token_in_account.mint
        },
        token_out_mint: ctx.accounts.user_token_out.mint,
        amount_in,
        amount_out,
        timestamp: Clock::get()?.unix_timestamp,
    });
    
    Ok(())
} 