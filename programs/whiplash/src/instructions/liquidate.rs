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
    
    /// CHECK: This can be either an SPL token account OR a native SOL account (liquidator wallet)
    #[account(mut)]
    pub liquidator_reward_account: UncheckedAccount<'info>,
    
    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
}

pub fn handle_liquidate(ctx: Context<Liquidate>) -> Result<()> {
    let position = &ctx.accounts.position;
    let pool = &ctx.accounts.pool;
    
    // -----------------------------------------------------------------
    // Calculate expected payout and check liquidation condition
    // -----------------------------------------------------------------

    let position_size = position.size;

    // Current total reserves (real + virtual)
    let total_x: u128 = pool.lamports
        .checked_add(pool.virtual_sol_amount)
        .ok_or(error!(WhiplashError::MathOverflow))? as u128;
    let total_y: u128 = pool.token_y_amount
        .checked_add(pool.virtual_token_y_amount)
        .ok_or(error!(WhiplashError::MathOverflow))? as u128;

    let position_size_u128: u128 = position_size as u128;
    let delta_k: u128 = position.delta_k;

    // Calculate expected payout and liquidation threshold
    let (expected_payout, liquidation_threshold) = if position.is_long {
        // Long: user returns Y tokens and gets SOL
        // X_out = (x * y_pos - delta_k) / (y + y_pos)
        let product_val = total_x
            .checked_mul(position_size_u128)
            .ok_or(error!(WhiplashError::MathOverflow))?;

        let expected_payout = if product_val <= delta_k {
            0u128
        } else {
            let numerator = product_val
                .checked_sub(delta_k)
                .ok_or(error!(WhiplashError::MathOverflow))?;
            let denominator = total_y
                .checked_add(position_size_u128)
                .ok_or(error!(WhiplashError::MathOverflow))?;
            numerator
                .checked_div(denominator)
                .ok_or(error!(WhiplashError::MathOverflow))?
        };

        // Liquidation threshold: (delta_k / x_current) * 1.05
        let threshold = delta_k
            .checked_mul(105)
            .ok_or(error!(WhiplashError::MathOverflow))?
            .checked_div(100)
            .ok_or(error!(WhiplashError::MathOverflow))?
            .checked_div(total_x)
            .ok_or(error!(WhiplashError::MathOverflow))?;

        (expected_payout, threshold)
    } else {
        // Short: user returns SOL and gets Y tokens
        // Y_out = (x_pos * y - delta_k) / (x + x_pos)
        let product_val = position_size_u128
            .checked_mul(total_y)
            .ok_or(error!(WhiplashError::MathOverflow))?;

        let expected_payout = if product_val <= delta_k {
            0u128
        } else {
            let numerator = product_val
                .checked_sub(delta_k)
                .ok_or(error!(WhiplashError::MathOverflow))?;
            let denominator = total_x
                .checked_add(position_size_u128)
                .ok_or(error!(WhiplashError::MathOverflow))?;
            numerator
                .checked_div(denominator)
                .ok_or(error!(WhiplashError::MathOverflow))?
        };

        // Liquidation threshold: (delta_k / y_current) * 1.05
        let threshold = delta_k
            .checked_mul(105)
            .ok_or(error!(WhiplashError::MathOverflow))?
            .checked_div(100)
            .ok_or(error!(WhiplashError::MathOverflow))?
            .checked_div(total_y)
            .ok_or(error!(WhiplashError::MathOverflow))?;

        (expected_payout, threshold)
    };

    // Check if position is liquidatable: expected_payout <= threshold
    // comment out require block for testing
    require!(
        expected_payout <= liquidation_threshold,
        WhiplashError::PositionNotLiquidatable
    );

    // -----------------------------------------------------------------
    // Execute liquidation
    // -----------------------------------------------------------------

    // Calculate exact amount needed to restore invariant
    let restore_amount = if position.is_long {
        // For longs: delta_k / x_current
        delta_k
            .checked_div(total_x)
            .ok_or(error!(WhiplashError::MathOverflow))?
    } else {
        // For shorts: delta_k / y_current  
        delta_k
            .checked_div(total_y)
            .ok_or(error!(WhiplashError::MathOverflow))?
    };

    // Ensure restore amount fits in u64
    if restore_amount > u64::MAX as u128 {
        return Err(error!(WhiplashError::MathOverflow));
    }
    let restore_amount_u64 = restore_amount as u64;

    // Calculate liquidator reward
    let liquidator_reward = position_size
        .checked_sub(restore_amount_u64)
        .ok_or(error!(WhiplashError::MathUnderflow))?;
    
    // Get the position bump from context
    let bump = *ctx.bumps.get("position").unwrap();
    let pool_key = ctx.accounts.pool.key();
    let position_owner_key = ctx.accounts.position_owner.key();
    let position_nonce = position.nonce;

    // Handle based on position type
    if position.is_long {
        // LONG POSITION LIQUIDATION
        
        // 1. Transfer tokens from position to vault (restore amount)
        let nonce_bytes = position_nonce.to_le_bytes();
        let position_seeds = &[
            b"position".as_ref(),
            pool_key.as_ref(),
            position_owner_key.as_ref(),
            nonce_bytes.as_ref(),
            &[bump],
        ];
        let position_signer = &[&position_seeds[..]];
        
        // Transfer restore amount to vault
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
            restore_amount_u64,
        )?;

        // 2. Transfer liquidator reward to liquidator
        if liquidator_reward > 0 {
            token::transfer(
                CpiContext::new_with_signer(
                    ctx.accounts.token_program.to_account_info(),
                    Transfer {
                        from: ctx.accounts.position_token_account.to_account_info(),
                        to: ctx.accounts.liquidator_reward_account.to_account_info(),
                        authority: ctx.accounts.position.to_account_info(),
                    },
                    position_signer,
                ),
                liquidator_reward,
            )?;
        }
        
        // 3. Update pool state
        {
            let pool = &mut ctx.accounts.pool;
            pool.token_y_amount = pool.token_y_amount
                .checked_add(restore_amount_u64)
                .ok_or(error!(WhiplashError::MathOverflow))?;
            
            pool.leveraged_token_y_amount = pool.leveraged_token_y_amount
                .checked_sub(position.leveraged_token_amount)
                .ok_or(error!(WhiplashError::MathUnderflow))?;
        }
    } else {
        // SHORT POSITION LIQUIDATION
        
        // For short positions, the position holds SOL. We need to:
        // 1. Send restore amount to pool
        // 2. Send reward to liquidator
        // 3. Update pool accounting

        let position_account_lamports = ctx.accounts.position_token_account.to_account_info().lamports();
        
        // Ensure position has enough lamports
        require!(
            position_account_lamports >= position_size,
            WhiplashError::InsufficientFunds
        );

        // Transfer restore amount to pool
        let pool_starting_lamports = ctx.accounts.pool.to_account_info().lamports();
        let position_starting_lamports = ctx.accounts.position_token_account.to_account_info().lamports();
        
        **ctx.accounts.pool.to_account_info().try_borrow_mut_lamports()? = pool_starting_lamports
            .checked_add(restore_amount_u64)
            .ok_or(error!(WhiplashError::MathOverflow))?;
            
        **ctx.accounts.position_token_account.to_account_info().try_borrow_mut_lamports()? = position_starting_lamports
            .checked_sub(restore_amount_u64)
            .ok_or(error!(WhiplashError::MathUnderflow))?;

        // Transfer liquidator reward to liquidator if any
        if liquidator_reward > 0 {
            let liquidator_starting_lamports = ctx.accounts.liquidator_reward_account.to_account_info().lamports();
            let position_current_lamports = ctx.accounts.position_token_account.to_account_info().lamports();
            
            **ctx.accounts.liquidator_reward_account.to_account_info().try_borrow_mut_lamports()? = liquidator_starting_lamports
                .checked_add(liquidator_reward)
                .ok_or(error!(WhiplashError::MathOverflow))?;
                
            **ctx.accounts.position_token_account.to_account_info().try_borrow_mut_lamports()? = position_current_lamports
                .checked_sub(liquidator_reward)
                .ok_or(error!(WhiplashError::MathUnderflow))?;
        }
        
        // Update pool state
        {
            let pool = &mut ctx.accounts.pool;
            pool.lamports = pool.lamports
                .checked_add(restore_amount_u64)
                .ok_or(error!(WhiplashError::MathOverflow))?;
            
            pool.leveraged_sol_amount = pool.leveraged_sol_amount
                .checked_sub(position.leveraged_token_amount)
                .ok_or(error!(WhiplashError::MathUnderflow))?;
        }
    }

    // Emit liquidation event
    emit!(PositionLiquidated {
        liquidator: ctx.accounts.liquidator.key(),
        position_owner: ctx.accounts.position_owner.key(),
        pool: ctx.accounts.pool.key(),
        position: ctx.accounts.position.key(),
        position_size,
        borrowed_amount: position.leveraged_token_amount,
        expected_output: expected_payout as u64,
        liquidator_reward,
        timestamp: Clock::get()?.unix_timestamp,
    });
    
    // Position account is automatically closed due to the close = liquidator constraint
    
    Ok(())
} 