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
use crate::{state::*, events::*, FacemeltError};

#[derive(Accounts)]
#[instruction(token_name: String, token_ticker: String, metadata_uri: String)]
pub struct LaunchOnCurve<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,
    
    #[account(
        init,
        payer = authority,
        mint::decimals = 6,
        mint::authority = authority,
        mint::freeze_authority = authority,
    )]
    pub token_mint: Box<Account<'info, Mint>>,
    
    #[account(
        init,
        seeds = [
            b"bonding_curve".as_ref(),
            token_mint.key().as_ref(),
        ],
        bump,
        payer = authority,
        space = BondingCurve::LEN
    )]
    pub bonding_curve: Box<Account<'info, BondingCurve>>,
    
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
    pub pool: Box<Account<'info, Pool>>,
    
    #[account(
        init,
        payer = authority,
        associated_token::mint = token_mint,
        associated_token::authority = pool,
    )]
    pub token_vault: Box<Account<'info, TokenAccount>>,

    /// CHECK: This is the metadata account that will be created
    #[account(
        mut,
        seeds = [
            b"metadata",
            token_metadata_program.key().as_ref(),
            token_mint.key().as_ref(),
        ],
        seeds::program = token_metadata_program.key(),
        bump,
    )]
    pub metadata: UncheckedAccount<'info>,
    
    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,

    /// CHECK: This is the token metadata program
    pub token_metadata_program: UncheckedAccount<'info>,
}

pub fn handle_launch_on_curve(
    ctx: Context<LaunchOnCurve>,
    token_name: String,
    token_ticker: String,
    metadata_uri: String,
    total_supply: Option<u64>,
    target_sol: Option<u64>,
    target_tokens_sold: Option<u64>,
) -> Result<()> {
    // Use defaults if not provided
    let total_supply = total_supply.unwrap_or(BondingCurve::DEFAULT_TOTAL_SUPPLY);
    let target_sol = target_sol.unwrap_or(BondingCurve::DEFAULT_TARGET_SOL);
    let target_tokens_sold = target_tokens_sold.unwrap_or(BondingCurve::DEFAULT_TARGET_TOKENS_SOLD);
    
    // Validate parameters
    require!(total_supply > 0, FacemeltError::InvalidBondingCurveParams);
    require!(target_sol > 0, FacemeltError::InvalidBondingCurveParams);
    require!(target_tokens_sold > 0, FacemeltError::InvalidBondingCurveParams);
    require!(
        target_tokens_sold <= total_supply,
        FacemeltError::InvalidBondingCurveParams
    );
    
    // Calculate the bonding curve slope
    let slope = BondingCurve::calculate_slope(target_sol, target_tokens_sold)?;
    
    // Mint total supply to the token vault
    {
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
            error!(FacemeltError::InvalidMintAuthority)
        })?;
    }

    // Create metadata - scope to reduce stack usage
    {
        let creator = Creator {
            address: ctx.accounts.authority.key(),
            verified: true,
            share: 100,
        };
        
        let token_metadata_program_key = ctx.accounts.token_metadata_program.key();
        let metadata_key = ctx.accounts.metadata.key();
        let mint_key = ctx.accounts.token_mint.key();
        let authority_key = ctx.accounts.authority.key();
        
        let accounts = mpl_instruction::create_metadata_accounts_v3(
            token_metadata_program_key,
            metadata_key,
            mint_key,
            authority_key,
            authority_key,
            authority_key,
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

        anchor_lang::solana_program::program::invoke(
            &accounts,
            &[
                ctx.accounts.metadata.to_account_info(),
                ctx.accounts.token_mint.to_account_info(),
                ctx.accounts.authority.to_account_info(),
                ctx.accounts.authority.to_account_info(),
                ctx.accounts.authority.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
                ctx.accounts.token_metadata_program.to_account_info(),
            ],
        ).map_err(|e| {
            msg!("Metadata creation error: {:?}", e);
            error!(FacemeltError::MetadataCreationFailed)
        })?;
    }
    
    // Disable mint authority - scope to reduce stack usage
    {
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
            error!(FacemeltError::AuthorityChangeFailed)
        })?;
    }
    
    // Disable freeze authority - scope to reduce stack usage
    {
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
            error!(FacemeltError::AuthorityChangeFailed)
        })?;
    }
    
    // Initialize bonding curve state
    let bonding_curve = &mut ctx.accounts.bonding_curve;
    bonding_curve.authority = ctx.accounts.authority.key();
    bonding_curve.token_mint = ctx.accounts.token_mint.key();
    bonding_curve.pool = ctx.accounts.pool.key();
    bonding_curve.token_vault = ctx.accounts.token_vault.key();
    bonding_curve.bonding_curve_slope_m = slope;
    bonding_curve.tokens_sold_on_curve = 0;
    bonding_curve.sol_raised_on_curve = 0;
    bonding_curve.bonding_target_sol = target_sol;
    bonding_curve.bonding_target_tokens_sold = target_tokens_sold;
    bonding_curve.status = BondingCurveStatus::Active as u8;
    bonding_curve.bump = *ctx.bumps.get("bonding_curve").unwrap();
    
    // Initialize pool state (uninitialized, will be activated on graduation)
    let pool = &mut ctx.accounts.pool;
    pool.authority = ctx.accounts.authority.key();
    pool.token_mint = ctx.accounts.token_mint.key();
    pool.token_vault = ctx.accounts.token_vault.key();
    pool.bump = *ctx.bumps.get("pool").unwrap();
    
    // Set reserves to 0 (will be set during graduation)
    pool.token_reserve = 0;
    pool.sol_reserve = 0;
    pool.effective_token_reserve = 0;
    pool.effective_sol_reserve = 0;
    
    // Initialize funding rate fields
    pool.total_delta_k_longs = 0;
    pool.total_delta_k_shorts = 0;
    pool.cumulative_funding_accumulator = 0;
    pool.last_update_timestamp = Clock::get()?.unix_timestamp;
    
    // Initialize EMA oracle fields
    pool.ema_price = 0;
    pool.ema_initialized = false;
    
    // Initialize configurable parameters with defaults
    {
        const PRECISION: u128 = 1u128 << 32;
        pool.funding_constant_c = PRECISION / 10000; // Default: 0.0001/sec
        pool.liquidation_divergence_threshold = 10; // Default: 10%
    }
    
    // Emit launch event
    {
        let token_mint_key = ctx.accounts.token_mint.key();
        let bonding_curve_key = ctx.accounts.bonding_curve.key();
        let pool_key = ctx.accounts.pool.key();
        let authority_key = ctx.accounts.authority.key();
        let timestamp = Clock::get()?.unix_timestamp;
        
        emit!(BondingCurveLaunched {
            token_mint: token_mint_key,
            bonding_curve: bonding_curve_key,
            pool: pool_key,
            authority: authority_key,
            total_supply,
            target_sol,
            target_tokens_sold,
            timestamp,
        });
    }
    
    Ok(())
}

