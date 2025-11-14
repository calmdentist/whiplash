use anchor_lang::prelude::*;
use anchor_spl::{
    token::{Token, Mint, TokenAccount, Transfer},
    associated_token::AssociatedToken,
};
use crate::{state::*, events::*, FacemeltError};

#[derive(Accounts)]
pub struct SwapOnCurve<'info> {
    #[account(mut)]
    pub user: Signer<'info>,
    
    pub token_mint: Account<'info, Mint>,
    
    #[account(
        mut,
        seeds = [
            b"bonding_curve".as_ref(),
            token_mint.key().as_ref(),
        ],
        bump = bonding_curve.bump,
    )]
    pub bonding_curve: Account<'info, BondingCurve>,
    
    #[account(
        mut,
        seeds = [
            b"pool".as_ref(),
            token_mint.key().as_ref(),
        ],
        bump = pool.bump,
    )]
    pub pool: Account<'info, Pool>,
    
    #[account(
        mut,
        associated_token::mint = token_mint,
        associated_token::authority = pool,
    )]
    pub token_vault: Account<'info, TokenAccount>,
    
    #[account(
        init_if_needed,
        payer = user,
        associated_token::mint = token_mint,
        associated_token::authority = user,
    )]
    pub user_token_account: Account<'info, TokenAccount>,
    
    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
}

pub fn handle_swap_on_curve(
    mut ctx: Context<SwapOnCurve>,
    amount_in: u64,
    min_amount_out: u64,
    input_is_sol: bool,
) -> Result<()> {
    require!(amount_in > 0, FacemeltError::ZeroSwapAmount);
    
    // Extract immutable values we need upfront
    let token_mint_key = ctx.accounts.token_mint.key();
    
    // Check that bonding curve is still active
    require!(
        ctx.accounts.bonding_curve.is_active(),
        FacemeltError::BondingCurveNotActive
    );
    
    let amount_out: u64;
    
    if input_is_sol {
        // Buying tokens with SOL
        
        // Calculate how many tokens can be bought
        let mut tokens_out = ctx.accounts.bonding_curve.calculate_tokens_out_for_sol(amount_in)?;
        let mut sol_spent = amount_in;
        let mut sol_refund = 0u64;
        
        // Check if this would exceed the target
        let new_tokens_sold = ctx.accounts.bonding_curve.tokens_sold_on_curve
            .checked_add(tokens_out)
            .ok_or(error!(FacemeltError::MathOverflow))?;
        
        if new_tokens_sold > ctx.accounts.bonding_curve.bonding_target_tokens_sold {
            // Cap at target and calculate exact SOL needed
            tokens_out = ctx.accounts.bonding_curve.bonding_target_tokens_sold
                .checked_sub(ctx.accounts.bonding_curve.tokens_sold_on_curve)
                .ok_or(error!(FacemeltError::MathUnderflow))?;
            
            // Calculate exact SOL needed for these tokens
            let q1 = ctx.accounts.bonding_curve.tokens_sold_on_curve as u128;
            let q2 = q1
                .checked_add(tokens_out as u128)
                .ok_or(error!(FacemeltError::MathOverflow))?;
            
            let q1_squared = q1
                .checked_mul(q1)
                .ok_or(error!(FacemeltError::MathOverflow))?;
            let q2_squared = q2
                .checked_mul(q2)
                .ok_or(error!(FacemeltError::MathOverflow))?;
            
            let diff = q2_squared
                .checked_sub(q1_squared)
                .ok_or(error!(FacemeltError::MathUnderflow))?;
            
            // sol_needed = (m * diff) / (2 * PRECISION)
            sol_spent = ctx.accounts.bonding_curve.bonding_curve_slope_m
                .checked_mul(diff)
                .ok_or(error!(FacemeltError::MathOverflow))?
                .checked_div(2u128)
                .ok_or(error!(FacemeltError::MathOverflow))?
                .checked_div(BondingCurve::SLOPE_PRECISION)
                .ok_or(error!(FacemeltError::MathOverflow))? as u64;
            
            // Calculate refund
            sol_refund = amount_in
                .checked_sub(sol_spent)
                .ok_or(error!(FacemeltError::MathUnderflow))?;
        }
        
        // Check slippage
        require!(
            tokens_out >= min_amount_out,
            FacemeltError::SlippageToleranceExceeded
        );
        
        // Transfer SOL from user to pool (the pool PDA will hold the SOL)
        let transfer_ix = anchor_lang::solana_program::system_instruction::transfer(
            &ctx.accounts.user.key(),
            &ctx.accounts.pool.key(),
            sol_spent,
        );
        anchor_lang::solana_program::program::invoke(
            &transfer_ix,
            &[
                ctx.accounts.user.to_account_info(),
                ctx.accounts.pool.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
        )?;
        
        // Update bonding curve state
        ctx.accounts.bonding_curve.sol_raised_on_curve = ctx.accounts.bonding_curve.sol_raised_on_curve
            .checked_add(sol_spent)
            .ok_or(error!(FacemeltError::MathOverflow))?;
        
        ctx.accounts.bonding_curve.tokens_sold_on_curve = ctx.accounts.bonding_curve.tokens_sold_on_curve
            .checked_add(tokens_out)
            .ok_or(error!(FacemeltError::MathOverflow))?;
        
        // Transfer tokens from vault to user (vault is owned by pool)
        let pool_bump = ctx.accounts.pool.bump;
        let seeds = &[
            b"pool".as_ref(),
            token_mint_key.as_ref(),
            &[pool_bump],
        ];
        let signer_seeds = &[&seeds[..]];
        
        let cpi_accounts = Transfer {
            from: ctx.accounts.token_vault.to_account_info(),
            to: ctx.accounts.user_token_account.to_account_info(),
            authority: ctx.accounts.pool.to_account_info(),
        };
        let cpi_ctx = CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            cpi_accounts,
            signer_seeds,
        );
        anchor_spl::token::transfer(cpi_ctx, tokens_out)?;
        
        amount_out = tokens_out;
        
        // Check if graduation threshold reached (either SOL target OR token target)
        let should_graduate = ctx.accounts.bonding_curve.sol_raised_on_curve >= ctx.accounts.bonding_curve.bonding_target_sol
            || ctx.accounts.bonding_curve.tokens_sold_on_curve >= ctx.accounts.bonding_curve.bonding_target_tokens_sold;
        
        if should_graduate {
            graduate_to_amm(&mut ctx, token_mint_key, sol_refund)?;
            return Ok(());
        }
        
        // Refund excess SOL if needed (only if not graduating)
        if sol_refund > 0 {
            // Use direct lamport manipulation for data accounts
            let pool_lamports = ctx.accounts.pool.to_account_info().lamports();
            let user_lamports = ctx.accounts.user.to_account_info().lamports();
            
            let new_pool_lamports = pool_lamports.checked_sub(sol_refund)
                .ok_or(error!(FacemeltError::InsufficientFunds))?;
            let new_user_lamports = user_lamports.checked_add(sol_refund)
                .ok_or(error!(FacemeltError::MathOverflow))?;
            
            **ctx.accounts.pool.to_account_info().try_borrow_mut_lamports()? = new_pool_lamports;
            **ctx.accounts.user.to_account_info().try_borrow_mut_lamports()? = new_user_lamports;
        }
        
    } else {
        // Selling tokens for SOL
        
        // Calculate how much SOL will be received
        let sol_out = ctx.accounts.bonding_curve.calculate_sol_out_for_tokens(amount_in)?;
        
        // Check slippage
        require!(
            sol_out >= min_amount_out,
            FacemeltError::SlippageToleranceExceeded
        );
        
        // Transfer tokens from user to vault
        let cpi_accounts = Transfer {
            from: ctx.accounts.user_token_account.to_account_info(),
            to: ctx.accounts.token_vault.to_account_info(),
            authority: ctx.accounts.user.to_account_info(),
        };
        let cpi_ctx = CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            cpi_accounts,
        );
        anchor_spl::token::transfer(cpi_ctx, amount_in)?;
        
        // Update bonding curve state
        ctx.accounts.bonding_curve.tokens_sold_on_curve = ctx.accounts.bonding_curve.tokens_sold_on_curve
            .checked_sub(amount_in)
            .ok_or(error!(FacemeltError::MathUnderflow))?;
        
        ctx.accounts.bonding_curve.sol_raised_on_curve = ctx.accounts.bonding_curve.sol_raised_on_curve
            .checked_sub(sol_out)
            .ok_or(error!(FacemeltError::MathUnderflow))?;
        
        // Transfer SOL from pool to user (using direct lamport manipulation for data accounts)
        let pool_lamports = ctx.accounts.pool.to_account_info().lamports();
        let user_lamports = ctx.accounts.user.to_account_info().lamports();
        
        let new_pool_lamports = pool_lamports.checked_sub(sol_out)
            .ok_or(error!(FacemeltError::InsufficientFunds))?;
        let new_user_lamports = user_lamports.checked_add(sol_out)
            .ok_or(error!(FacemeltError::MathOverflow))?;
        
        // Update lamports
        **ctx.accounts.pool.to_account_info().try_borrow_mut_lamports()? = new_pool_lamports;
        **ctx.accounts.user.to_account_info().try_borrow_mut_lamports()? = new_user_lamports;
        
        amount_out = sol_out;
    }
    
    // Emit swap event
    emit!(BondingCurveSwapped {
        user: ctx.accounts.user.key(),
        bonding_curve: ctx.accounts.bonding_curve.key(),
        input_is_sol,
        amount_in,
        amount_out,
        tokens_sold_on_curve: ctx.accounts.bonding_curve.tokens_sold_on_curve,
        sol_raised_on_curve: ctx.accounts.bonding_curve.sol_raised_on_curve,
        timestamp: Clock::get()?.unix_timestamp,
    });
    
    Ok(())
}

// Internal function to graduate the bonding curve to an AMM
fn graduate_to_amm(ctx: &mut Context<SwapOnCurve>, token_mint_key: Pubkey, sol_refund: u64) -> Result<()> {
    // Mark bonding curve as graduated
    ctx.accounts.bonding_curve.status = BondingCurveStatus::Graduated as u8;
    
    // Calculate LP tokens: half of tokens sold go to LP
    // Remaining tokens in vault = total_supply - tokens_sold
    // LP tokens = target_tokens_sold / 2 (as per architecture)
    let lp_tokens = ctx.accounts.bonding_curve.bonding_target_tokens_sold
        .checked_div(2)
        .ok_or(error!(FacemeltError::MathOverflow))?;
    
    let sol_raised = ctx.accounts.bonding_curve.sol_raised_on_curve;
    
    // Initialize pool reserves
    // The token vault already has the remaining tokens (total_supply - tokens_sold_on_curve)
    // We use lp_tokens for the pool
    ctx.accounts.pool.sol_reserve = sol_raised;
    ctx.accounts.pool.effective_sol_reserve = sol_raised;
    ctx.accounts.pool.token_reserve = lp_tokens;
    ctx.accounts.pool.effective_token_reserve = lp_tokens;
    
    // Initialize timestamp for funding calculations
    ctx.accounts.pool.last_update_timestamp = Clock::get()?.unix_timestamp;
    
    // Initialize EMA with current price
    // Price = sol_reserve / token_reserve (in fixed-point)
    let current_price = (ctx.accounts.pool.effective_sol_reserve as u128)
        .checked_mul(Pool::PRICE_PRECISION)
        .ok_or(error!(FacemeltError::MathOverflow))?
        .checked_div(ctx.accounts.pool.effective_token_reserve as u128)
        .ok_or(error!(FacemeltError::MathOverflow))?;
    
    ctx.accounts.pool.ema_price = current_price;
    ctx.accounts.pool.ema_initialized = true;
    
    // Emit graduation event
    emit!(BondingCurveGraduated {
        bonding_curve: ctx.accounts.bonding_curve.key(),
        pool: ctx.accounts.pool.key(),
        token_mint: token_mint_key,
        sol_raised,
        tokens_for_lp: lp_tokens,
        timestamp: Clock::get()?.unix_timestamp,
    });
    
    // Refund excess SOL to user if needed
    if sol_refund > 0 {
        // Use direct lamport manipulation for data accounts
        let pool_lamports = ctx.accounts.pool.to_account_info().lamports();
        let user_lamports = ctx.accounts.user.to_account_info().lamports();
        
        let new_pool_lamports = pool_lamports.checked_sub(sol_refund)
            .ok_or(error!(FacemeltError::InsufficientFunds))?;
        let new_user_lamports = user_lamports.checked_add(sol_refund)
            .ok_or(error!(FacemeltError::MathOverflow))?;
        
        **ctx.accounts.pool.to_account_info().try_borrow_mut_lamports()? = new_pool_lamports;
        **ctx.accounts.user.to_account_info().try_borrow_mut_lamports()? = new_user_lamports;
    }
    
    Ok(())
}
