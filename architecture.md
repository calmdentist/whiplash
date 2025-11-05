### Whiplash Architecture Overview

## Pool State

sol_reserve (lamports) - real sol in vault (for auditing)
token_reserve - real token in vault (for auditing)
effective_sol_reserve
effective_token_reserve
total_delta_k_longs - sum of original delta_k from all open LONG positions
total_delta_k_shorts - sum of original delta_k from all open SHORT positions
cumulative_funding_accumulator
last_updated_timestamp

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

## Position State

size (original size, denominated in the output token)
delta_k (original delta_k)
entry_funding_accumulator
is_long (boolean)
collateral

## Functions

function swap(amount_in, input_is_sol, min_amount_out):
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
        pool.total_delta_k_longs -= position.delta_k # Remove original full debt
        # Settle by swapping back effective size and receiving payout
        pool.effective_token_reserve += effective_size
        pool.effective_sol_reserve -= payout
    else:
        payout = (pool.effective_token_reserve * effective_size - effective_delta_k) / (pool.effective_sol_reserve + effective_size)
        payout_is_sol = false
        pool.total_delta_k_shorts -= position.delta_k # Remove original full debt
        # Settle by swapping back effective size and receiving payout
        pool.effective_sol_reserve += effective_size
        pool.effective_token_reserve -= payout
    
    return payout, payout_is_sol

function liquidate(position, liquidator):
    # This function would be implemented using the same core logic as close_position,
    # with an added check for liquidatability and a reward distribution mechanism.
    # The settlement of the position's debt against the pool state is identical.
    return