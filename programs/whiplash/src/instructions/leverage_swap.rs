use anchor_lang::prelude::*;
use anchor_spl::{
    token::{self, Token, TokenAccount, Transfer, Mint},
    associated_token::AssociatedToken,
};
use crate::{state::*, events::*, WhiplashError};

#[derive(Accounts)]
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
    leverage: u8,
) -> Result<()> {
    // Validate input amount
    if amount_in == 0 {
        return Err(error!(WhiplashError::ZeroSwapAmount));
    }
    
    // Check if token in is SOL based on the owner of the account
    // If the owner is the System Program, it's a native SOL account
    let is_sol_to_y = ctx.accounts.user_token_in.owner == &anchor_lang::solana_program::system_program::ID;
    
    // Validate token accounts
    if is_sol_to_y {
        // For SOL->Token leverage, validate that position_token_mint is token Y
        require!(
            ctx.accounts.position_token_mint.key() == ctx.accounts.pool.token_y_mint,
            WhiplashError::InvalidTokenAccounts
        );
    } else {
        // For Token->SOL leverage, validate that user_token_in is a token Y account
        let user_token_in_account = Account::<TokenAccount>::try_from(&ctx.accounts.user_token_in)?;
        require!(
            user_token_in_account.mint == ctx.accounts.pool.token_y_mint,
            WhiplashError::InvalidTokenAccounts
        );
        require!(
            user_token_in_account.owner == ctx.accounts.user.key(),
            WhiplashError::InvalidTokenAccounts
        );
        // For a Token->SOL leverage, verify the user_token_out is the user's wallet
        require!(
            ctx.accounts.user_token_out.key() == ctx.accounts.user.key(),
            WhiplashError::InvalidTokenAccounts
        );
    }
    
    // Calculate the output amount based on the constant product formula
    let amount_out = if is_sol_to_y {
        ctx.accounts.pool.calculate_swap_x_to_y(amount_in * leverage as u64 / 10)?
    } else {
        ctx.accounts.pool.calculate_swap_y_to_x(amount_in * leverage as u64 / 10)?
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

        // Transfer token Y from vault to position token account
        let pool_signer_seeds = &[
            b"pool".as_ref(),
            ctx.accounts.pool.token_y_mint.as_ref(),
            &[ctx.accounts.pool.bump],
        ];
        let pool_signer = &[&pool_signer_seeds[..]];
        
        let cpi_accounts_out = Transfer {
            from: ctx.accounts.token_y_vault.to_account_info(),
            to: ctx.accounts.position_token_account.to_account_info(),
            authority: ctx.accounts.pool.to_account_info(),
        };
        let cpi_ctx_out = CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            cpi_accounts_out,
            pool_signer,
        );
        token::transfer(cpi_ctx_out, amount_out)?;
    } else {
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

        // Transfer SOL from pool to position token account
        let pool_signer_seeds = &[
            b"pool".as_ref(),
            ctx.accounts.pool.token_y_mint.as_ref(),
            &[ctx.accounts.pool.bump],
        ];
        let pool_signer = &[&pool_signer_seeds[..]];
        
        // For a short position, we're transferring SOL to the position
        // Make sure the position_token_account is used as the destination
        // This account must be able to receive SOL (not be a token account)
        // Use direct lamport transfer instead of system program transfer
        let pool_lamports = ctx.accounts.pool.to_account_info().lamports();
        let position_lamports = ctx.accounts.position_token_account.to_account_info().lamports();
        
        // Calculate new lamport values
        let new_pool_lamports = pool_lamports.checked_sub(amount_out)
            .ok_or(error!(WhiplashError::MathOverflow))?;
        let new_position_lamports = position_lamports.checked_add(amount_out)
            .ok_or(error!(WhiplashError::MathOverflow))?;
        
        // Update lamports
        **ctx.accounts.pool.to_account_info().try_borrow_mut_lamports()? = new_pool_lamports;
        **ctx.accounts.position_token_account.to_account_info().try_borrow_mut_lamports()? = new_position_lamports;
    }
    
    // Initialize position data
    let position = &mut ctx.accounts.position;
    position.authority = ctx.accounts.user.key();
    position.pool = ctx.accounts.pool.key();
    position.position_vault = ctx.accounts.position_token_account.key();
    position.is_long = is_sol_to_y; // long if SOL to Y, short if Y to SOL
    position.collateral = amount_in;
    position.leverage = leverage;
    position.size = amount_out;
    
    // Calculate entry price (simple estimation as average price) as Q64.64 u128
    let entry_price = ((amount_in as u128 * leverage as u128) << 64) / ((amount_out as u128) << 64);
    position.entry_price = entry_price;
    
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
            ctx.accounts.pool.token_y_mint
        },
        token_out_mint: if is_sol_to_y {
            ctx.accounts.pool.token_y_mint
        } else {
            anchor_lang::solana_program::system_program::ID // Use System Program ID for SOL
        },
        amount_in,
        amount_out,
        timestamp: Clock::get()?.unix_timestamp,
    });
    
    Ok(())
} 