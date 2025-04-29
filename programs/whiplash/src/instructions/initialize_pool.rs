use anchor_lang::prelude::*;
use anchor_spl::{
    token::{Token, Mint, TokenAccount},
    associated_token::AssociatedToken,
};
use crate::{state::*, events::*};

#[derive(Accounts)]
#[instruction(bump: u8, initial_virtual_sol: u64)]
pub struct Launch<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,
    
    pub token_y_mint: Account<'info, Mint>,
    
    #[account(
        init,
        seeds = [
            b"pool".as_ref(),
            token_y_mint.key().as_ref(),
        ],
        bump,
        payer = authority,
        space = Pool::LEN
    )]
    pub pool: Account<'info, Pool>,
    
    #[account(
        init,
        payer = authority,
        associated_token::mint = token_y_mint,
        associated_token::authority = pool,
    )]
    pub token_y_vault: Account<'info, TokenAccount>,
    
    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub rent: Sysvar<'info, Rent>,
}

pub fn handle_launch(ctx: Context<Launch>, bump: u8, initial_virtual_sol: u64) -> Result<()> {
    let pool = &mut ctx.accounts.pool;
    
    // Set up the pool state
    pool.authority = ctx.accounts.authority.key();
    pool.token_y_mint = ctx.accounts.token_y_mint.key();
    pool.token_y_vault = ctx.accounts.token_y_vault.key();
    pool.virtual_sol_reserve = initial_virtual_sol;
    pool.token_y_amount = 0;
    pool.bump = bump;
    
    // Emit the pool launched event
    emit!(PoolLaunched {
        token_y_mint: ctx.accounts.token_y_mint.key(),
        pool: ctx.accounts.pool.key(),
        initial_virtual_sol,
        timestamp: Clock::get()?.unix_timestamp,
    });
    
    Ok(())
} 