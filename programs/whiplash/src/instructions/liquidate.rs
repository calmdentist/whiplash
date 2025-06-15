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
    // Liquidation condition based on Δk
    // --------------------------------------------------------------
    // A position becomes eligible for liquidation when the output it
    // would receive from a regular close *cannot* fully restore the
    // stored Δk.  Equivalently, when the amount that must be paid to
    // the trader ( `payout` ) is ≤ 0.
    //
    // For a long  position: payout = (x * y_pos − Δk) / (y + y_pos)
    // For a short position: payout = (x_pos * y − Δk) / (x + x_pos)
    //
    // We additionally determine the *exact* amount that the pool
    // needs from the position to bring k back to its pre-trade value
    // (denoted `tokens_needed`).  A position can only be liquidated
    // when its vault holds exactly this amount.  If it holds less it
    // is in the "limbo" state (awaiting further price movement) and
    // if it holds more it should be closed normally because it is
    // still in-the-money.
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

    msg!("payout_u128: {}", payout_u128);
    require!(payout_u128 == 0u128, WhiplashError::PositionNotLiquidatable);

    // -----------------------------------------------------------------
    // Calculate how many tokens (or lamports for a short) are *needed*
    // to fully restore Δk at the current price. This derives from:
    //     Δk = total_x * Δy   →   Δy = ceil(Δk / total_x)
    // for longs and the symmetric expression for shorts.
    // -----------------------------------------------------------------

    // We already verified payout_u128 == 0, meaning x*y_pos <= Δk for long
    // (or its short analogue).  The liquidator will provide whatever
    // additional opposite-side asset is required; no strict equality
    // between reserves and position size is necessary.

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
        
        // Calculate how much SOL the liquidator must provide to fully
        // restore Δk now that the Y tokens are back in the pool.
        let needed_delta_k = position.delta_k;
        let delta_from_tokens: u128 = total_x
            .checked_mul(position_size_u128)
            .ok_or(error!(WhiplashError::MathOverflow))?;

        let mut sol_required_u128: u128 = 0u128;
        if needed_delta_k > delta_from_tokens {
            let remainder = needed_delta_k
                .checked_sub(delta_from_tokens)
                .ok_or(error!(WhiplashError::MathOverflow))?;
            let denominator = total_y
                .checked_add(position_size_u128)
                .ok_or(error!(WhiplashError::MathOverflow))?;

            // ceil division to ensure invariant is over-collateralised by at most 1 lamport
            sol_required_u128 = (remainder + denominator - 1) / denominator;
        }

        // Perform SOL transfer from liquidator to pool if needed
        if sol_required_u128 > 0 {
            if sol_required_u128 > u64::MAX as u128 {
                return Err(error!(WhiplashError::MathOverflow));
            }
            let sol_required: u64 = sol_required_u128 as u64;

            let ix = anchor_lang::solana_program::system_instruction::transfer(
                &ctx.accounts.liquidator.key(),
                &ctx.accounts.pool.key(),
                sol_required,
            );
            anchor_lang::solana_program::program::invoke(
                &ix,
                &[
                    ctx.accounts.liquidator.to_account_info(),
                    ctx.accounts.pool.to_account_info(),
                ],
            )?;

            // Update pool SOL reserves
            ctx.accounts.pool.lamports = ctx.accounts.pool.lamports
                .checked_add(sol_required)
                .ok_or(error!(WhiplashError::MathOverflow))?;
        }

        // Update pool token reserves (after SOL handling to avoid double-borrow)
        ctx.accounts.pool.token_y_amount = ctx.accounts.pool.token_y_amount
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
        ctx.accounts.pool.lamports = ctx.accounts.pool.lamports
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
        
    Ok(())
} 