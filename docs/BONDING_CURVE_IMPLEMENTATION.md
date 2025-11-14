# Bonding Curve Implementation

This document describes the bonding curve liquidity bootstrapping mechanism implemented for the Facemelt protocol.

## Overview

The bonding curve allows for a fair and decentralized token launch mechanism before transitioning to the full AMM with leverage functionality. It uses a linear bonding curve formula to price tokens during the initial distribution phase.

## Architecture

### 1. State: BondingCurve (`programs/facemelt/src/state/bonding_curve.rs`)

A new state account that tracks the bonding curve parameters and current state:

**Key Fields:**
- `bonding_curve_slope_m`: The slope of the linear bonding curve (in fixed-point)
- `tokens_sold_on_curve`: Counter for tokens sold during bonding phase
- `sol_raised_on_curve`: Counter for SOL raised during bonding phase
- `bonding_target_sol`: Target SOL to be raised (default: 200 SOL)
- `bonding_target_tokens_sold`: Target tokens to be sold (default: 280M tokens)
- `status`: Active or Graduated

**Key Functions:**
- `calculate_slope()`: Calculates m = (2 * target_sol) / (target_tokens_sold^2)
- `calculate_tokens_out_for_sol()`: Uses formula q2 = sqrt(q1^2 + (2 * sol_in) / m)
- `calculate_sol_out_for_tokens()`: Uses formula sol_out = (m * (q1^2 - q2^2)) / 2

### 2. Instruction: launch_on_curve (`programs/facemelt/src/instructions/launch_on_curve.rs`)

Launches a new token with bonding curve liquidity bootstrapping.

**Default Parameters:**
- Total supply: 420M tokens (with 6 decimals)
- Target SOL to raise: 200 SOL
- Target tokens to sell: 280M tokens
- Remaining tokens (140M) will become LP tokens when graduated

**Process:**
1. Creates new token mint with metadata
2. Mints total supply to token vault
3. Initializes BondingCurve account with calculated slope
4. Initializes Pool account in uninitialized state
5. Disables mint and freeze authorities
6. Emits `BondingCurveLaunched` event

### 3. Instruction: swap_on_curve (`programs/facemelt/src/instructions/swap_on_curve.rs`)

Handles buying and selling tokens on the bonding curve.

**Features:**

**Buying (SOL → Tokens):**
- Calculates tokens out based on SOL input
- If purchase would exceed target, caps at target and refunds excess SOL
- Transfers SOL to pool PDA
- Transfers tokens to buyer
- Checks for graduation threshold
- Calls `graduate_to_amm()` if threshold reached

**Selling (Tokens → SOL):**
- Calculates SOL out based on token input
- Transfers tokens back to vault
- Transfers SOL from pool to seller
- Updates bonding curve state

**Graduation (`graduate_to_amm()`):**
- Marks bonding curve as graduated
- Calculates LP tokens: target_tokens_sold / 2
- Initializes pool reserves:
  - SOL reserve: sol_raised_on_curve
  - Token reserve: lp_tokens (140M with defaults)
- Initializes EMA price oracle
- Refunds any excess SOL to last buyer
- Emits `BondingCurveGraduated` event

## Events

Three new events were added:

1. **BondingCurveLaunched**: Emitted when a new bonding curve is created
2. **BondingCurveSwapped**: Emitted on each buy/sell transaction
3. **BondingCurveGraduated**: Emitted when bonding curve graduates to AMM

## Error Codes

New error codes added:
- `InvalidBondingCurveParams`: Invalid parameters for bonding curve
- `InsufficientCurveTokens`: Not enough tokens left on curve
- `InsufficientCurveSol`: Not enough SOL in curve for payout
- `InsufficientTokensSold`: Cannot sell more tokens than sold
- `BondingCurveAlreadyGraduated`: Curve has already graduated
- `BondingCurveNotActive`: Curve is not in active state

## Integration

The bonding curve is fully integrated into the Facemelt program:
- State module exports `BondingCurve`
- Instructions module exports `LaunchOnCurve` and `SwapOnCurve`
- Program exposes `launch_on_curve()` and `swap_on_curve()` public functions

## Usage Flow

1. **Launch**: Call `launch_on_curve()` to create a new token with bonding curve
2. **Trade**: Users call `swap_on_curve()` to buy/sell tokens on the curve
3. **Graduate**: Automatically transitions to AMM when 200 SOL is raised
4. **AMM Trading**: After graduation, all standard AMM functions become available

## Mathematical Formula

The bonding curve uses a linear pricing model:
- Price at quantity q: `price(q) = m * q`
- Cost to buy from q1 to q2: `cost = (m * (q2^2 - q1^2)) / 2`
- Slope: `m = (2 * target_sol) / (target_tokens_sold^2)`

This creates a fair launch where price increases linearly with quantity sold, and the curve automatically graduates when the target is reached.

## Files Modified/Created

**Created:**
- `programs/facemelt/src/state/bonding_curve.rs`
- `programs/facemelt/src/instructions/launch_on_curve.rs`
- `programs/facemelt/src/instructions/swap_on_curve.rs`

**Modified:**
- `programs/facemelt/src/state/mod.rs` - Added bonding_curve module
- `programs/facemelt/src/instructions/mod.rs` - Added new instructions
- `programs/facemelt/src/lib.rs` - Added public instruction handlers
- `programs/facemelt/src/error.rs` - Added bonding curve errors
- `programs/facemelt/src/events.rs` - Added bonding curve events
- `programs/facemelt/src/state/pool.rs` - Made PRICE_PRECISION public

## Build Status

✅ All files compile successfully with no warnings
✅ Anchor build completes successfully
✅ All borrow checker issues resolved
✅ All fixed-point math operations use checked arithmetic

