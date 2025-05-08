use anchor_lang::prelude::*;
use crate::WhiplashError;


pub fn calculate_position_expected_output(
    total_x: u64,
    total_y: u64,
    position_size: u64,
    is_long: bool,
) -> Result<u64> {
    // Using u128 for intermediate calculations
    let x_u128: u128 = total_x as u128;
    let y_u128: u128 = total_y as u128;
    let position_size_u128: u128 = position_size as u128;
    
    // Calculate the expected output based on position type
    let expected_output_u128 = if is_long {
        // Long position: holding tokens, need to calculate Token->SOL swap
        // Formula: (x * y_position) / (y + y_position)
        (x_u128.checked_mul(position_size_u128)
            .ok_or(error!(WhiplashError::MathOverflow))?)
            .checked_div(y_u128.checked_add(position_size_u128)
                .ok_or(error!(WhiplashError::MathOverflow))?)
            .ok_or(error!(WhiplashError::MathOverflow))?
    } else {
        // Short position: holding SOL, need to calculate SOL->Token swap
        // Formula: (y * x_position) / (x + x_position)
        (y_u128.checked_mul(position_size_u128)
            .ok_or(error!(WhiplashError::MathOverflow))?)
            .checked_div(x_u128.checked_add(position_size_u128)
                .ok_or(error!(WhiplashError::MathOverflow))?)
            .ok_or(error!(WhiplashError::MathOverflow))?
    };

    // Ensure the result fits in u64
    if expected_output_u128 > u64::MAX as u128 {
        return Err(error!(WhiplashError::MathOverflow));
    }

    // Return the result directly without any adjustment, letting the
    // caller handle any needed adjustments based on position type
    Ok(expected_output_u128 as u64)
} 