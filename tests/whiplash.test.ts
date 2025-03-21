import * as anchor from "@coral-xyz/anchor";
import { BN, Program } from "@coral-xyz/anchor";
import { ultrasound } from "../target/types/whiplash";
import { PublicKey, Keypair } from "@solana/web3.js";
import { TOKEN_PROGRAM_ID, createMint, createAccount, mintTo } from "@solana/spl-token";
import { assert } from "chai";

describe("whiplash", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);
  const program = anchor.workspace.Whiplash as Program<whiplash>;

  // Test variables
  let token0Mint: PublicKey;
  let token1Mint: PublicKey;
  let userToken0Account: PublicKey;
  let userToken1Account: PublicKey;
  let poolToken0Account: PublicKey;
  let poolToken1Account: PublicKey;
  let poolPDA: PublicKey;
  let poolAuthority: Keypair;
  let tickBitmap: PublicKey;

  const initialMintAmount = 1_000_000_000; // 1 billion tokens
  const feeTier = 500; // 0.05%

  before(async () => {
    // Create token mints
    token0Mint = await createMint(
      provider.connection,
      (provider.wallet as anchor.Wallet).payer,
      provider.wallet.publicKey,
      null,
      6
    );
    token1Mint = await createMint(
      provider.connection,
      (provider.wallet as anchor.Wallet).payer,
      provider.wallet.publicKey,
      null,
      6
    );

    // Derive the pool PDA
    const [poolAddress, _] = await PublicKey.findProgramAddress(
      [Buffer.from("pool"), token0Mint.toBuffer(), token1Mint.toBuffer()],
      program.programId
    );
    poolPDA = poolAddress;

    // Create user token accounts
    userToken0Account = await createAccount(
      provider.connection,
      (provider.wallet as anchor.Wallet).payer,
      token0Mint,
      provider.wallet.publicKey
    );
    userToken1Account = await createAccount(
      provider.connection,
      (provider.wallet as anchor.Wallet).payer,
      token1Mint,
      provider.wallet.publicKey
    );

    // Mint tokens to user
    await mintTo(
      provider.connection,
      (provider.wallet as anchor.Wallet).payer,
      token0Mint,
      userToken0Account,
      (provider.wallet as anchor.Wallet).payer,
      initialMintAmount
    );
    await mintTo(
      provider.connection,
      (provider.wallet as anchor.Wallet).payer,
      token1Mint,
      userToken1Account,
      (provider.wallet as anchor.Wallet).payer,
      initialMintAmount
    );

    // Create pool authority
    poolAuthority = Keypair.generate();

    // Create pool token accounts
    poolToken0Account = await createAccount(
      provider.connection,
      (provider.wallet as anchor.Wallet).payer,
      token0Mint,
      poolAuthority.publicKey
    );
    poolToken1Account = await createAccount(
      provider.connection,
      (provider.wallet as anchor.Wallet).payer,
      token1Mint,
      poolAuthority.publicKey
    );

    // Derive the tick bitmap PDA using poolPDA
    const [tickBitmapPDA] = await PublicKey.findProgramAddress(
      [Buffer.from("tickbitmap"), poolPDA.toBuffer()],
      program.programId
    );
    tickBitmap = tickBitmapPDA;

    // Debug logging
    console.log("Pool Configuration:");
    console.log("  Pool PDA:", poolPDA.toBase58());
    console.log("  Pool Authority:", poolAuthority.publicKey.toBase58());
    console.log("  Pool Token0 Account:", poolToken0Account.toBase58());
    console.log("  Pool Token1 Account:", poolToken1Account.toBase58());
    console.log("  TickBitmap Account:", tickBitmap.toBase58());
  });

  it("Initialize Pool", async () => {
    await program.methods
      .initializePool(feeTier)
      .accounts({
        pool: poolPDA,
        tickBitmap: tickBitmap,
        user: provider.wallet.publicKey,
        token0: token0Mint,
        token1: token1Mint,
        poolTokenAccount0: poolToken0Account,
        poolTokenAccount1: poolToken1Account,
        systemProgram: anchor.web3.SystemProgram.programId,
      })
      .rpc();

    // Verify pool initialization
    const poolAccount = await program.account.pool.fetch(poolPDA);
    assert.ok(poolAccount.token0.equals(token0Mint));
    assert.ok(poolAccount.token1.equals(token1Mint));
    assert.ok(poolAccount.tokenAccount0.equals(poolToken0Account));
    assert.ok(poolAccount.tokenAccount1.equals(poolToken1Account));
    assert.equal(poolAccount.feeTier, feeTier);
    assert.equal(poolAccount.sqrtPrice.toString(), "0");
    assert.equal(poolAccount.liquidity.toString(), "0");
    assert.ok(poolAccount.tickBitmap.equals(tickBitmap));
  });

  it("Add Liquidity", async () => {
    const amount0 = 100_000;
    const amount1 = 100_000;

    await program.methods
      .addLiquidity(new BN(amount0), new BN(amount1))
      .accounts({
        pool: poolPDA,
        user: provider.wallet.publicKey,
        poolAuthority: poolAuthority.publicKey,
        tokenAccount0: userToken0Account,
        poolTokenAccount0: poolToken0Account,
        tokenAccount1: userToken1Account,
        poolTokenAccount1: poolToken1Account,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([poolAuthority])
      .rpc();

    const poolAccount = await program.account.pool.fetch(poolPDA);
    assert.equal(poolAccount.reserve0.toNumber(), amount0);
    assert.equal(poolAccount.reserve1.toNumber(), amount1);
    assert.ok(poolAccount.liquidity.gt(new BN(0)), "Pool should have liquidity");
    assert.ok(poolAccount.sqrtPrice.gt(new BN(0)), "Pool should have non-zero sqrt_price");
  });

  it("Swap token0 for token1", async () => {
    const amountIn = 10_000;
    const minimumAmountOut = 0;

    await program.methods
      .swap(new BN(amountIn), new BN(minimumAmountOut))
      .accounts({
        pool: poolPDA,
        tickBitmap: tickBitmap,
        tokenAccountIn: userToken0Account,
        tokenAccountOut: userToken1Account,
        poolTokenAccount0: poolToken0Account,
        poolTokenAccount1: poolToken1Account,
        user: provider.wallet.publicKey,
        poolAuthority: poolAuthority.publicKey,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([poolAuthority])
      .rpc();
  });

  it("Swap token1 for token0", async () => {
    const amountIn = 10_000;
    const minimumAmountOut = 0;

    await program.methods
      .swap(new BN(amountIn), new BN(minimumAmountOut))
      .accounts({
        pool: poolPDA,
        tickBitmap: tickBitmap,
        tokenAccountIn: userToken1Account,
        tokenAccountOut: userToken0Account,
        poolTokenAccount0: poolToken0Account,
        poolTokenAccount1: poolToken1Account,
        user: provider.wallet.publicKey,
        poolAuthority: poolAuthority.publicKey,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([poolAuthority])
      .rpc();
  });

  it("Leveraged swap creates a new position", async () => {
    // Set test parameters for leveraged swap.
    const leveragedAmount = 10_000;
    const minimumAmountOut = 0;
    const leverageFactor = 2;

    // Fetch the current pool state to check the borrowed amount.
    const poolBefore = await program.account.pool.fetch(poolPDA);
    const initialBorrowed = new BN(poolBefore.borrowedFromBid.toString());

    // Derive the PDA for the new individual position.
    const amountBuffer = Buffer.alloc(8);
    amountBuffer.writeBigUInt64LE(BigInt(leveragedAmount));
    const [positionPDA] = await PublicKey.findProgramAddress(
      [
        Buffer.from("position"),
        poolPDA.toBuffer(),
        provider.wallet.publicKey.toBuffer(),
        amountBuffer,
      ],
      program.programId
    );

    // Execute a leveraged swap.
    await program.methods
      .swapLeveraged(new BN(leveragedAmount), new BN(minimumAmountOut), leverageFactor)
      .accounts({
        pool: poolPDA,
        tickBitmap: tickBitmap,
        position: positionPDA,
        tokenAccountIn: userToken1Account, // token1 input implies a leveraged long
        tokenAccountOut: userToken0Account,
        poolTokenAccount0: poolToken0Account,
        poolTokenAccount1: poolToken1Account,
        user: provider.wallet.publicKey,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: anchor.web3.SystemProgram.programId,
      })
      .rpc();

    // Verify the newly created position.
    const positionAccount = await program.account.position.fetch(positionPDA);
    assert.equal(
      positionAccount.collateral.toString(),
      leveragedAmount.toString(),
      "Position collateral should match the leveraged amount"
    );
    assert.equal(positionAccount.isLong, true, "Position should be long for a leveraged swap with token1 input");

    // Verify that the pool's borrowed amount increased as expected.
    const poolAfter = await program.account.pool.fetch(poolPDA);
    const expectedBorrowedIncrease = new BN(leveragedAmount).mul(new BN(leverageFactor - 1));
    const actualBorrowedIncrease = new BN(poolAfter.borrowedFromBid.toString()).sub(initialBorrowed);
    assert.ok(
      actualBorrowedIncrease.eq(expectedBorrowedIncrease),
      `Pool borrowedFromBid should increase by ${expectedBorrowedIncrease.toString()}, but got ${actualBorrowedIncrease.toString()}`
    );
  });

  it("Remove Liquidity", async () => {
    const liquidityToRemove = 50_000; // Remove half of the liquidity

    await program.methods
      .removeLiquidity(new BN(liquidityToRemove))
      .accounts({
        pool: poolPDA,
        user: provider.wallet.publicKey,
        poolAuthority: poolAuthority.publicKey,
        tokenAccount0: userToken0Account,
        poolTokenAccount0: poolToken0Account,
        tokenAccount1: userToken1Account,
        poolTokenAccount1: poolToken1Account,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([poolAuthority])
      .rpc();

    const poolAccount = await program.account.pool.fetch(poolPDA);
    // Add assertions to verify the new pool state after liquidity removal.
  });

  it("Verify AMM invariants and fee-based k increase", async () => {
    // First, add a fresh amount of liquidity.
    const initialAmount0 = 100_000;
    const initialAmount1 = 100_000;
    const feeTier = 500;
    const feeMultiplier = BigInt(10000 + feeTier) / BigInt(10000);

    await program.methods
      .addLiquidity(new BN(initialAmount0), new BN(initialAmount1))
      .accounts({
        pool: poolPDA,
        user: provider.wallet.publicKey,
        poolAuthority: poolAuthority.publicKey,
        tokenAccount0: userToken0Account,
        poolTokenAccount0: poolToken0Account,
        tokenAccount1: userToken1Account,
        poolTokenAccount1: poolToken1Account,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([poolAuthority])
      .rpc();

    // Get the initial pool state.
    let poolState = await program.account.pool.fetch(poolPDA);
    const initialK =
      BigInt(poolState.reserve0.toString()) * BigInt(poolState.reserve1.toString());
    const initialSqrtPrice = poolState.sqrtPrice;

    console.log("Initial state:");
    console.log("  Reserve0:", poolState.reserve0.toString());
    console.log("  Reserve1:", poolState.reserve1.toString());
    console.log("  Initial k:", initialK.toString());
    console.log("  Initial sqrt price:", initialSqrtPrice.toString());
    console.log("  Fee multiplier:", feeMultiplier.toString());

    // Perform a token0 for token1 swap (buy).
    const buyAmount = 10_000;
    await program.methods
      .swap(new BN(buyAmount), new BN(0))
      .accounts({
        pool: poolPDA,
        tickBitmap: tickBitmap,
        tokenAccountIn: userToken0Account,
        tokenAccountOut: userToken1Account,
        poolTokenAccount0: poolToken0Account,
        poolTokenAccount1: poolToken1Account,
        user: provider.wallet.publicKey,
        poolAuthority: poolAuthority.publicKey,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([poolAuthority])
      .rpc();

    poolState = await program.account.pool.fetch(poolPDA);
    const kAfterBuy =
      BigInt(poolState.reserve0.toString()) * BigInt(poolState.reserve1.toString());
    const expectedMinKAfterBuy = initialK * feeMultiplier;

    console.log("\nAfter buy:");
    console.log("  Reserve0:", poolState.reserve0.toString());
    console.log("  Reserve1:", poolState.reserve1.toString());
    console.log("  Actual k:", kAfterBuy.toString());
    console.log("  Expected min k:", expectedMinKAfterBuy.toString());
    console.log("  Sqrt price:", poolState.sqrtPrice.toString());

    assert(
      kAfterBuy >= expectedMinKAfterBuy,
      `k should increase by at least the fee percentage. Expected: ${expectedMinKAfterBuy}, Got: ${kAfterBuy}`
    );

    // Perform a token1 for token0 swap (sell).
    const sellAmount = 5_000;
    await program.methods
      .swap(new BN(sellAmount), new BN(0))
      .accounts({
        pool: poolPDA,
        tickBitmap: tickBitmap,
        tokenAccountIn: userToken1Account,
        tokenAccountOut: userToken0Account,
        poolTokenAccount0: poolToken0Account,
        poolTokenAccount1: poolToken1Account,
        user: provider.wallet.publicKey,
        poolAuthority: poolAuthority.publicKey,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([poolAuthority])
      .rpc();

    poolState = await program.account.pool.fetch(poolPDA);
    const kAfterSell =
      BigInt(poolState.reserve0.toString()) * BigInt(poolState.reserve1.toString());
    const expectedMinKAfterSell = kAfterBuy * feeMultiplier;

    console.log("\nAfter sell:");
    console.log("  Reserve0:", poolState.reserve0.toString());
    console.log("  Reserve1:", poolState.reserve1.toString());
    console.log("  Actual k:", kAfterSell.toString());
    console.log("  Expected min k:", expectedMinKAfterSell.toString());
    console.log("  Sqrt price:", poolState.sqrtPrice.toString());

    assert(
      kAfterSell >= expectedMinKAfterSell,
      `k should increase by at least the fee percentage. Expected: ${expectedMinKAfterSell}, Got: ${kAfterSell}`
    );

    const totalKIncrease = (kAfterSell * BigInt(10000)) / initialK - BigInt(10000);
    console.log("\nTotal k increase (basis points):", totalKIncrease.toString());

    if (poolState.sqrtPrice < initialSqrtPrice) {
      assert.equal(
        poolState.lastSlotPrice.toString(),
        initialSqrtPrice.toString(),
        "Bid price should remain constant within slot window"
      );
    }
  });
});
