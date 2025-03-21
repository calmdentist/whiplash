use anchor_lang::prelude::*;
use crate::{
    state::{Pool, TickBitmap, LiquidationOrder},
    error::SrAmmError,
};

#[derive(Accounts)]
pub struct ManageLiquidation<'info> {
    #[account(mut)]
    pub pool: Account<'info, Pool>,
    #[account(mut)]
    pub tick_bitmap: Account<'info, TickBitmap>,
    #[account(mut)]
    pub user: Signer<'info>,
}

pub fn add_liquidation_order(
    ctx: Context<ManageLiquidation>,
    liquidation_price: u128,
    collateral: u64,
    is_long: bool,
) -> Result<()> {
    let pool = &mut ctx.accounts.pool;
    let bitmap = &mut ctx.accounts.tick_bitmap;

    // Convert price to tick
    let tick = TickBitmap::price_to_tick(liquidation_price)?;

    // Create new liquidation order
    let order = LiquidationOrder {
        owner: ctx.accounts.user.key(),
        position_id: pool.liquidation_orders.len() as u64,
        liquidation_price,
        collateral,
        is_long,
    };

    // Add order to pool
    pool.liquidation_orders.push(order);

    // Update bitmap
    bitmap.flip_tick(tick)?;

    Ok(())
}

pub fn remove_liquidation_order(
    ctx: Context<ManageLiquidation>,
    position_id: u64,
) -> Result<()> {
    let pool = &mut ctx.accounts.pool;
    let bitmap = &mut ctx.accounts.tick_bitmap;

    // Find and remove the order
    let order_index = pool.liquidation_orders
        .iter()
        .position(|order| order.position_id == position_id)
        .ok_or(SrAmmError::InvalidTokenAccount)?;

    let order = pool.liquidation_orders.remove(order_index);

    // Convert price to tick and update bitmap
    let tick = TickBitmap::price_to_tick(order.liquidation_price)?;
    bitmap.flip_tick(tick)?;

    Ok(())
} 