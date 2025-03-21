use crate::error::SrAmmError;
use anchor_lang::prelude::*;

pub const SLOT_WINDOW_SIZE: u64 = 4; // Solana slot window size
pub const MINIMUM_LIQUIDITY: u128 = 1000;
pub const Q64: u128 = 1 << 64;

pub fn calculate_swap_outcome(
    current_sqrt_price: u128,
    last_slot_price: u128,
    amount_in: u64,
    total_liquidity: u128,
    locked_bid_liquidity: u128,
    locked_ask_liquidity: u128,
    is_buy: bool,
) -> Result<(u64, u128)> {
    msg!("Calculate Swap Outcome:");
    msg!("Total Liquidity: {}", total_liquidity);
    msg!("Locked Bid Liquidity: {}", locked_bid_liquidity);
    msg!("Locked Ask Liquidity: {}", locked_ask_liquidity);
    msg!("Is Buy: {}", is_buy);
    msg!("Current Sqrt Price: {}", current_sqrt_price);
    msg!("Last Slot Price: {}", last_slot_price);
    msg!("Amount In: {}", amount_in);

    // Check if there's any liquidity at all
    if total_liquidity == 0 {
        return Err(SrAmmError::InsufficientLiquidity.into());
    }

    let amount_in = amount_in as u128;
    
    if is_buy {
        // For buys, we use ask-side liquidity
        let available_liquidity = total_liquidity
            .checked_sub(locked_ask_liquidity)
            .ok_or(SrAmmError::MathError)?;
            
        msg!("Available liquidity for buy: {}", available_liquidity);
        
        if available_liquidity == 0 {
            return Err(SrAmmError::InsufficientLiquidity.into());
        }

        // Calculate price impact - For buys, price moves up from current_sqrt_price
        let price_delta = amount_in
            .checked_mul(Q64)
            .ok_or(SrAmmError::MathError)?
            .checked_div(available_liquidity)
            .ok_or(SrAmmError::MathError)?;
            
        let new_sqrt_price = current_sqrt_price
            .checked_add(price_delta)
            .ok_or(SrAmmError::MathError)?;

        // Calculate amount out using the average price
        let avg_sqrt_price = (current_sqrt_price + new_sqrt_price) / 2;
        let amount_out = amount_in
            .checked_mul(avg_sqrt_price)
            .ok_or(SrAmmError::MathError)?
            .checked_div(Q64)
            .ok_or(SrAmmError::MathError)?;
        
        Ok((amount_out as u64, new_sqrt_price))
    } else {
        let available_liquidity = total_liquidity
            .checked_sub(locked_bid_liquidity)
            .ok_or(SrAmmError::MathError)?;
            
        if available_liquidity == 0 {
            return Err(SrAmmError::InsufficientLiquidity.into());
        }

        // Calculate price impact - For sells, price moves down from current_sqrt_price
        let price_delta = amount_in
            .checked_mul(Q64)
            .ok_or(SrAmmError::MathError)?
            .checked_div(available_liquidity)
            .ok_or(SrAmmError::MathError)?;
            
        let new_sqrt_price = current_sqrt_price
            .checked_sub(price_delta)
            .ok_or(SrAmmError::MathError)?;

        // Calculate amount out using the average price
        let avg_sqrt_price = (current_sqrt_price + new_sqrt_price) / 2;
        let amount_out = amount_in
            .checked_mul(avg_sqrt_price)
            .ok_or(SrAmmError::MathError)?
            .checked_div(Q64)
            .ok_or(SrAmmError::MathError)?;
        
        Ok((amount_out as u64, new_sqrt_price))
    }
}

pub fn sqrt_price_to_price(sqrt_price: u128) -> Result<u128> {
    // First multiply by sqrt_price, then divide by Q64 to maintain precision
    Ok(sqrt_price.checked_mul(sqrt_price).ok_or(SrAmmError::MathError)? / Q64)
}

pub fn sqrt(mut value: u128) -> u128 {
    if value < 2 {
        return value;
    }

    let mut bit = 1u128 << (128 - 2);
    let mut res = 0;

    while bit > value {
        bit >>= 2;
    }

    while bit != 0 {
        if value >= res + bit {
            value -= res + bit;
            res = (res >> 1) + bit;
        } else {
            res >>= 1;
        }
        bit >>= 2;
    }

    res
}

pub fn calculate_liquidity_amount(
    amount_0: u64,
    amount_1: u64,
    sqrt_price: u128,
) -> Result<u128> {
    let amount_0 = amount_0 as u128;
    let amount_1 = amount_1 as u128;
    
    // Calculate liquidity based on geometric mean
    let liquidity = (amount_0
        .checked_mul(amount_1)
        .ok_or(SrAmmError::MathError)?)
        .checked_mul(sqrt_price)
        .ok_or(SrAmmError::MathError)?;

    if liquidity < MINIMUM_LIQUIDITY {
        return Err(SrAmmError::InsufficientLiquidity.into());
    }

    Ok(liquidity)
}

pub fn calculate_withdraw_amounts(
    liquidity: u128,
    total_liquidity: u128,
    reserve_0: u64,
    reserve_1: u64,
) -> Result<(u64, u64)> {
    let amount_0 = (liquidity as u128)
        .checked_mul(reserve_0 as u128)
        .ok_or(SrAmmError::MathError)?
        .checked_div(total_liquidity)
        .ok_or(SrAmmError::MathError)? as u64;

    let amount_1 = (liquidity as u128)
        .checked_mul(reserve_1 as u128)
        .ok_or(SrAmmError::MathError)?
        .checked_div(total_liquidity)
        .ok_or(SrAmmError::MathError)? as u64;

    Ok((amount_0, amount_1))
}

pub fn calculate_leveraged_swap_outcome(
    current_sqrt_price: u128,
    amount_in: u64,
    effective_liquidity: u128,
    is_buy: bool,
) -> Result<(u64, u128)> {
    // We follow a similar approach to calculate_swap_outcome but use the effective liquidity.
    let amount_in = amount_in as u128;
    
    if is_buy {
        // For leveraged buys (longs), price moves up.
        let price_delta = amount_in
            .checked_mul(Q64)
            .ok_or(SrAmmError::MathError)?
            .checked_div(effective_liquidity)
            .ok_or(SrAmmError::MathError)?;
            
        let new_sqrt_price = current_sqrt_price
            .checked_add(price_delta)
            .ok_or(SrAmmError::MathError)?;
            
        let avg_sqrt_price = (current_sqrt_price + new_sqrt_price) / 2;
        let amount_out = amount_in
            .checked_mul(avg_sqrt_price)
            .ok_or(SrAmmError::MathError)?
            .checked_div(Q64)
            .ok_or(SrAmmError::MathError)?;
            
        Ok((amount_out as u64, new_sqrt_price))
    } else {
        // For leveraged sells (shorts), price moves down.
        let price_delta = amount_in
            .checked_mul(Q64)
            .ok_or(SrAmmError::MathError)?
            .checked_div(effective_liquidity)
            .ok_or(SrAmmError::MathError)?;
            
        let new_sqrt_price = current_sqrt_price
            .checked_sub(price_delta)
            .ok_or(SrAmmError::MathError)?;
            
        let avg_sqrt_price = (current_sqrt_price + new_sqrt_price) / 2;
        let amount_out = amount_in
            .checked_mul(avg_sqrt_price)
            .ok_or(SrAmmError::MathError)?
            .checked_div(Q64)
            .ok_or(SrAmmError::MathError)?;
            
        Ok((amount_out as u64, new_sqrt_price))
    }
} 