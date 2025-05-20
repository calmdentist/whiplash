import math

class WhiplashAMMSimulator:
    def __init__(self, sol_init, token_init, sol_virtual, token_virtual):
        self.sol = sol_init  # real SOL reserve
        self.token = token_init  # real TOKEN reserve
        self.sol_virtual = sol_virtual
        self.token_virtual = token_virtual
        self.k = (self.sol + self.sol_virtual) * (self.token + self.token_virtual)

    def get_price(self):
        # Price of 1 TOKEN in SOL (spot price)
        return (self.sol + self.sol_virtual) / (self.token + self.token_virtual)

    def spot_swap_token_for_sol(self, token_in):
        """
        User swaps token_in TOKEN for SOL (TOKEN -> SOL direction)
        Returns: sol_out, new reserves
        """
        token_reserve_before = self.token
        sol_reserve_before = self.sol
        token_new = self.token + token_in
        sol_virtual_total = self.sol + self.sol_virtual
        token_virtual_total = token_new + self.token_virtual
        # (sol' + sol_virtual) * (token' + token_virtual) = k
        sol_virtual_new = self.k / token_virtual_total
        sol_new = sol_virtual_new - self.sol_virtual
        sol_out = sol_reserve_before - sol_new
        # Update reserves
        self.token = token_new
        self.sol = sol_new
        return sol_out, self.sol, self.token

    def spot_swap_sol_for_token(self, sol_in):
        """
        User swaps sol_in SOL for TOKEN (SOL -> TOKEN direction)
        Returns: token_out, new reserves
        """
        sol_reserve_before = self.sol
        token_reserve_before = self.token
        sol_new = self.sol + sol_in
        sol_virtual_total = sol_new + self.sol_virtual
        token_virtual_total = self.k / sol_virtual_total
        token_new = token_virtual_total - self.token_virtual
        token_out = token_reserve_before - token_new
        # Update reserves
        self.sol = sol_new
        self.token = token_new
        return token_out, self.sol, self.token

    def leverage_long(self, collateral_sol, leverage):
        """
        Open a leveraged long position (SOL for TOKEN).
        The user deposits collateral_sol (real), borrows (leverage-1)x more (virtual), and swaps total for TOKEN.
        Returns: token_position, new reserves
        """
        borrowed_sol = collateral_sol * (leverage - 1)
        effective_sol = collateral_sol * leverage
        # Add collateral to real, borrowed to virtual
        self.sol += collateral_sol
        self.sol_virtual += borrowed_sol
        # Perform swap as if swapping effective_sol for TOKEN
        sol_virtual_total = self.sol + self.sol_virtual
        token_virtual_total = self.k / sol_virtual_total
        token_new = token_virtual_total - self.token_virtual
        token_position = self.token - token_new
        self.token = token_new
        return token_position, self.sol, self.token, self.sol_virtual, self.token_virtual

    def leverage_short(self, collateral_token, leverage):
        """
        Open a leveraged short position (TOKEN for SOL).
        The user deposits collateral_token (real), borrows (leverage-1)x more (virtual), and swaps total for SOL.
        Returns: sol_position, new reserves
        """
        borrowed_token = collateral_token * (leverage - 1)
        effective_token = collateral_token * leverage
        # Add collateral to real, borrowed to virtual
        self.token += collateral_token
        self.token_virtual += borrowed_token
        # Perform swap as if swapping effective_token for SOL
        token_virtual_total = self.token + self.token_virtual
        sol_virtual_total = self.k / token_virtual_total
        sol_new = sol_virtual_total - self.sol_virtual
        sol_position = self.sol - sol_new
        self.sol = sol_new
        return sol_position, self.sol, self.token, self.sol_virtual, self.token_virtual

    def status(self):
        return {
            'sol': self.sol,
            'token': self.token,
            'sol_virtual': self.sol_virtual,
            'token_virtual': self.token_virtual,
            'k': self.k,
            'price': self.get_price()
        }

if __name__ == "__main__":
    # Example usage
    # 1000 SOL (real), 1_000_000 TOKEN (real), 1000 SOL virtual, 0 TOKEN virtual
    amm = WhiplashAMMSimulator(sol_init=1000, token_init=1_000_000, sol_virtual=0, token_virtual=0)
    print("Initial status:", amm.status())

    # Swap 10 SOL for TOKEN
    token_out, sol_res, token_res = amm.spot_swap_sol_for_token(10)
    print(f"\nSwap 10 SOL for TOKEN: {token_out:.6f} TOKEN out")
    print("Status after swap:", amm.status())

    # Example leverage usage
    # Open a 5x long with 1000 SOL collateral
    token_pos, sol_res, token_res, sol_virt, token_virt = amm.leverage_long(collateral_sol=10000, leverage=5)
    print(f"\nOpen 5x long with 1000 SOL collateral: {token_pos:.6f} TOKEN position")
    print("Status after leverage long:", amm.status())

    # Swap initial token out from spot swap for SOL
    sol_out, sol_res, token_res = amm.spot_swap_token_for_sol(token_out)
    print("Status after swap:", amm.status())
