use anchor_lang::prelude::*;
use anchor_spl::{
    token::{self, Token, TokenAccount, Transfer},
    associated_token::AssociatedToken,
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
    let pool = &mut ctx.accounts.pool;

    // Pointers to common accounts
    let token_program = ctx.accounts.token_program.to_account_info();

    // Convenience values
    let spot_in: u64 = position.spot_size;
    let debt: u64 = position.debt_size;

    let gross_amount_out: u64;

    // ------------------------------------------------------------------
    // LONG  (user holds token-Y, will receive SOL back)
    // ------------------------------------------------------------------
    if position.is_long {
        // ---------- 1) Repay the borrowed tokens ----------
        // Transfer `debt` tokens from the position vault to the pool vault.
        let bump = *ctx.bumps.get("position").unwrap();
        let nonce_bytes = position.nonce.to_le_bytes();
        let pool_key = pool.key();
        let user_key = ctx.accounts.user.key();
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
                token_program.clone(),
                Transfer {
                    from: ctx.accounts.position_token_account.to_account_info(),
                    to: ctx.accounts.token_y_vault.to_account_info(),
                    authority: ctx.accounts.position.to_account_info(),
                },
                position_signer,
            ),
            debt,
        )?;

        // Update pool bookkeeping for debt repayment
        pool.token_y_amount = pool.token_y_amount.checked_add(debt)
            .ok_or(error!(WhiplashError::MathOverflow))?;
        pool.virtual_token_y_amount = pool.virtual_token_y_amount.checked_sub(debt)
            .ok_or(error!(WhiplashError::MathUnderflow))?;

        // ---------- 2) Spot unwind ----------
        // Compute SOL the user should receive for `spot_in` tokens.
        let sol_out = pool.calculate_swap_y_to_x(spot_in)?;
        gross_amount_out = sol_out;

        // Transfer the `spot_in` tokens (spot leg) to the pool vault
        token::transfer(
            CpiContext::new_with_signer(
                token_program.clone(),
                Transfer {
                    from: ctx.accounts.position_token_account.to_account_info(),
                    to: ctx.accounts.token_y_vault.to_account_info(),
                    authority: ctx.accounts.position.to_account_info(),
                },
                position_signer,
            ),
            spot_in,
        )?;

        // Book real reserves
        pool.token_y_amount = pool.token_y_amount.checked_add(spot_in)
            .ok_or(error!(WhiplashError::MathOverflow))?;

        // Transfer SOL to user, then synchronize the internal counter.
        **pool.to_account_info().try_borrow_mut_lamports()? -= sol_out;
        **ctx.accounts.user.to_account_info().try_borrow_mut_lamports()? += sol_out;
        pool.lamports = **pool.to_account_info().try_borrow_lamports()?;

    // ------------------------------------------------------------------
    // SHORT (user holds SOL, will receive token-Y back)
    // ------------------------------------------------------------------
    } else {
        // For a short, the position PDA itself holds the SOL that was borrowed.
        // To close, we must transfer this SOL back to the pool.
        let total_sol_to_repay = position.spot_size.checked_add(position.debt_size)
            .ok_or(error!(WhiplashError::MathOverflow))?;

        // ---------- 1) Repay SOL from position account to pool ----------
        // Directly move lamports since both accounts are owned by this program.
        **ctx.accounts.position.to_account_info().try_borrow_mut_lamports()? -= total_sol_to_repay;
        **pool.to_account_info().try_borrow_mut_lamports()? += total_sol_to_repay;

        // Update pool bookkeeping to match the new account balance
        pool.lamports = **pool.to_account_info().try_borrow_lamports()?;
        
        pool.virtual_sol_amount = pool.virtual_sol_amount.checked_sub(position.debt_size)
            .ok_or(error!(WhiplashError::MathUnderflow))?;

        // ---------- 2) Calculate and send token-Y output ----------
        // The user is effectively swapping their original spot size worth of SOL
        // back into the pool for token-Y at the new price.
        let tokens_out = pool.calculate_swap_x_to_y(position.spot_size)?;
        gross_amount_out = tokens_out;

        // Book real reserve changes for the token-Y output
        pool.token_y_amount = pool.token_y_amount.checked_sub(tokens_out)
            .ok_or(error!(WhiplashError::MathUnderflow))?;

        // Transfer token-Y to user
        let pool_signer_seeds = &[b"pool".as_ref(), pool.token_y_mint.as_ref(), &[pool.bump]];
        let pool_signer = &[&pool_signer_seeds[..]];

        token::transfer(
            CpiContext::new_with_signer(
                token_program,
                Transfer {
                    from: ctx.accounts.token_y_vault.to_account_info(),
                    to: ctx.accounts.user_token_out.to_account_info(),
                    authority: pool.to_account_info(),
                },
                pool_signer,
            ),
            tokens_out,
        )?;
    }

    // Emit event
    emit!(PositionClosed {
        user: ctx.accounts.user.key(),
        pool: pool.key(),
        position: ctx.accounts.position.key(),
        is_long: position.is_long,
        position_size: position.spot_size + position.debt_size,
        borrowed_amount: position.debt_size,
        output_amount: gross_amount_out,
        user_received: gross_amount_out.wrapping_sub(position.collateral), // PnL
        timestamp: Clock::get()?.unix_timestamp,
    });

    Ok(())
} 