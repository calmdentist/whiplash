class AMM:
    def __init__(self, reserve_x=1000.0, reserve_y=1000.0):
        """
        Initialize the AMM with reserves.
        reserve_x: reserve for token X (e.g. ETH)
        reserve_y: reserve for token Y (e.g. DOG)
        """
        self.reserve_x = float(reserve_x)
        self.reserve_y = float(reserve_y)
        # Positions for leveraged trades, keyed by user identifier.
        self.positions = {}

    def get_k(self):
        """Return the invariant product k = reserve_x * reserve_y."""
        return self.reserve_x * self.reserve_y

    def spot_swap_x_for_y(self, x_in):
        """
        Swap token X for token Y as a spot trade.
        Uses the constant-product formula:
            y_out = reserve_y - k/(reserve_x + x_in)
        Then updates the pool reserves.
        """
        k = self.get_k()
        new_x = self.reserve_x + x_in
        new_y = k / new_x
        y_out = self.reserve_y - new_y

        self.reserve_x = new_x
        self.reserve_y = new_y
        return y_out

    def spot_swap_y_for_x(self, y_in):
        """
        Swap token Y for token X as a spot trade.
        Uses the constant-product formula:
            x_out = reserve_x - k/(reserve_y + y_in)
        Then updates the pool reserves.
        """
        k = self.get_k()
        new_y = self.reserve_y + y_in
        new_x = k / new_y
        x_out = self.reserve_x - new_x

        self.reserve_y = new_y
        self.reserve_x = new_x
        return x_out

    def open_leveraged_position(self, user, deposit_x, leverage):
        """
        Open a leveraged position for a user.

        Parameters:
            user: an identifier for the user (e.g. "Bob").
            deposit_x: the actual token X deposit.
            leverage: the leverage factor (e.g. 5).

        Process:
         - Compute the notional input: notional_x = deposit_x * leverage.
         - Compute the virtual outcome of swapping notional_x of token X:
               virtual_new_x = current reserve_x + notional_x,
               virtual_new_y = k/(current reserve_x + notional_x),
               dog_received = current reserve_y - virtual_new_y.
         - Update pool reserves:
              reserve_x increases only by deposit_x (real liquidity),
              reserve_y is set equal to virtual_new_y.
         - Store the position with:
              borrowed amount = notional_x - deposit_x,
              dog_amount = dog_received.
        """
        notional_x = deposit_x * leverage
        borrowed = notional_x - deposit_x

        k = self.get_k()
        virtual_new_x = self.reserve_x + notional_x
        virtual_new_y = k / virtual_new_x
        dog_received = self.reserve_y - virtual_new_y

        # Update the real reserves (only the deposit is added to token X)
        self.reserve_x += deposit_x
        self.reserve_y = virtual_new_y

        # Store the leveraged position for the user.
        self.positions[user] = {
            "borrowed": borrowed,
            "dog_amount": dog_received,
            "leverage": leverage,
            "deposit": deposit_x
        }
        return dog_received

    def close_leveraged_position(self, user):
        """
        Close a leveraged position for a user.

        Process:
         - Retrieve the user's position.
         - Augment the current X reserve by adding back the borrowed liquidity:
               x_virtual = current reserve_x + borrowed.
         - The virtual invariant is K_virtual = x_virtual * current reserve_y.
         - Simulate swapping the saved dog_amount for token X:
               x_out_virtual = x_virtual - (K_virtual/(current reserve_y + dog_amount)).
         - The user's net payout is then:
               user_payout = x_out_virtual - borrowed.
         - Update pool reserves:
               reserve_y is increased by the dog_amount,
               reserve_x is decreased by (x_out_virtual - borrowed).
         - Remove the user's position.
        """
        if user not in self.positions:
            raise ValueError("No leveraged position open for user.")

        pos = self.positions[user]
        borrowed = pos["borrowed"]
        dog_amount = pos["dog_amount"]

        # Augment the X reserve with the borrowed (virtual liquidity)
        x_virtual = self.reserve_x + borrowed
        K_virtual = x_virtual * self.reserve_y

        # Simulate swap: swapping dog_amount of Y for X on the virtual pool.
        x_out_virtual = x_virtual - (K_virtual / (self.reserve_y + dog_amount))

        # User's net payout is then:
        user_payout = x_out_virtual - borrowed

        # Update the real pool reserves:
        self.reserve_y += dog_amount
        self.reserve_x -= (x_out_virtual - borrowed)

        # Remove the user's leveraged position.
        del self.positions[user]

        return user_payout


def compute_liquidation_threshold(k_before, reserve_x, reserve_y, position_amount):
    """
    Compute the target Y reserve (y_t) at which a leveraged position would be liquidated.
    The system of equations is:
        x_t * y_t = current_k   and
        x_t * (y_t + position_amount) = k_before
    where:
        current_k = reserve_x * reserve_y
    Solving, we get:
        y_t = (position_amount * current_k) / (k_before - current_k)
    """
    current_k = reserve_x * reserve_y
    if k_before <= current_k:
        # In practice this should not happen
        return float('inf')
    return position_amount * current_k / (k_before - current_k)


def simulate_multiple_positions():
    """
    Simulate the "case of multiple positions" scenario using exact computation of the
    liquidation target for each position via solving:

          x_t * y_t = current_k
          x_t * (y_t + position_amount) = k_before

    For a given position, solving for the target y_reserve gives:
          y_target = (position_amount * current_k) / (k_before - current_k)

    Scenario:
      - Start with initial reserves: 1000 ETH, 1000 DOG.
      - Bob opens a leveraged position with 10 ETH at 5x leverage.
      - Alice opens a leveraged position with 10 ETH at 2x leverage.
      
      Then Carol (the market) swaps in DOG until:
        1. The pool's Y reserve reaches Bob's computed target, triggering Bob's liquidation.
           (Bob's stored DOG amount is then added back to the pool.)
        2. Next, Carol swaps in DOG until the pool's Y reserve reaches Alice's computed target,
           triggering Alice's liquidation.
    """
    print("\n--- Simulating Multiple Positions Scenario (Exact Liquidation Target) ---")
    amm = AMM(1000, 1000)
    print("Initial reserves: X = {:.8f} ETH, Y = {:.8f} DOG".format(amm.reserve_x, amm.reserve_y))
    
    # --- Bob opens his leveraged position ---
    bob_deposit = 10
    bob_leverage = 5
    k_before_bob = amm.get_k()  # invariant before Bob's open (should be 1,000,000)
    bob_dog = amm.open_leveraged_position("Bob", bob_deposit, bob_leverage)
    print("\nBob opens leveraged position: deposit = {} ETH at {}x leverage".format(bob_deposit, bob_leverage))
    print("  DOG received (position_amount): {:.8f}".format(bob_dog))
    print("Reserves after Bob open: X = {:.8f} ETH, Y = {:.8f} DOG".format(amm.reserve_x, amm.reserve_y))
    k_after_bob = amm.get_k()
    y_target_bob = compute_liquidation_threshold(k_before_bob, amm.reserve_x, amm.reserve_y, bob_dog)
    print("Bob's liquidation target Y reserve computed as: {:.8f} DOG".format(y_target_bob))
    
    # --- Alice opens her leveraged position ---
    alice_deposit = 10
    alice_leverage = 2
    k_before_alice = amm.get_k()  # invariant before Alice's open (after Bob's open)
    alice_dog = amm.open_leveraged_position("Alice", alice_deposit, alice_leverage)
    print("\nAlice opens leveraged position: deposit = {} ETH at {}x leverage".format(alice_deposit, alice_leverage))
    print("  DOG received (position_amount): {:.8f}".format(alice_dog))
    print("Reserves after Alice open: X = {:.8f} ETH, Y = {:.8f} DOG".format(amm.reserve_x, amm.reserve_y))
    k_after_alice = amm.get_k()
    y_target_alice = compute_liquidation_threshold(k_before_alice, amm.reserve_x, amm.reserve_y, alice_dog)
    print("Alice's liquidation target Y reserve computed as: {:.8f} DOG".format(y_target_alice))
    
    # --- Liquidate Bob ---
    # Carol supplies DOG until the pool's Y reserve reaches Bob's computed target.
    if amm.reserve_y < y_target_bob:
        y_in_bob = y_target_bob - amm.reserve_y
        x_out = amm.spot_swap_y_for_x(y_in_bob)
        print("\nCarol swaps in {:.8f} DOG to raise the pool Y reserve to {:.8f} DOG (target for Bob)".format(
            y_in_bob, amm.reserve_y))
        print("Reserves after Carol's swap: X = {:.8f} ETH, Y = {:.8f} DOG".format(amm.reserve_x, amm.reserve_y))
    else:
        print("\nNo swap needed for Bob liquidation target.")
    
    bob_payout = amm.close_leveraged_position("Bob")
    print("\nBob is liquidated.")
    print("  Bob's net payout: {:.8f} ETH".format(bob_payout))
    print("Reserves after Bob liquidation: X = {:.8f} ETH, Y = {:.8f} DOG".format(amm.reserve_x, amm.reserve_y))
    
    # --- Liquidate Alice ---
    # Now Carol supplies DOG until the pool's Y reserve reaches Alice's computed target.
    if amm.reserve_y < y_target_alice:
        y_in_alice = y_target_alice - amm.reserve_y
        x_out = amm.spot_swap_y_for_x(y_in_alice)
        print("\nCarol swaps in {:.8f} DOG to raise the pool Y reserve to {:.8f} DOG (target for Alice)".format(
            y_in_alice, amm.reserve_y))
        print("Reserves after Carol's swap: X = {:.8f} ETH, Y = {:.8f} DOG".format(amm.reserve_x, amm.reserve_y))
    else:
        print("\nNo swap needed for Alice liquidation target.")
    
    alice_payout = amm.close_leveraged_position("Alice")
    print("\nAlice is liquidated.")
    print("  Alice's net payout: {:.8f} ETH".format(alice_payout))
    print("Reserves after Alice liquidation: X = {:.8f} ETH, Y = {:.8f} DOG".format(amm.reserve_x, amm.reserve_y))
    
    print("\nFinal invariant: k = {:.8f}".format(amm.get_k()))


if __name__ == "__main__":
    # You can run different simulation functions.
    # For example, to run the multiple positions simulation with computed liquidation targets:
    simulate_multiple_positions() 