import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { Whiplash } from "../target/types/whiplash";
import { expect } from "chai";
import {
  PublicKey, 
  Keypair, 
  SystemProgram,
  SYSVAR_RENT_PUBKEY,
} from "@solana/web3.js";
import {
  TOKEN_PROGRAM_ID,
  ASSOCIATED_TOKEN_PROGRAM_ID,
  createMint,
  createAssociatedTokenAccount,
  mintTo,
  getAssociatedTokenAddress,
  getAccount,
} from "@solana/spl-token";
import BN from "bn.js";

describe("whiplash", () => {
  // Configure the client to use the local cluster
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.Whiplash as Program<Whiplash>;
  const wallet = provider.wallet as anchor.Wallet;

  // Test state
  let tokenYMint: PublicKey;
  let tokenYAccount: PublicKey;
  let poolPda: PublicKey;
  let poolBump: number;
  let tokenYVault: PublicKey;

  // Pool initial values
  const INITIAL_TOKEN_AMOUNT = 1_000_000_000_000; // 1 million tokens
  const INITIAL_POOL_LIQUIDITY = 100_000_000; // 100 tokens with 6 decimals
  const SWAP_AMOUNT = 10_000_000; // 10 tokens with 6 decimals
  const DECIMALS = 9;
  const INITIAL_VIRTUAL_SOL = 1_000_000_000; // 1 SOL

  before(async () => {
    // Create token mint for our tests
    const mintAuthority = wallet.payer;
    
    // Create token Y mint
    tokenYMint = await createMint(
      provider.connection,
      mintAuthority,
      mintAuthority.publicKey,
      null,
      DECIMALS
    );
    console.log("Created Token Y Mint:", tokenYMint.toBase58());

    // Create associated token account for wallet
    tokenYAccount = await createAssociatedTokenAccount(
      provider.connection,
      mintAuthority,
      tokenYMint,
      wallet.publicKey
    );
    console.log("Created Token Y Account:", tokenYAccount.toBase58());

    // Mint tokens to wallet
    await mintTo(
      provider.connection,
      mintAuthority,
      tokenYMint,
      tokenYAccount,
      mintAuthority.publicKey,
      INITIAL_TOKEN_AMOUNT
    );
    console.log(`Minted ${INITIAL_TOKEN_AMOUNT} tokens to Token Y Account`);

    // Derive the pool PDA
    [poolPda, poolBump] = await PublicKey.findProgramAddressSync(
      [
        Buffer.from("pool"),
        tokenYMint.toBuffer(),
      ],
      program.programId
    );
    console.log("Pool PDA:", poolPda.toBase58(), "with bump:", poolBump);

    // Calculate the token vault address
    tokenYVault = await getAssociatedTokenAddress(
      tokenYMint,
      poolPda,
      true // allowOwnerOffCurve
    );
    
    console.log("Token Y Vault:", tokenYVault.toBase58());
  });

  it("Launches a pool", async () => {
    // Launch the pool
    const tx = await program.methods
      .launch(poolBump, new BN(INITIAL_VIRTUAL_SOL))
      .accounts({
        authority: wallet.publicKey,
        tokenYMint: tokenYMint,
        pool: poolPda,
        tokenYVault: tokenYVault,
        systemProgram: SystemProgram.programId,
        tokenProgram: TOKEN_PROGRAM_ID,
        associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        rent: SYSVAR_RENT_PUBKEY,
      })
      .rpc();
    
    console.log("Launch pool transaction signature", tx);

    // Verify pool state
    const poolAccount = await program.account.pool.fetch(poolPda);
    expect(poolAccount.authority.toString()).to.equal(wallet.publicKey.toString());
    expect(poolAccount.tokenYMint.toString()).to.equal(tokenYMint.toString());
    expect(poolAccount.tokenYVault.toString()).to.equal(tokenYVault.toString());
    expect(poolAccount.virtualSolReserve.toNumber()).to.equal(INITIAL_VIRTUAL_SOL);
    expect(poolAccount.tokenYAmount.toNumber()).to.equal(0);
    expect(poolAccount.bump).to.equal(poolBump);
  });

  it("Adds initial liquidity to the pool", async () => {
    // Add initial liquidity
    const tx = await program.methods
      .addLiquidity(
        new BN(INITIAL_VIRTUAL_SOL), // amountSolDesired
        new BN(INITIAL_POOL_LIQUIDITY), // amountYDesired
        new BN(INITIAL_VIRTUAL_SOL), // amountSolMin
        new BN(INITIAL_POOL_LIQUIDITY) // amountYMin
      )
      .accounts({
        provider: wallet.publicKey,
        pool: poolPda,
        tokenYVault: tokenYVault,
        providerTokenY: tokenYAccount,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .rpc();
    
    console.log("Add initial liquidity transaction signature", tx);

    // Verify pool state
    const poolAccount = await program.account.pool.fetch(poolPda);
    expect(poolAccount.tokenYAmount.toNumber()).to.equal(INITIAL_POOL_LIQUIDITY);
  });

  it("Swaps SOL for token Y", async () => {
    // Get initial balances
    const initialTokenYAccountInfo = await getAccount(provider.connection, tokenYAccount);
    const initialTokenYBalance = Number(initialTokenYAccountInfo.amount);

    // Get initial pool state
    const initialPoolAccount = await program.account.pool.fetch(poolPda);
    const initialPoolYAmount = initialPoolAccount.tokenYAmount.toNumber();

    // Calculate expected output amount based on constant product formula
    // output_amount = (reserve_out * input_amount) / (reserve_in + input_amount)
    const expectedOutputAmount = Math.floor(
      (initialPoolYAmount * SWAP_AMOUNT) / (INITIAL_VIRTUAL_SOL + SWAP_AMOUNT)
    );
    
    console.log(`Expected output amount from SOL->Y swap: ${expectedOutputAmount}`);

    // Allow 1% slippage
    const minOutputAmount = Math.floor(expectedOutputAmount * 0.99);

    // Perform the swap
    const tx = await program.methods
      .swap(
        new BN(SWAP_AMOUNT),     // amountIn
        new BN(minOutputAmount)  // minAmountOut
      )
      .accounts({
        user: wallet.publicKey,
        pool: poolPda,
        tokenYVault: tokenYVault,
        userTokenIn: wallet.publicKey,
        userTokenOut: tokenYAccount,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .rpc();
    
    console.log("Swap SOL->Y transaction signature", tx);

    // Verify balances after swap
    const finalTokenYAccountInfo = await getAccount(provider.connection, tokenYAccount);
    const finalTokenYBalance = Number(finalTokenYAccountInfo.amount);

    // Verify pool state
    const finalPoolAccount = await program.account.pool.fetch(poolPda);
    const finalPoolYAmount = finalPoolAccount.tokenYAmount.toNumber();

    // Check user tokens changed correctly
    expect(finalTokenYBalance).to.be.above(initialTokenYBalance);
    expect(finalTokenYBalance - initialTokenYBalance).to.be.at.least(minOutputAmount);

    // Check pool reserves changed correctly
    expect(finalPoolYAmount).to.equal(initialPoolYAmount - (finalTokenYBalance - initialTokenYBalance));
  });

  it("Swaps token Y for SOL", async () => {
    // Get initial balances
    const initialTokenYAccountInfo = await getAccount(provider.connection, tokenYAccount);
    const initialTokenYBalance = Number(initialTokenYAccountInfo.amount);

    // Get initial pool state
    const initialPoolAccount = await program.account.pool.fetch(poolPda);
    const initialPoolYAmount = initialPoolAccount.tokenYAmount.toNumber();

    // Calculate expected output amount based on constant product formula
    // output_amount = (reserve_out * input_amount) / (reserve_in + input_amount)
    const expectedOutputAmount = Math.floor(
      (INITIAL_VIRTUAL_SOL * SWAP_AMOUNT) / (initialPoolYAmount + SWAP_AMOUNT)
    );
    
    console.log(`Expected output amount from Y->SOL swap: ${expectedOutputAmount}`);

    // Allow 1% slippage
    const minOutputAmount = Math.floor(expectedOutputAmount * 0.99);

    // Perform the swap
    const tx = await program.methods
      .swap(
        new BN(SWAP_AMOUNT),     // amountIn
        new BN(minOutputAmount)  // minAmountOut
      )
      .accounts({
        user: wallet.publicKey,
        pool: poolPda,
        tokenYVault: tokenYVault,
        userTokenIn: tokenYAccount,
        userTokenOut: wallet.publicKey,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .rpc();
    
    console.log("Swap Y->SOL transaction signature", tx);

    // Verify balances after swap
    const finalTokenYAccountInfo = await getAccount(provider.connection, tokenYAccount);
    const finalTokenYBalance = Number(finalTokenYAccountInfo.amount);

    // Verify pool state
    const finalPoolAccount = await program.account.pool.fetch(poolPda);
    const finalPoolYAmount = finalPoolAccount.tokenYAmount.toNumber();

    // Check user tokens changed correctly
    expect(finalTokenYBalance).to.equal(initialTokenYBalance - SWAP_AMOUNT);

    // Check pool reserves changed correctly
    expect(finalPoolYAmount).to.equal(initialPoolYAmount + SWAP_AMOUNT);
  });

  it("Adds more liquidity to the pool", async () => {
    // Get current pool state to calculate the correct ratio
    const poolAccount = await program.account.pool.fetch(poolPda);
    const poolYAmount = poolAccount.tokenYAmount.toNumber();
    
    // Calculate amounts to add (we'll add 20% more liquidity)
    const additionalLiquidityPercentage = 0.2; // 20%
    const additionalSolAmount = Math.floor(INITIAL_VIRTUAL_SOL * additionalLiquidityPercentage);
    const additionalYAmount = Math.floor(poolYAmount * additionalLiquidityPercentage);
    
    // Get initial balances
    const initialTokenYAccountInfo = await getAccount(provider.connection, tokenYAccount);
    const initialTokenYBalance = Number(initialTokenYAccountInfo.amount);
    
    console.log(`Adding more liquidity: ${additionalSolAmount} SOL and ${additionalYAmount} Y`);

    // Allow 0.5% slippage
    const minSolAmount = Math.floor(additionalSolAmount * 0.995);
    const minYAmount = Math.floor(additionalYAmount * 0.995);

    // Add more liquidity
    const tx = await program.methods
      .addLiquidity(
        new BN(additionalSolAmount), // amountSolDesired
        new BN(additionalYAmount),   // amountYDesired
        new BN(minSolAmount),        // amountSolMin
        new BN(minYAmount)           // amountYMin
      )
      .accounts({
        provider: wallet.publicKey,
        pool: poolPda,
        tokenYVault: tokenYVault,
        providerTokenY: tokenYAccount,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .rpc();
    
    console.log("Add additional liquidity transaction signature", tx);

    // Verify pool reserves increased
    const finalPoolAccount = await program.account.pool.fetch(poolPda);
    const finalPoolYAmount = finalPoolAccount.tokenYAmount.toNumber();
    
    expect(finalPoolYAmount).to.be.above(poolYAmount);
    
    // Verify wallet tokens decreased
    const finalTokenYAccountInfo = await getAccount(provider.connection, tokenYAccount);
    const finalTokenYBalance = Number(finalTokenYAccountInfo.amount);
    
    expect(initialTokenYBalance - finalTokenYBalance).to.be.at.least(minYAmount);
  });

  it("Opens a leveraged position", async () => {
    const amountIn = 1_000_000; // 0.001 SOL
    const minAmountOut = 0;
    const leverage = 2;
    
    const [position] = await PublicKey.findProgramAddressSync(
      [Buffer.from("position"), poolPda.toBuffer(), wallet.publicKey.toBuffer()],
      program.programId
    );
    
    const [positionTokenAccount] = await PublicKey.findProgramAddressSync(
      [Buffer.from("position_token"), position.toBuffer()],
      program.programId
    );
    
    const tx = await program.methods
      .leverageSwap(new BN(amountIn), new BN(minAmountOut), leverage)
      .accounts({
        user: wallet.publicKey,
        pool: poolPda,
        tokenYVault: tokenYVault,
        userTokenIn: wallet.publicKey,
        userTokenOut: tokenYAccount,
        position,
        positionTokenAccount,
        positionTokenMint: tokenYMint,
        tokenProgram: TOKEN_PROGRAM_ID,
        associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
        rent: SYSVAR_RENT_PUBKEY,
      })
      .rpc();
      
    console.log("Leverage swap transaction signature", tx);

    // Verify position state
    const positionAccount = await program.account.position.fetch(position);
    expect(positionAccount.authority.toString()).to.equal(wallet.publicKey.toString());
    expect(positionAccount.leverage).to.equal(leverage);
  });

  it("Liquidates an underwater position", async () => {
    // Create a position that will be liquidated
    const [position] = await PublicKey.findProgramAddressSync(
      [Buffer.from("position"), poolPda.toBuffer(), wallet.publicKey.toBuffer()],
      program.programId
    );

    // Create position token account
    const [positionTokenAccount] = await PublicKey.findProgramAddressSync(
      [Buffer.from("position_token"), position.toBuffer()],
      program.programId
    );

    // Open a leveraged position
    const leverage = 5; // High leverage to ensure underwater
    const collateral = 1_000_000; // 0.001 SOL
    const amountIn = collateral * leverage;

    const tx = await program.methods
      .leverageSwap(new BN(amountIn), new BN(0), leverage)
      .accounts({
        user: wallet.publicKey,
        pool: poolPda,
        tokenYVault: tokenYVault,
        userTokenIn: wallet.publicKey,
        userTokenOut: tokenYAccount,
        position,
        positionTokenAccount,
        positionTokenMint: tokenYMint,
        tokenProgram: TOKEN_PROGRAM_ID,
        associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
        rent: SYSVAR_RENT_PUBKEY,
      })
      .rpc();

    console.log("Leverage swap transaction signature", tx);

    // Simulate price movement that makes position liquidatable
    // In a real scenario, this would happen through market movements
    // For testing, we'll just call liquidate directly
    const liquidateTx = await program.methods
      .liquidate()
      .accounts({
        liquidator: wallet.publicKey,
        pool: poolPda,
        tokenYVault: tokenYVault,
        position,
        positionTokenAccount,
        liquidatorTokenAccount: tokenYAccount,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    console.log("Liquidation transaction signature", liquidateTx);

    // Verify position was liquidated
    const positionAccount = await program.account.position.fetch(position);
    expect(positionAccount.size.toNumber()).to.equal(0);
  });
}); 