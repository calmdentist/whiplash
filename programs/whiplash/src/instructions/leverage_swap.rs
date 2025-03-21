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
            pool.token_x_mint.as_ref(),
            pool.token_y_mint.as_ref(),
        ],
        bump = pool.bump,
    )]
    pub pool: Account<'info, Pool>,
    
    #[account(
        mut,
        constraint = token_x_vault.key() == pool.token_x_vault @ WhiplashError::InvalidTokenAccounts,
        constraint = token_x_vault.mint == pool.token_x_mint @ WhiplashError::InvalidTokenAccounts,
        constraint = token_x_vault.owner == pool.key() @ WhiplashError::InvalidTokenAccounts,
    )]
    pub token_x_vault: Account<'info, TokenAccount>,
    
    #[account(
        mut,
        constraint = token_y_vault.key() == pool.token_y_vault @ WhiplashError::InvalidTokenAccounts,
        constraint = token_y_vault.mint == pool.token_y_mint @ WhiplashError::InvalidTokenAccounts,
        constraint = token_y_vault.owner == pool.key() @ WhiplashError::InvalidTokenAccounts,
    )]
    pub token_y_vault: Account<'info, TokenAccount>,
    
    #[account(
        mut,
        constraint = user_token_in.owner == user.key() @ WhiplashError::InvalidTokenAccounts,
    )]
    pub user_token_in: Account<'info, TokenAccount>,
    
    #[account(
        mut,
        constraint = user_token_out.owner == user.key() @ WhiplashError::InvalidTokenAccounts,
    )]
    pub user_token_out: Account<'info, TokenAccount>,

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
    
    // Check if token in is X or Y
    let is_x_to_y = ctx.accounts.user_token_in.mint == ctx.accounts.pool.token_x_mint;
    
    // Validate token accounts
    if is_x_to_y {
        require!(
            ctx.accounts.user_token_out.mint == ctx.accounts.pool.token_y_mint,
            WhiplashError::InvalidTokenAccounts
        );
    } else {
        require!(
            ctx.accounts.user_token_in.mint == ctx.accounts.pool.token_y_mint,
            WhiplashError::InvalidTokenAccounts
        );
        require!(
            ctx.accounts.user_token_out.mint == ctx.accounts.pool.token_x_mint,
            WhiplashError::InvalidTokenAccounts
        );
    }
    
    // Calculate the output amount based on the constant product formula
    let amount_out = if is_x_to_y {
        ctx.accounts.pool.calculate_swap_x_to_y(amount_in * leverage as u64 / 10)?
    } else {
        ctx.accounts.pool.calculate_swap_y_to_x(amount_in * leverage as u64 / 10)?
    };
    
    // Check minimum output amount
    require!(
        amount_out >= min_amount_out,
        WhiplashError::SlippageToleranceExceeded
    );
    
    // Transfer token from user to vault
    let cpi_accounts_in = Transfer {
        from: ctx.accounts.user_token_in.to_account_info(),
        to: if is_x_to_y {
            ctx.accounts.token_x_vault.to_account_info()
        } else {
            ctx.accounts.token_y_vault.to_account_info()
        },
        authority: ctx.accounts.user.to_account_info(),
    };
    let cpi_program = ctx.accounts.token_program.to_account_info();
    let cpi_ctx_in = CpiContext::new(cpi_program.clone(), cpi_accounts_in);
    token::transfer(cpi_ctx_in, amount_in)?;
    
    // Transfer token from vault to position token account
    let pool_signer_seeds = &[
        b"pool".as_ref(),
        ctx.accounts.pool.token_x_mint.as_ref(),
        ctx.accounts.pool.token_y_mint.as_ref(),
        &[ctx.accounts.pool.bump],
    ];
    let pool_signer = &[&pool_signer_seeds[..]];
    
    let cpi_accounts_out = Transfer {
        from: if is_x_to_y {
            ctx.accounts.token_y_vault.to_account_info()
        } else {
            ctx.accounts.token_x_vault.to_account_info()
        },
        to: ctx.accounts.position_token_account.to_account_info(),
        authority: ctx.accounts.pool.to_account_info(),
    };
    let cpi_ctx_out = CpiContext::new_with_signer(cpi_program, cpi_accounts_out, pool_signer);
    token::transfer(cpi_ctx_out, amount_out)?;
    
    // Initialize position data
    let position = &mut ctx.accounts.position;
    position.authority = ctx.accounts.user.key();
    position.pool = ctx.accounts.pool.key();
    position.position_vault = ctx.accounts.position_token_account.key();
    position.is_long = is_x_to_y; // long if X to Y, short if Y to X
    position.collateral = amount_in;
    position.leverage = leverage;
    position.size = amount_out;
    
    // Calculate entry price (simple estimation as average price) as Q64.64 u128
    let entry_price = ((amount_in as u128 * leverage as u128) << 64) / ((amount_out as u128) << 64);
    position.entry_price = entry_price;
    
    // Update pool reserves
    let pool = &mut ctx.accounts.pool;
    if is_x_to_y {
        pool.token_x_amount = pool.token_x_amount.checked_add(amount_in)
            .ok_or(error!(WhiplashError::MathOverflow))?;
        pool.token_y_amount = pool.token_y_amount.checked_sub(amount_out)
            .ok_or(error!(WhiplashError::MathOverflow))?;
    } else {
        pool.token_y_amount = pool.token_y_amount.checked_add(amount_in)
            .ok_or(error!(WhiplashError::MathOverflow))?;
        pool.token_x_amount = pool.token_x_amount.checked_sub(amount_out)
            .ok_or(error!(WhiplashError::MathOverflow))?;
    }
    
    // Emit swap event
    emit!(Swapped {
        user: ctx.accounts.user.key(),
        pool: ctx.accounts.pool.key(),
        token_in_mint: ctx.accounts.user_token_in.mint,
        token_out_mint: ctx.accounts.user_token_out.mint,
        amount_in,
        amount_out,
        timestamp: Clock::get()?.unix_timestamp,
    });
    
    Ok(())
} 