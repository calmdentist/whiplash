use anchor_lang::prelude::*;
use anchor_spl::{
    token::{self, Token, TokenAccount, Transfer, Mint},
    associated_token::AssociatedToken,
};
use crate::{state::*, events::*, WhiplashError};

#[derive(Accounts)]
#[instruction(amount_in: u64, min_amount_out: u64, leverage: u32, nonce: u64)]
pub struct LeverageSwap<'info> {
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
    
    /// CHECK: This can be either an SPL token account OR a native SOL account (user wallet)
    #[account(mut)]
    pub user_token_out: UncheckedAccount<'info>,

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
    
    #[account(
        init_if_needed,
        payer = user,
        associated_token::mint = position_token_mint,
        associated_token::authority = position,
    )]
    pub position_token_account: Account<'info, TokenAccount>,
    
    pub position_token_mint: Account<'info, Mint>,
    
    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
}

pub fn handle_leverage_swap(
    ctx: Context<LeverageSwap>, 
    amount_in: u64, 
    min_amount_out: u64, 
    leverage: u32,
    nonce: u64,
) -> Result<()> {
    // Validate input amount
    if amount_in == 0 {
        return Err(error!(WhiplashError::ZeroSwapAmount));
    }

    // Validate leverage (expressed as multiplier scaled by 10, e.g. 25 = 2.5x)
    require!(
        leverage >= 10 && leverage <= 100, // 1x to 10x leverage
        WhiplashError::InvalidLeverage
    );
    
    // -----------------------------------------------------------------
    // Determine side (long/short) and perform the SPOT leg
    // -----------------------------------------------------------------
    // Native SOL accounts are owned by system program.  If the `in` account is
    // owned by `SystemProgram`, the user is depositing SOL and therefore
    // opening a LONG.  Otherwise the user is depositing token-Y and opening a
    // SHORT.
    let is_long = ctx.accounts.user_token_in.owner == &anchor_lang::solana_program::system_program::ID;
    
    // For convenience grab mutable references we will update later.
    let pool = &mut ctx.accounts.pool;
    let token_program = ctx.accounts.token_program.to_account_info();

    // ---------- Spot quote ----------
    let spot_out: u64 = if is_long {
        pool.calculate_swap_x_to_y(amount_in)?
    } else {
        pool.calculate_swap_y_to_x(amount_in)?
    };

    // Total exposure after borrowing (leverage is scaled by 10)
    let total_out_u128 = (spot_out as u128)
        .checked_mul(leverage as u128)
        .ok_or(error!(WhiplashError::MathOverflow))?
        .checked_div(10)
        .ok_or(error!(WhiplashError::MathOverflow))?;
    
    if total_out_u128 > u64::MAX as u128 {
        return Err(error!(WhiplashError::MathOverflow));
    }
    let total_out = total_out_u128 as u64;

    let borrow_out: u64 = total_out.checked_sub(spot_out)
        .ok_or(error!(WhiplashError::MathUnderflow))?;

    // slippage check against user requirement (applies to total received)
    require!(total_out >= min_amount_out, WhiplashError::SlippageToleranceExceeded);

    // -----------------------------------------------------------------
    // Token movements – SPOT leg
    // -----------------------------------------------------------------
    if is_long {
        // User sends SOL -> pool, receives token-Y
        // 1) transfer SOL collateral to pool PDA
        let ix = anchor_lang::solana_program::system_instruction::transfer(
            &ctx.accounts.user.key(),
            &pool.key(),
            amount_in,
        );
        anchor_lang::solana_program::program::invoke(
            &ix,
            &[ctx.accounts.user.to_account_info(), pool.to_account_info()],
        )?;

        // Check for sufficient liquidity BEFORE transfer
        require!(
            pool.token_y_amount >= total_out,
            WhiplashError::InsufficientLiquidity
        );

        // 2) transfer `total_out` tokens from vault to position token account
        let pool_signer_seeds = &[b"pool".as_ref(), pool.token_y_mint.as_ref(), &[pool.bump]];
        let pool_signer = &[&pool_signer_seeds[..]];

        token::transfer(
            CpiContext::new_with_signer(
                token_program.clone(),
                Transfer {
                    from: ctx.accounts.token_y_vault.to_account_info(),
                    to: ctx.accounts.position_token_account.to_account_info(),
                    authority: pool.to_account_info(),
                },
                pool_signer,
            ),
            total_out,
        )?;

        // Real-reserve bookkeeping
        pool.lamports = pool.lamports.checked_add(amount_in)
            .ok_or(error!(WhiplashError::MathOverflow))?;
        pool.token_y_amount = pool.token_y_amount.checked_sub(total_out)
            .ok_or(error!(WhiplashError::MathUnderflow))?;

        // Virtual reserve update (mirror the borrowed share)
        pool.virtual_token_y_amount = pool.virtual_token_y_amount.checked_add(borrow_out)
            .ok_or(error!(WhiplashError::MathOverflow))?;
    } else {
        // SHORT: user deposits token-Y, receives SOL
        // 1) Transfer token-Y collateral into the vault
        token::transfer(
            CpiContext::new(
                token_program.clone(),
                Transfer {
                    from: ctx.accounts.user_token_in.to_account_info(),
                    to: ctx.accounts.token_y_vault.to_account_info(),
                    authority: ctx.accounts.user.to_account_info(),
                },
            ),
            amount_in,
        )?;

        // 2) Transfer `total_out` lamports (SOL) from pool to position token account
        let pool_lamports = pool.to_account_info().lamports();
        require!(pool_lamports >= total_out, WhiplashError::InsufficientLiquidity);
        
        // The `position` PDA will hold the SOL proceeds for a short.
        let dest_info = ctx.accounts.position.to_account_info();
        **pool.to_account_info().try_borrow_mut_lamports()? = pool_lamports.checked_sub(total_out)
            .ok_or(error!(WhiplashError::MathUnderflow))?;
        **dest_info.try_borrow_mut_lamports()? = dest_info.lamports().checked_add(total_out)
            .ok_or(error!(WhiplashError::MathOverflow))?;

        // Real-reserve bookkeeping
        pool.token_y_amount = pool.token_y_amount.checked_add(amount_in)
            .ok_or(error!(WhiplashError::MathOverflow))?;
        pool.lamports = pool.lamports.checked_sub(total_out)
            .ok_or(error!(WhiplashError::MathUnderflow))?;

        // Virtual reserve update for borrowed SOL
        pool.virtual_sol_amount = pool.virtual_sol_amount.checked_add(borrow_out)
            .ok_or(error!(WhiplashError::MathOverflow))?;
    }

    // -----------------------------------------------------------------
    // Record position state
    // -----------------------------------------------------------------
    let position = &mut ctx.accounts.position;
    position.authority = ctx.accounts.user.key();
    position.pool = pool.key();
    position.position_vault = ctx.accounts.position_token_account.key();
    position.is_long = is_long;
    position.collateral = amount_in;
    position.spot_size = spot_out;
    position.debt_size = borrow_out;
    position.nonce = nonce;

    // Emit event (reuse Swapped for now)
    emit!(Swapped {
        user: ctx.accounts.user.key(),
        pool: pool.key(),
        token_in_mint: if is_long {
            anchor_lang::solana_program::system_program::ID
        } else {
            pool.token_y_mint
        },
        token_out_mint: if is_long {
            pool.token_y_mint
        } else {
            anchor_lang::solana_program::system_program::ID
        },
        amount_in,
        amount_out: total_out,
        timestamp: Clock::get()?.unix_timestamp,
    });

    Ok(())
} 