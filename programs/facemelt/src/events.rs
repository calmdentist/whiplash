use anchor_lang::prelude::*;

#[event]
pub struct PoolLaunched {
    pub token_mint: Pubkey,
    pub pool: Pubkey,
    pub virtual_sol_reserve: u64,
    pub timestamp: i64,
}

#[event]
pub struct Swapped {
    pub user: Pubkey,
    pub pool: Pubkey,
    pub token_in_mint: Pubkey,
    pub token_out_mint: Pubkey,
    pub amount_in: u64,
    pub amount_out: u64,
    pub timestamp: i64,
}

#[event]
pub struct PositionOpened {
    pub user: Pubkey,
    pub pool: Pubkey,
    pub position: Pubkey,
    pub is_long: bool,
    pub collateral: u64,
    pub leverage: u32,
    pub size: u64,
    pub entry_price: u128,
    pub timestamp: i64,
}

#[event]
pub struct PositionClosed {
    pub user: Pubkey,
    pub pool: Pubkey,
    pub position: Pubkey,
    pub is_long: bool,
    pub position_size: u64,
    pub borrowed_amount: u64,
    pub output_amount: u64,
    pub user_received: u64,
    pub timestamp: i64,
}

#[event]
pub struct PositionLiquidated {
    pub liquidator: Pubkey,
    pub position_owner: Pubkey,
    pub pool: Pubkey,
    pub position: Pubkey,
    pub position_size: u64,
    pub borrowed_amount: u64,
    pub expected_output: u64,
    pub liquidator_reward: u64,
    pub timestamp: i64,
}

#[event]
pub struct BondingCurveLaunched {
    pub token_mint: Pubkey,
    pub bonding_curve: Pubkey,
    pub pool: Pubkey,
    pub authority: Pubkey,
    pub total_supply: u64,
    pub target_sol: u64,
    pub target_tokens_sold: u64,
    pub timestamp: i64,
}

#[event]
pub struct BondingCurveSwapped {
    pub user: Pubkey,
    pub bonding_curve: Pubkey,
    pub input_is_sol: bool,
    pub amount_in: u64,
    pub amount_out: u64,
    pub tokens_sold_on_curve: u64,
    pub sol_raised_on_curve: u64,
    pub timestamp: i64,
}

#[event]
pub struct BondingCurveGraduated {
    pub bonding_curve: Pubkey,
    pub pool: Pubkey,
    pub token_mint: Pubkey,
    pub sol_raised: u64,
    pub tokens_for_lp: u64,
    pub timestamp: i64,
} 