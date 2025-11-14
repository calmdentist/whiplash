### Facemelt Architecture Overview

## Pool State

sol_reserve (lamports) - real sol in vault (for auditing)
token_reserve - real token in vault (for auditing)
effective_sol_reserve
effective_token_reserve
total_delta_k_longs - sum of original delta_k from all open LONG positions
total_delta_k_shorts - sum of original delta_k from all open SHORT positions
cumulative_funding_accumulator
last_updated_timestamp
ema_price - exponential moving average of price (for manipulation detection)
ema_initialized - whether EMA has been set
pool_status - Enum (`Uninitialized`, `Live`)


function update_funding_accumulators(self, current_timestamp):
    delta_t = current_timestamp - self.last_updated_timestamp
    if delta_t <= 0: return

    total_delta_k = self.total_delta_k_longs + self.total_delta_k_shorts
    if total_delta_k <= 0:
        self.last_updated_timestamp = current_timestamp
        return

    effective_k = self.effective_sol_reserve * self.effective_token_reserve
    leverage_ratio = total_delta_k / effective_k
    funding_rate = C * (leverage_ratio ** 2)

    self.cumulative_funding_accumulator += (funding_rate * delta_t)

    # Calculate fees paid by each side, proportional to their share of the total debt
    fees_paid_by_longs = funding_rate * self.total_delta_k_longs * delta_t
    fees_paid_by_shorts = funding_rate * self.total_delta_k_shorts * delta_t

    # Convert k-denominated fees back to the appropriate reserve asset and distribute
    self.effective_token_reserve += fees_paid_by_longs / self.effective_sol_reserve
    self.effective_sol_reserve += fees_paid_by_shorts / self.effective_token_reserve
    
    # Funding payments reduce the outstanding total debt
    self.total_delta_k_longs -= fees_paid_by_longs
    self.total_delta_k_shorts -= fees_paid_by_shorts

    self.last_updated_timestamp = current_timestamp
    
    # Update EMA price oracle (nested in funding accumulator update)
    current_price = self.effective_sol_reserve / self.effective_token_reserve
    if not self.ema_initialized:
        self.ema_price = current_price
        self.ema_initialized = true
    else:
        alpha = delta_t / (EMA_HALF_LIFE + delta_t)  # 5 minute half-life
        self.ema_price = self.ema_price * (1 - alpha) + current_price * alpha

function check_liquidation_price_safety(self):
    if not self.ema_initialized:
        return true  # Allow if EMA not yet initialized
    
    spot_price = self.effective_sol_reserve / self.effective_token_reserve
    if spot_price >= self.ema_price:
        return true  # Price rising or stable, safe to liquidate
    
    divergence_pct = ((self.ema_price - spot_price) / self.ema_price) * 100
    return divergence_pct <= 10  # Block liquidation if >10% divergence

function calculate_position_remaining_factor(self, entry_funding_accumulator):
    self.update_funding_accumulators(current_timestamp())
    remaining_factor = 1.0 - (self.cumulative_funding_accumulator - entry_funding_accumulator)
    return max(0, min(1.0, remaining_factor))

function calculate_output(self, input_amount, input_is_sol):
    k = self.effective_sol_reserve * self.effective_token_reserve
    if input_is_sol:
        output = self.effective_token_reserve - (k / (self.effective_sol_reserve + input_amount))
    else:
        output = self.effective_sol_reserve - (k / (self.effective_token_reserve + input_amount))
    return output

## BondingCurve State

bonding_curve_slope_m - The slope of the linear bonding curve
tokens_sold_on_curve - Counter for tokens sold during bonding phase
sol_raised_on_curve - Counter for SOL raised during bonding phase
bonding_target_sol - The target SOL to be raised (e.g., 200 SOL)
bonding_target_tokens_sold - The target tokens to be sold (e.g., 280M)

## Position State

size (original size, denominated in the output token)
delta_k (original delta_k)
entry_funding_accumulator
is_long (boolean)
collateral

## Functions

function launch_bonding_curve(total_supply, target_sol, target_tokens_sold):
    # Creator deposits total_supply of the token into a program vault
    # A new BondingCurve account is created and initialized with the curve parameters
    bonding_curve.bonding_target_sol = target_sol
    bonding_curve.bonding_target_tokens_sold = target_tokens_sold
    bonding_curve.bonding_curve_slope_m = (2 * target_sol) / (target_tokens_sold^2)
    bonding_curve.tokens_sold_on_curve = 0
    bonding_curve.sol_raised_on_curve = 0
    # A new Pool account is created in an Uninitialized state
    pool.pool_status = Uninitialized

function swap_on_curve(bonding_curve, amount_in, input_is_sol):
    # This instruction operates on the BondingCurve account
    
    if input_is_sol:
        # Buying tokens with SOL
        sol_in = amount_in
        q1 = bonding_curve.tokens_sold_on_curve
        
        # Derived formula to find how many tokens can be bought for a given SOL amount
        # q2 = sqrt(q1^2 + (2 * sol_in) / m)
        q2 = (q1^2 + (2 * sol_in) / bonding_curve.bonding_curve_slope_m).sqrt()
        
        tokens_out = q2 - q1
        assert(q2 <= bonding_curve.bonding_target_tokens_sold, "Not enough tokens left on curve")

        # Transfer SOL from buyer to pool vault and update state
        transfer_sol(sol_in, from=buyer, to=pool_vault)
        bonding_curve.sol_raised_on_curve += sol_in
        bonding_curve.tokens_sold_on_curve += tokens_out

        # Transfer tokens from pool vault to buyer
        transfer_token(tokens_out, from=pool_vault, to=buyer)

        # Check for graduation
        if bonding_curve.sol_raised_on_curve >= bonding_curve.bonding_target_sol:
            self.graduate_to_amm(bonding_curve, pool)
    else:
        # Selling tokens for SOL
        tokens_in = amount_in
        q1 = bonding_curve.tokens_sold_on_curve
        q2 = q1 - tokens_in
        assert(q2 >= 0, "Cannot sell more tokens than have been sold")

        # Calculate SOL out using the integral formula
        sol_out = (bonding_curve.bonding_curve_slope_m * (q1^2 - q2^2)) / 2
        assert(sol_out <= bonding_curve.sol_raised_on_curve, "Not enough SOL in curve to pay out")

        # Transfer tokens from seller to pool vault and update state
        transfer_token(tokens_in, from=seller, to=pool_vault)
        bonding_curve.tokens_sold_on_curve -= tokens_in
        bonding_curve.sol_raised_on_curve -= sol_out

        # Transfer SOL from pool vault to seller
        transfer_sol(sol_out, from=pool_vault, to=seller)

function graduate_to_amm(bonding_curve, pool):
    # This is an internal function triggered by the final buy transaction
    
    lp_tokens = bonding_curve.bonding_target_tokens_sold / 2
    
    # The real and effective reserves in the Pool account are now seeded
    pool.sol_reserve = bonding_curve.sol_raised_on_curve
    pool.effective_sol_reserve = bonding_curve.sol_raised_on_curve
    pool.token_reserve = lp_tokens
    pool.effective_token_reserve = lp_tokens
    pool.pool_status = Live
    
    # The BondingCurve account can now be closed and its rent reclaimed.

function swap(amount_in, input_is_sol, min_amount_out):
    assert(pool.pool_status == Live, "AMM is not live yet")
    pool.update_funding_accumulators(current_timestamp())
    amount_out = pool.calculate_output(amount_in, input_is_sol)
    assert(amount_out >= min_amount_out, "Slippage limit exceeded")

    # Spot swaps update both real and effective reserves
    if input_is_sol:
        pool.sol_reserve += amount_in; pool.token_reserve -= amount_out
        pool.effective_sol_reserve += amount_in; pool.effective_token_reserve -= amount_out
    else:
        pool.token_reserve += amount_in; pool.sol_reserve -= amount_out
        pool.effective_token_reserve += amount_in; pool.effective_sol_reserve -= amount_out
    return amount_out

function leverage_swap(collateral, is_long, leverage):
    pool.update_funding_accumulators(current_timestamp())
    notional_amount = collateral * leverage
    position_size = pool.calculate_output(notional_amount, is_long)

    k_before = pool.effective_sol_reserve * pool.effective_token_reserve
    
    # Leverage swaps only update effective reserves
    if is_long:
        pool.effective_sol_reserve += collateral
        pool.effective_token_reserve -= position_size
        k_after = pool.effective_sol_reserve * pool.effective_token_reserve
        delta_k = k_before - k_after
        pool.total_delta_k_longs += delta_k
    else:
        pool.effective_token_reserve += collateral
        pool.effective_sol_reserve -= position_size
        k_after = pool.effective_sol_reserve * pool.effective_token_reserve
        delta_k = k_before - k_after
        pool.total_delta_k_shorts += delta_k

    return Position(size=position_size, delta_k=delta_k, ...)

function close_position(position):
    remaining_factor = pool.calculate_position_remaining_factor(position.entry_funding_accumulator)
    effective_size = position.size * remaining_factor
    effective_delta_k = position.delta_k * remaining_factor

    if position.is_long:
        payout = (pool.effective_sol_reserve * effective_size - effective_delta_k) / (pool.effective_token_reserve + effective_size)
        payout_is_sol = true
        # Remove EFFECTIVE delta_k from pool tracking
        # Funding fees reduce total_delta_k proportionally, so we subtract effective delta_k
        pool.total_delta_k_longs -= effective_delta_k
        # Settle by swapping back effective size and receiving payout
        pool.effective_token_reserve += effective_size
        pool.effective_sol_reserve -= payout
    else:
        payout = (pool.effective_token_reserve * effective_size - effective_delta_k) / (pool.effective_sol_reserve + effective_size)
        payout_is_sol = false
        # Remove EFFECTIVE delta_k from pool tracking
        pool.total_delta_k_shorts -= effective_delta_k
        # Settle by swapping back effective size and receiving payout
        pool.effective_sol_reserve += effective_size
        pool.effective_token_reserve -= payout
    
    return payout, payout_is_sol

function liquidate(position, liquidator):
    pool.update_funding_accumulators(current_timestamp())
    
    # 0. Check EMA oracle - block if price manipulation detected
    assert(pool.check_liquidation_price_safety(), "Liquidation blocked: price manipulation detected")
    
    remaining_factor = pool.calculate_position_remaining_factor(position.entry_funding_accumulator)
    effective_size = position.size * remaining_factor
    effective_delta_k = position.delta_k * remaining_factor

    # 1. Calculate the gross value of the position's effective size.
    position_value_in_collateral = pool.calculate_output(effective_size, !position.is_long)

    # 2. Calculate the net payout after repaying debt.
    if position.is_long:
        payout = (pool.effective_sol_reserve * effective_size - effective_delta_k) / (pool.effective_token_reserve + effective_size)
    else:
        payout = (pool.effective_token_reserve * effective_size - effective_delta_k) / (pool.effective_sol_reserve + effective_size)
    
    # 3. Prevent bad debt: position must not be underwater
    assert(payout > 0, "Position underwater - cannot liquidate")
    
    # 4. Check if the net payout is less than 5% of the gross value.
    liquidation_threshold = position_value_in_collateral * 0.05
    assert(payout <= liquidation_threshold, "Position not liquidatable")

    # 4. The liquidator's reward is the entire remaining payout.
    reward = payout
    
    # 5. Settle the position against the pool. This is the same logic as close_position.
    if position.is_long:
        pool.total_delta_k_longs -= effective_delta_k # Remove EFFECTIVE delta_k
        pool.effective_token_reserve += effective_size
        pool.effective_sol_reserve -= payout # The payout (reward) is removed
        transfer_sol(reward, to=liquidator)
    else:
        pool.total_delta_k_shorts -= effective_delta_k # Remove EFFECTIVE delta_k
        pool.effective_sol_reserve += effective_size
        pool.effective_token_reserve -= payout # The payout (reward) is removed
        transfer_token(reward, to=liquidator)
    
    # The position is now closed.
    return