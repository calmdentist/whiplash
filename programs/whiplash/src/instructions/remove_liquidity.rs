use anchor_lang::prelude::*;
use anchor_spl::token::{Token, TokenAccount};
use crate::{
    error::SrAmmError,
    state::*,
    utils::{
        token::transfer_tokens,
        math::calculate_withdraw_amounts,
    },
};

#[derive(Accounts)]
#[instruction(liquidity: u128)]
pub struct RemoveLiquidity<'info> {
    #[account(mut)]
    pub pool: Account<'info, Pool>,
    #[account(mut)]
    pub user: Signer<'info>,
    #[account(mut)]
    pub pool_authority: Signer<'info>,
    #[account(mut,
        constraint = token_account_0.mint == pool.token_0,
        constraint = token_account_0.owner == user.key()
    )]
    pub token_account_0: Account<'info, TokenAccount>,
    #[account(mut,
        constraint = pool_token_account_0.key() == pool.token_account_0
    )]
    pub pool_token_account_0: Account<'info, TokenAccount>,
    #[account(mut,
        constraint = token_account_1.mint == pool.token_1,
        constraint = token_account_1.owner == user.key()
    )]
    pub token_account_1: Account<'info, TokenAccount>,
    #[account(mut,
        constraint = pool_token_account_1.key() == pool.token_account_1
    )]
    pub pool_token_account_1: Account<'info, TokenAccount>,
    pub token_program: Program<'info, Token>,
}

pub fn handler(
    ctx: Context<RemoveLiquidity>,
    liquidity: u128,
) -> Result<()> {
    let pool = &mut ctx.accounts.pool;

    let (amount_0, amount_1) = calculate_withdraw_amounts(
        liquidity,
        pool.liquidity,
        pool.reserve_0,
        pool.reserve_1,
    )?;

    pool.reserve_0 = pool.reserve_0.checked_sub(amount_0)
        .ok_or(SrAmmError::MathError)?;
    pool.reserve_1 = pool.reserve_1.checked_sub(amount_1)
        .ok_or(SrAmmError::MathError)?;
    pool.liquidity = pool.liquidity.checked_sub(liquidity)
        .ok_or(SrAmmError::MathError)?;

    transfer_tokens(
        &ctx.accounts.token_program,
        &ctx.accounts.pool_token_account_0,
        &ctx.accounts.token_account_0,
        &ctx.accounts.pool_authority,
        amount_0,
    )?;

    transfer_tokens(
        &ctx.accounts.token_program,
        &ctx.accounts.pool_token_account_1,
        &ctx.accounts.token_account_1,
        &ctx.accounts.pool_authority,
        amount_1,
    )?;

    Ok(())
} 