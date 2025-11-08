use anchor_lang::prelude::*;
use anchor_spl::{
    token::{self, Token, TokenAccount, Transfer},
};
use crate::{state::*, events::*, FacemeltError};

#[derive(Accounts)]
#[instruction(amount_in: u64, min_amount_out: u64, leverage: u32, nonce: u64)]
pub struct LeverageSwap<'info> {
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
        constraint = token_vault.key() == pool.token_vault @ FacemeltError::InvalidTokenAccounts,
        constraint = token_vault.mint == pool.token_mint @ FacemeltError::InvalidTokenAccounts,
        constraint = token_vault.owner == pool.key() @ FacemeltError::InvalidTokenAccounts,
    )]
    pub token_vault: Account<'info, TokenAccount>,
    
    /// CHECK: This can be either an SPL token account OR a native SOL account (user wallet)
    #[account(mut)]
    pub user_token_in: UncheckedAccount<'info>,

    #[account(
        init_if_needed,
        payer = user,
        space = 8 + Position::INIT_SPACE,
        seeds = [
            b"position".as_ref(),
            pool.key().as_ref(),
            user.key().as_ref(),
            nonce.to_le_bytes().as_ref(),
        ],
        bump,
    )]
    pub position: Account<'info, Position>,
    
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

pub fn handle_leverage_swap(
    ctx: Context<LeverageSwap>, 
    amount_in: u64, 
    min_amount_out: u64, 
    leverage: u32,
    nonce: u64,
) -> Result<()> {
    // Update funding rate accumulators before any position operations
    let current_timestamp = Clock::get()?.unix_timestamp;
    ctx.accounts.pool.update_funding_accumulators(current_timestamp)?;
    
    // Validate input amount
    if amount_in == 0 {
        return Err(error!(FacemeltError::ZeroSwapAmount));
    }

    // Validate leverage (max 10x = 100)
    require!(
        leverage >= 10 && leverage <= 100,
        FacemeltError::InvalidLeverage
    );
    
    // Check if token in is SOL based on the owner of the account
    // If the owner is the System Program, it's a native SOL account
    let is_sol_to_y = ctx.accounts.user_token_in.owner == &anchor_lang::solana_program::system_program::ID;
    
    // Validate token accounts for short positions (Token->SOL)
    if !is_sol_to_y {
        // For Token->SOL leverage, validate that user_token_in is a token account
        let user_token_in_account = Account::<TokenAccount>::try_from(&ctx.accounts.user_token_in)?;
        require!(
            user_token_in_account.mint == ctx.accounts.pool.token_mint,
            FacemeltError::InvalidTokenAccounts
        );
        require!(
            user_token_in_account.owner == ctx.accounts.user.key(),
            FacemeltError::InvalidTokenAccounts
        );
    }
    
    // -----------------------------------------------------------------
    // Calculate output amounts & soft-boundary premium
    // -----------------------------------------------------------------
    let total_input = amount_in
        .checked_mul(leverage as u64)
        .ok_or(error!(FacemeltError::MathOverflow))?
        .checked_div(10)
        .ok_or(error!(FacemeltError::MathOverflow))?;

    let amount_out = ctx.accounts.pool.calculate_output(total_input, is_sol_to_y)?;
    // msg!("leveraged_amount_out: {}", leveraged_amount_out);
    
    // -----------------------------------------------------------------
    // Calculate and store Δk (delta_k)
    // -----------------------------------------------------------------
    let pool_before = &ctx.accounts.pool;

    // Effective reserves before the swap
    let x_before: u128 = pool_before.effective_sol_reserve as u128;
    let y_before: u128 = pool_before.effective_token_reserve as u128;

    // Effective reserves after the swap (but before we mutate pool state)
    let (x_after, y_after): (u128, u128) = if is_sol_to_y {
        // Long position: adds SOL (amount_in) and takes tokens (amount_out)
        (
            x_before
                .checked_add(amount_in as u128)
                .ok_or(error!(FacemeltError::MathOverflow))?,
            y_before
                .checked_sub(amount_out as u128)
                .ok_or(error!(FacemeltError::MathUnderflow))?,
        )
    } else {
        // Short position: adds tokens (amount_in) and takes SOL (amount_out)
        (
            x_before
                .checked_sub(amount_out as u128)
                .ok_or(error!(FacemeltError::MathUnderflow))?,
            y_before
                .checked_add(amount_in as u128)
                .ok_or(error!(FacemeltError::MathOverflow))?,
        )
    };

    let k_before = x_before
        .checked_mul(y_before)
        .ok_or(error!(FacemeltError::MathOverflow))?;
    let k_after = x_after
        .checked_mul(y_after)
        .ok_or(error!(FacemeltError::MathOverflow))?;

    let delta_k = k_before
        .checked_sub(k_after)
        .ok_or(error!(FacemeltError::MathUnderflow))?;
    
    // // Validate delta_k is at most 10% of current k
    // let max_delta_k = k_before
    //     .checked_mul(10)
    //     .ok_or(error!(FacemeltError::MathOverflow))?
    //     .checked_div(100)
    //     .ok_or(error!(FacemeltError::MathOverflow))?;
    // require!(
    //     delta_k <= max_delta_k,
    //     FacemeltError::DeltaKOverload
    // );
    
    // Check minimum output amount
    require!(
        amount_out >= min_amount_out,
        FacemeltError::SlippageToleranceExceeded
    );
    
    // Handle collateral transfer from user to pool
    if is_sol_to_y {
        // Long position: Transfer SOL collateral from user to pool
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
    } else {
        // Short position: Transfer token collateral from user to vault
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
    }
    
    // Note: We do NOT transfer output tokens to a position account
    // The position is virtual - tokens stay in the pool
    // position.size tracks the virtual claim on pool tokens
    
    // Initialize position data
    let position = &mut ctx.accounts.position;
    position.authority = ctx.accounts.user.key();
    position.pool = ctx.accounts.pool.key();
    position.is_long = is_sol_to_y; // long if SOL to token, short if token to SOL
    position.collateral = amount_in;
    position.leverage = leverage;
    position.size = amount_out; // Virtual claim on pool tokens
    position.delta_k = delta_k;
    position.nonce = nonce;
    position.bump = *ctx.bumps.get("position").unwrap();
    
    // Store the current cumulative funding accumulator for this position
    position.entry_funding_accumulator = ctx.accounts.pool.cumulative_funding_accumulator;
    
    // Update pool reserves accounting
    // Leverage swaps only update effective reserves (not real reserves)
    let pool = &mut ctx.accounts.pool;
    if is_sol_to_y {
        // Long position: adds collateral and takes virtual tokens
        // Update real reserves (collateral is deposited)
        pool.sol_reserve = pool.sol_reserve.checked_add(amount_in)
            .ok_or(error!(FacemeltError::MathOverflow))?;
        
        // Update effective reserves
        pool.effective_sol_reserve = pool.effective_sol_reserve.checked_add(amount_in)
            .ok_or(error!(FacemeltError::MathOverflow))?;
        pool.effective_token_reserve = pool.effective_token_reserve.checked_sub(amount_out)
            .ok_or(error!(FacemeltError::MathUnderflow))?;
        
        // Add to longs delta_k pool
        pool.total_delta_k_longs = pool.total_delta_k_longs
            .checked_add(delta_k)
            .ok_or(error!(FacemeltError::MathOverflow))?;
    } else {
        // Short position: adds token collateral and takes virtual SOL
        // Update real reserves (collateral is deposited)
        pool.token_reserve = pool.token_reserve.checked_add(amount_in)
            .ok_or(error!(FacemeltError::MathOverflow))?;
        
        // Update effective reserves
        pool.effective_token_reserve = pool.effective_token_reserve.checked_add(amount_in)
            .ok_or(error!(FacemeltError::MathOverflow))?;
        pool.effective_sol_reserve = pool.effective_sol_reserve.checked_sub(amount_out)
            .ok_or(error!(FacemeltError::MathUnderflow))?;
        
        // Add to shorts delta_k pool
        pool.total_delta_k_shorts = pool.total_delta_k_shorts
            .checked_add(delta_k)
            .ok_or(error!(FacemeltError::MathOverflow))?;
    }
    
    // Emit swap event
    emit!(Swapped {
        user: ctx.accounts.user.key(),
        pool: ctx.accounts.pool.key(),
        token_in_mint: if is_sol_to_y {
            anchor_lang::solana_program::system_program::ID // Use System Program ID for SOL
        } else {
            ctx.accounts.pool.token_mint
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