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
  let tokenXMint: PublicKey;
  let tokenYMint: PublicKey;
  let tokenXAccount: PublicKey;
  let tokenYAccount: PublicKey;
  let poolPda: PublicKey;
  let poolBump: number;
  let tokenXVault: PublicKey;
  let tokenYVault: PublicKey;

  // Pool initial values
  const INITIAL_TOKEN_AMOUNT = 1_000_000_000; // 1000 tokens with 6 decimals
  const INITIAL_POOL_LIQUIDITY = 100_000_000; // 100 tokens with 6 decimals
  const SWAP_AMOUNT = 10_000_000; // 10 tokens with 6 decimals
  const DECIMALS = 6;

  before(async () => {
    // Create two token mints for our tests
    const mintAuthority = wallet.payer;
    
    // Create token X mint
    tokenXMint = await createMint(
      provider.connection,
      mintAuthority,
      mintAuthority.publicKey,
      null,
      DECIMALS
    );
    console.log("Created Token X Mint:", tokenXMint.toBase58());

    // Create token Y mint
    tokenYMint = await createMint(
      provider.connection,
      mintAuthority,
      mintAuthority.publicKey,
      null,
      DECIMALS
    );
    console.log("Created Token Y Mint:", tokenYMint.toBase58());

    // Create associated token accounts for wallet
    tokenXAccount = await createAssociatedTokenAccount(
      provider.connection,
      mintAuthority,
      tokenXMint,
      wallet.publicKey
    );
    console.log("Created Token X Account:", tokenXAccount.toBase58());

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
      tokenXMint,
      tokenXAccount,
      mintAuthority.publicKey,
      INITIAL_TOKEN_AMOUNT
    );
    console.log(`Minted ${INITIAL_TOKEN_AMOUNT} tokens to Token X Account`);

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
        tokenXMint.toBuffer(), 
        tokenYMint.toBuffer(),
      ],
      program.programId
    );
    console.log("Pool PDA:", poolPda.toBase58(), "with bump:", poolBump);

    // Calculate the token vaults addresses
    tokenXVault = await getAssociatedTokenAddress(
      tokenXMint,
      poolPda,
      true // allowOwnerOffCurve
    );
    
    tokenYVault = await getAssociatedTokenAddress(
      tokenYMint,
      poolPda,
      true // allowOwnerOffCurve
    );
    
    console.log("Token X Vault:", tokenXVault.toBase58());
    console.log("Token Y Vault:", tokenYVault.toBase58());
  });

  it("Initializes a pool", async () => {
    // Initialize the pool
    const tx = await program.methods
      .initializePool(poolBump)
      .accounts({
        authority: wallet.publicKey,
        tokenXMint: tokenXMint,
        tokenYMint: tokenYMint,
        pool: poolPda,
        tokenXVault: tokenXVault,
        tokenYVault: tokenYVault,
        systemProgram: SystemProgram.programId,
        tokenProgram: TOKEN_PROGRAM_ID,
        associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        rent: SYSVAR_RENT_PUBKEY,
      })
      .rpc();
    
    console.log("Initialize pool transaction signature", tx);

    // Verify pool state
    const poolAccount = await program.account.pool.fetch(poolPda);
    expect(poolAccount.authority.toString()).to.equal(wallet.publicKey.toString());
    expect(poolAccount.tokenXMint.toString()).to.equal(tokenXMint.toString());
    expect(poolAccount.tokenYMint.toString()).to.equal(tokenYMint.toString());
    expect(poolAccount.tokenXVault.toString()).to.equal(tokenXVault.toString());
    expect(poolAccount.tokenYVault.toString()).to.equal(tokenYVault.toString());
    expect(poolAccount.tokenXAmount.toNumber()).to.equal(0);
    expect(poolAccount.tokenYAmount.toNumber()).to.equal(0);
    expect(poolAccount.bump).to.equal(poolBump);
  });

  it("Adds initial liquidity to the pool", async () => {
    // Add initial liquidity
    const tx = await program.methods
      .addLiquidity(
        new BN(INITIAL_POOL_LIQUIDITY), // amountXDesired
        new BN(INITIAL_POOL_LIQUIDITY), // amountYDesired
        new BN(INITIAL_POOL_LIQUIDITY), // amountXMin
        new BN(INITIAL_POOL_LIQUIDITY)  // amountYMin
      )
      .accounts({
        provider: wallet.publicKey,
        pool: poolPda,
        tokenXVault: tokenXVault,
        tokenYVault: tokenYVault,
        providerTokenX: tokenXAccount,
        providerTokenY: tokenYAccount,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .rpc();
    
    console.log("Add liquidity transaction signature", tx);

    // Verify pool reserves increased
    const poolAccount = await program.account.pool.fetch(poolPda);
    expect(poolAccount.tokenXAmount.toNumber()).to.equal(INITIAL_POOL_LIQUIDITY);
    expect(poolAccount.tokenYAmount.toNumber()).to.equal(INITIAL_POOL_LIQUIDITY);

    // Verify wallet tokens decreased
    const tokenXAccountInfo = await getAccount(provider.connection, tokenXAccount);
    const tokenYAccountInfo = await getAccount(provider.connection, tokenYAccount);
    expect(Number(tokenXAccountInfo.amount)).to.equal(INITIAL_TOKEN_AMOUNT - INITIAL_POOL_LIQUIDITY);
    expect(Number(tokenYAccountInfo.amount)).to.equal(INITIAL_TOKEN_AMOUNT - INITIAL_POOL_LIQUIDITY);
  });

  it("Swaps token X for token Y", async () => {
    // Get initial balances
    const initialTokenXAccountInfo = await getAccount(provider.connection, tokenXAccount);
    const initialTokenYAccountInfo = await getAccount(provider.connection, tokenYAccount);
    const initialTokenXBalance = Number(initialTokenXAccountInfo.amount);
    const initialTokenYBalance = Number(initialTokenYAccountInfo.amount);

    // Get initial pool state
    const initialPoolAccount = await program.account.pool.fetch(poolPda);
    const initialPoolXAmount = initialPoolAccount.tokenXAmount.toNumber();
    const initialPoolYAmount = initialPoolAccount.tokenYAmount.toNumber();

    // Calculate expected output amount based on constant product formula
    // output_amount = (reserve_out * input_amount) / (reserve_in + input_amount)
    const expectedOutputAmount = Math.floor(
      (initialPoolYAmount * SWAP_AMOUNT) / (initialPoolXAmount + SWAP_AMOUNT)
    );
    
    console.log(`Expected output amount from X->Y swap: ${expectedOutputAmount}`);

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
        tokenXVault: tokenXVault,
        tokenYVault: tokenYVault,
        userTokenIn: tokenXAccount,
        userTokenOut: tokenYAccount,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .rpc();
    
    console.log("Swap X->Y transaction signature", tx);

    // Verify balances after swap
    const finalTokenXAccountInfo = await getAccount(provider.connection, tokenXAccount);
    const finalTokenYAccountInfo = await getAccount(provider.connection, tokenYAccount);
    const finalTokenXBalance = Number(finalTokenXAccountInfo.amount);
    const finalTokenYBalance = Number(finalTokenYAccountInfo.amount);

    // Verify pool state
    const finalPoolAccount = await program.account.pool.fetch(poolPda);
    const finalPoolXAmount = finalPoolAccount.tokenXAmount.toNumber();
    const finalPoolYAmount = finalPoolAccount.tokenYAmount.toNumber();

    // Check user tokens changed correctly
    expect(finalTokenXBalance).to.equal(initialTokenXBalance - SWAP_AMOUNT);
    expect(finalTokenYBalance).to.be.above(initialTokenYBalance);
    expect(finalTokenYBalance - initialTokenYBalance).to.be.at.least(minOutputAmount);

    // Check pool reserves changed correctly
    expect(finalPoolXAmount).to.equal(initialPoolXAmount + SWAP_AMOUNT);
    expect(finalPoolYAmount).to.equal(initialPoolYAmount - (finalTokenYBalance - initialTokenYBalance));

    // Verify constant product formula is maintained (with small rounding difference allowed)
    const initialK = initialPoolXAmount * initialPoolYAmount;
    const finalK = finalPoolXAmount * finalPoolYAmount;
    
    // Allow for very small rounding differences
    const kDiffRatio = Math.abs(finalK - initialK) / initialK;
    expect(kDiffRatio).to.be.lessThan(0.0001); // 0.01% tolerance for rounding
  });

  it("Swaps token Y for token X", async () => {
    // Get initial balances
    const initialTokenXAccountInfo = await getAccount(provider.connection, tokenXAccount);
    const initialTokenYAccountInfo = await getAccount(provider.connection, tokenYAccount);
    const initialTokenXBalance = Number(initialTokenXAccountInfo.amount);
    const initialTokenYBalance = Number(initialTokenYAccountInfo.amount);

    // Get initial pool state
    const initialPoolAccount = await program.account.pool.fetch(poolPda);
    const initialPoolXAmount = initialPoolAccount.tokenXAmount.toNumber();
    const initialPoolYAmount = initialPoolAccount.tokenYAmount.toNumber();

    // Calculate expected output amount based on constant product formula
    // output_amount = (reserve_out * input_amount) / (reserve_in + input_amount)
    const expectedOutputAmount = Math.floor(
      (initialPoolXAmount * SWAP_AMOUNT) / (initialPoolYAmount + SWAP_AMOUNT)
    );
    
    console.log(`Expected output amount from Y->X swap: ${expectedOutputAmount}`);

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
        tokenXVault: tokenXVault,
        tokenYVault: tokenYVault,
        userTokenIn: tokenYAccount,
        userTokenOut: tokenXAccount,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .rpc();
    
    console.log("Swap Y->X transaction signature", tx);

    // Verify balances after swap
    const finalTokenXAccountInfo = await getAccount(provider.connection, tokenXAccount);
    const finalTokenYAccountInfo = await getAccount(provider.connection, tokenYAccount);
    const finalTokenXBalance = Number(finalTokenXAccountInfo.amount);
    const finalTokenYBalance = Number(finalTokenYAccountInfo.amount);

    // Verify pool state
    const finalPoolAccount = await program.account.pool.fetch(poolPda);
    const finalPoolXAmount = finalPoolAccount.tokenXAmount.toNumber();
    const finalPoolYAmount = finalPoolAccount.tokenYAmount.toNumber();

    // Check user tokens changed correctly
    expect(finalTokenYBalance).to.equal(initialTokenYBalance - SWAP_AMOUNT);
    expect(finalTokenXBalance).to.be.above(initialTokenXBalance);
    expect(finalTokenXBalance - initialTokenXBalance).to.be.at.least(minOutputAmount);

    // Check pool reserves changed correctly
    expect(finalPoolYAmount).to.equal(initialPoolYAmount + SWAP_AMOUNT);
    expect(finalPoolXAmount).to.equal(initialPoolXAmount - (finalTokenXBalance - initialTokenXBalance));

    // Verify constant product formula is maintained (with small rounding difference allowed)
    const initialK = initialPoolXAmount * initialPoolYAmount;
    const finalK = finalPoolXAmount * finalPoolYAmount;
    
    // Allow for very small rounding differences
    const kDiffRatio = Math.abs(finalK - initialK) / initialK;
    expect(kDiffRatio).to.be.lessThan(0.0001); // 0.01% tolerance for rounding
  });

  it("Adds more liquidity to the pool at the correct ratio", async () => {
    // Get current pool state to calculate the correct ratio
    const poolAccount = await program.account.pool.fetch(poolPda);
    const poolXAmount = poolAccount.tokenXAmount.toNumber();
    const poolYAmount = poolAccount.tokenYAmount.toNumber();
    
    // Calculate amounts to add (we'll add 20% more liquidity)
    const additionalLiquidityPercentage = 0.2; // 20%
    const additionalXAmount = Math.floor(poolXAmount * additionalLiquidityPercentage);
    const additionalYAmount = Math.floor(poolYAmount * additionalLiquidityPercentage);
    
    // Get initial balances
    const initialTokenXAccountInfo = await getAccount(provider.connection, tokenXAccount);
    const initialTokenYAccountInfo = await getAccount(provider.connection, tokenYAccount);
    const initialTokenXBalance = Number(initialTokenXAccountInfo.amount);
    const initialTokenYBalance = Number(initialTokenYAccountInfo.amount);
    
    console.log(`Adding more liquidity: ${additionalXAmount} X and ${additionalYAmount} Y`);

    // Allow 0.5% slippage
    const minXAmount = Math.floor(additionalXAmount * 0.995);
    const minYAmount = Math.floor(additionalYAmount * 0.995);

    // Add more liquidity
    const tx = await program.methods
      .addLiquidity(
        new BN(additionalXAmount), // amountXDesired
        new BN(additionalYAmount), // amountYDesired
        new BN(minXAmount),        // amountXMin
        new BN(minYAmount)         // amountYMin
      )
      .accounts({
        provider: wallet.publicKey,
        pool: poolPda,
        tokenXVault: tokenXVault,
        tokenYVault: tokenYVault,
        providerTokenX: tokenXAccount,
        providerTokenY: tokenYAccount,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .rpc();
    
    console.log("Add additional liquidity transaction signature", tx);

    // Verify pool reserves increased
    const finalPoolAccount = await program.account.pool.fetch(poolPda);
    const finalPoolXAmount = finalPoolAccount.tokenXAmount.toNumber();
    const finalPoolYAmount = finalPoolAccount.tokenYAmount.toNumber();
    
    expect(finalPoolXAmount).to.be.above(poolXAmount);
    expect(finalPoolYAmount).to.be.above(poolYAmount);
    
    // Verify wallet tokens decreased
    const finalTokenXAccountInfo = await getAccount(provider.connection, tokenXAccount);
    const finalTokenYAccountInfo = await getAccount(provider.connection, tokenYAccount);
    const finalTokenXBalance = Number(finalTokenXAccountInfo.amount);
    const finalTokenYBalance = Number(finalTokenYAccountInfo.amount);
    
    expect(initialTokenXBalance - finalTokenXBalance).to.be.at.least(minXAmount);
    expect(initialTokenYBalance - finalTokenYBalance).to.be.at.least(minYAmount);
    
    // Verify the ratio is maintained in pool reserves
    const initialRatio = poolXAmount / poolYAmount;
    const finalRatio = finalPoolXAmount / finalPoolYAmount;
    
    // Allow for small rounding differences
    const ratioDiff = Math.abs(finalRatio - initialRatio) / initialRatio;
    expect(ratioDiff).to.be.lessThan(0.001); // 0.1% tolerance for rounding
  });
}); 