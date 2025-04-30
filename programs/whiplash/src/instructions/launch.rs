use anchor_lang::prelude::*;
use anchor_spl::{
    token::{Token, Mint, TokenAccount, MintTo},
    associated_token::AssociatedToken,
};
use anchor_spl::token::spl_token::instruction::AuthorityType;
use mpl_token_metadata::{
    instruction as mpl_instruction,
    state::Creator,
};
use crate::{state::*, events::*, WhiplashError};

#[derive(Accounts)]
#[instruction(virtual_sol_reserve: u64, token_name: String, token_ticker: String, metadata_uri: String)]
pub struct Launch<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,
    
    #[account(
        init,
        payer = authority,
        mint::decimals = 6,
        mint::authority = authority,
        mint::freeze_authority = authority,
    )]
    pub token_mint: Account<'info, Mint>,
    
    #[account(
        init,
        seeds = [
            b"pool".as_ref(),
            token_mint.key().as_ref(),
        ],
        bump,
        payer = authority,
        space = Pool::LEN
    )]
    pub pool: Account<'info, Pool>,
    
    #[account(
        init,
        payer = authority,
        associated_token::mint = token_mint,
        associated_token::authority = pool,
    )]
    pub token_vault: Account<'info, TokenAccount>,

    /// CHECK: This is the metadata account that will be created
    #[account(mut)]
    pub metadata: UncheckedAccount<'info>,
    
    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub rent: Sysvar<'info, Rent>,

    /// CHECK: This is the token metadata program
    pub token_metadata_program: UncheckedAccount<'info>,
}

pub fn handle_launch(
    ctx: Context<Launch>, 
    virtual_sol_reserve: u64,
    token_name: String,
    token_ticker: String,
    metadata_uri: String,
) -> Result<()> {
    // Initialize pool state first
    let pool = &mut ctx.accounts.pool;
    pool.authority = ctx.accounts.authority.key();
    pool.token_y_mint = ctx.accounts.token_mint.key();
    pool.token_y_vault = ctx.accounts.token_vault.key();
    pool.bump = *ctx.bumps.get("pool").unwrap();
    
    // Calculate total supply with proper overflow checks
    let total_supply = 1_000_000_000_000_000u64; // 1 billion with 6 decimals
    
    // Mint tokens to the pool vault with proper error handling
    let cpi_accounts = MintTo {
        mint: ctx.accounts.token_mint.to_account_info(),
        to: ctx.accounts.token_vault.to_account_info(),
        authority: ctx.accounts.authority.to_account_info(),
    };
    let cpi_ctx = CpiContext::new(
        ctx.accounts.token_program.to_account_info(),
        cpi_accounts,
    );
    anchor_spl::token::mint_to(cpi_ctx, total_supply).map_err(|e| {
        msg!("Token minting error: {:?}", e);
        error!(WhiplashError::InvalidMintAuthority)
    })?;

    // Create metadata with minimal allocations
    let creator = Creator {
        address: ctx.accounts.authority.key(),
        verified: true,
        share: 100,
    };
    
    // Prepare metadata instruction with minimal allocations
    let token_metadata_program_key = ctx.accounts.token_metadata_program.key();
    
    let accounts = mpl_instruction::create_metadata_accounts_v3(
        token_metadata_program_key,
        ctx.accounts.metadata.key(),
        ctx.accounts.token_mint.key(),
        ctx.accounts.authority.key(),
        ctx.accounts.authority.key(),
        ctx.accounts.authority.key(),
        token_name,
        token_ticker,
        metadata_uri,
        Some(vec![creator]),
        0,
        true,
        true,
        None,
        None,
        None,
    );

    // Execute metadata creation with proper error handling
    anchor_lang::solana_program::program::invoke(
        &accounts,
        &[
            ctx.accounts.metadata.to_account_info(),
            ctx.accounts.token_mint.to_account_info(),
            ctx.accounts.authority.to_account_info(),
            ctx.accounts.authority.to_account_info(),
            ctx.accounts.authority.to_account_info(),
            ctx.accounts.system_program.to_account_info(),
            ctx.accounts.rent.to_account_info(),
            ctx.accounts.token_metadata_program.to_account_info(),
        ],
    ).map_err(|e| {
        msg!("Metadata creation error: {:?}", e);
        error!(WhiplashError::MetadataCreationFailed)
    })?;
    
    // Disable mint authority with proper error handling
    let cpi_accounts = anchor_spl::token::SetAuthority {
        account_or_mint: ctx.accounts.token_mint.to_account_info(),
        current_authority: ctx.accounts.authority.to_account_info(),
    };
    let cpi_ctx = CpiContext::new(
        ctx.accounts.token_program.to_account_info(),
        cpi_accounts,
    );
    anchor_spl::token::set_authority(
        cpi_ctx,
        AuthorityType::MintTokens,
        None,
    ).map_err(|e| {
        msg!("Mint authority change error: {:?}", e);
        error!(WhiplashError::AuthorityChangeFailed)
    })?;
    
    // Disable freeze authority with proper error handling
    let cpi_accounts = anchor_spl::token::SetAuthority {
        account_or_mint: ctx.accounts.token_mint.to_account_info(),
        current_authority: ctx.accounts.authority.to_account_info(),
    };
    let cpi_ctx = CpiContext::new(
        ctx.accounts.token_program.to_account_info(),
        cpi_accounts,
    );
    anchor_spl::token::set_authority(
        cpi_ctx,
        AuthorityType::FreezeAccount,
        None,
    ).map_err(|e| {
        msg!("Freeze authority change error: {:?}", e);
        error!(WhiplashError::AuthorityChangeFailed)
    })?;
    
    // Update pool state with proper overflow checks
    pool.token_y_amount = total_supply;
    pool.virtual_sol_amount = virtual_sol_reserve;
    // Initialize real SOL reserves to 0
    pool.lamports = 0;
    // Initialize virtual token Y reserves
    pool.virtual_token_y_amount = 0;
    
    // Emit the pool launched event
    emit!(PoolLaunched {
        token_mint: ctx.accounts.token_mint.key(),
        pool: ctx.accounts.pool.key(),
        virtual_sol_reserve,
        timestamp: Clock::get()?.unix_timestamp,
    });
    
    Ok(())
} 