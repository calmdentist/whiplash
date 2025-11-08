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
    
    /// CHECK: This can be either an SPL token account OR a native SOL account (user wallet)
    #[account(mut)]
    pub user_token_in: UncheckedAccount<'info>,
    
    /// CHECK: This can be either an SPL token account OR a native SOL account (user wallet)
    #[account(mut)]
    pub user_token_out: UncheckedAccount<'info>,
    
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

pub fn handle_swap(ctx: Context<Swap>, amount_in: u64, min_amount_out: u64) -> Result<()> {
    // Update funding rate accumulators to ensure they're current
    // This ensures spot traders benefit from accrued funding fees
    let current_timestamp = Clock::get()?.unix_timestamp;
    ctx.accounts.pool.update_funding_accumulators(current_timestamp)?;
    
    // Validate input amount
    if amount_in == 0 {
        return Err(error!(WhiplashError::ZeroSwapAmount));
    }
    
    // Check if token in is SOL based on the owner of the account
    // If the owner is the System Program, it's a native SOL account
    let is_sol_to_y = ctx.accounts.user_token_in.owner == &anchor_lang::solana_program::system_program::ID;
    
    if is_sol_to_y {
        // For SOL to token, we need to verify user_token_out is a token account
        let user_token_out_account = Account::<TokenAccount>::try_from(&ctx.accounts.user_token_out)?;
        require!(
            user_token_out_account.mint == ctx.accounts.pool.token_mint,
            WhiplashError::InvalidTokenAccounts
        );
        require!(
            user_token_out_account.owner == ctx.accounts.user.key(),
            WhiplashError::InvalidTokenAccounts
        );
    } else {
        // For token to SOL, we need to verify user_token_in is a token account
        let user_token_in_account = Account::<TokenAccount>::try_from(&ctx.accounts.user_token_in)?;
        require!(
            user_token_in_account.mint == ctx.accounts.pool.token_mint,
            WhiplashError::InvalidTokenAccounts
        );
        require!(
            user_token_in_account.owner == ctx.accounts.user.key(),
            WhiplashError::InvalidTokenAccounts
        );
        // For a token to SOL swap, we need to verify the user_token_out is the user's wallet for receiving SOL
        // For a SOL output, the account must be a system account (wallet)
        require!(
            ctx.accounts.user_token_out.key() == ctx.accounts.user.key(),
            WhiplashError::InvalidTokenAccounts
        );
    }
    
    // --------------------------------------------------
    // Calculate output using effective liquidity
    // --------------------------------------------------
    let amount_out = ctx.accounts.pool.calculate_output(amount_in, is_sol_to_y)?;
    
    // Check minimum output amount
    require!(amount_out >= min_amount_out, WhiplashError::SlippageToleranceExceeded);
    
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

        // Transfer token from vault to user
        let pool_signer_seeds = &[
            b"pool".as_ref(),
            ctx.accounts.pool.token_mint.as_ref(),
            &[ctx.accounts.pool.bump],
        ];
        let pool_signer = &[&pool_signer_seeds[..]];
        
        let cpi_accounts_out = Transfer {
            from: ctx.accounts.token_vault.to_account_info(),
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
        // This is a token to SOL swap
        // Transfer token from user to vault
        let cpi_accounts_in = Transfer {
            from: ctx.accounts.user_token_in.to_account_info(),
            to: ctx.accounts.token_vault.to_account_info(),
            authority: ctx.accounts.user.to_account_info(),
        };
        let cpi_ctx_in = CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            cpi_accounts_in,
        );
        token::transfer(cpi_ctx_in, amount_in)?;

        // Transfer SOL from pool to user
        // The user_token_out MUST be the user wallet account itself when swapping to SOL
        let pool_lamports = ctx.accounts.pool.to_account_info().lamports();
        let user_lamports = ctx.accounts.user.to_account_info().lamports();
        
        // Calculate new lamport values
        let new_pool_lamports = pool_lamports.checked_sub(amount_out)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?;
        let new_user_lamports = user_lamports.checked_add(amount_out)
            .ok_or(error!(crate::WhiplashError::MathOverflow))?;
        
        // Update lamports
        **ctx.accounts.pool.to_account_info().try_borrow_mut_lamports()? = new_pool_lamports;
        **ctx.accounts.user.to_account_info().try_borrow_mut_lamports()? = new_user_lamports;
    }
    
    // Update pool reserves
    // Spot swaps update both real and effective reserves
    let pool = &mut ctx.accounts.pool;
    if is_sol_to_y {
        pool.sol_reserve = pool.sol_reserve.checked_add(amount_in)
            .ok_or(error!(WhiplashError::MathOverflow))?;
        pool.token_reserve = pool.token_reserve.checked_sub(amount_out)
            .ok_or(error!(WhiplashError::MathUnderflow))?;
        pool.effective_sol_reserve = pool.effective_sol_reserve.checked_add(amount_in)
            .ok_or(error!(WhiplashError::MathOverflow))?;
        pool.effective_token_reserve = pool.effective_token_reserve.checked_sub(amount_out)
            .ok_or(error!(WhiplashError::MathUnderflow))?;
    } else {
        pool.token_reserve = pool.token_reserve.checked_add(amount_in)
            .ok_or(error!(WhiplashError::MathOverflow))?;
        pool.sol_reserve = pool.sol_reserve.checked_sub(amount_out)
            .ok_or(error!(WhiplashError::MathUnderflow))?;
        pool.effective_token_reserve = pool.effective_token_reserve.checked_add(amount_in)
            .ok_or(error!(WhiplashError::MathOverflow))?;
        pool.effective_sol_reserve = pool.effective_sol_reserve.checked_sub(amount_out)
            .ok_or(error!(WhiplashError::MathUnderflow))?;
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
        token_out_mint: if is_sol_to_y {
            ctx.accounts.pool.token_mint
        } else {
            anchor_lang::solana_program::system_program::ID // Use System Program ID for SOL
        },
        amount_in,
        amount_out,
        timestamp: Clock::get()?.unix_timestamp,
    });
    
    Ok(())
}