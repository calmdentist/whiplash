import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { Whiplash } from "../target/types/whiplash";
import { expect } from "chai";
import {
  PublicKey, 
  Keypair, 
  SystemProgram,
  SYSVAR_RENT_PUBKEY,
  LAMPORTS_PER_SOL,
} from "@solana/web3.js";
import {
  TOKEN_PROGRAM_ID,
  ASSOCIATED_TOKEN_PROGRAM_ID,
  getAssociatedTokenAddress,
  getAccount,
  createAssociatedTokenAccountInstruction,
  createTransferInstruction
} from "@solana/spl-token";
import BN from "bn.js";

// Define the Metaplex Token Metadata Program ID
const TOKEN_METADATA_PROGRAM_ID = new PublicKey("metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s");

describe("whiplash", () => {
  // Configure the client to use the local cluster
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.Whiplash as Program<Whiplash>;
  const wallet = provider.wallet as anchor.Wallet;

  // Test state
  const tokenMintKeypair = Keypair.generate();
  let tokenMint: PublicKey;
  let tokenAccount: PublicKey;
  let poolPda: PublicKey;
  let poolBump: number;
  let tokenVault: PublicKey;
  let metadataPda: PublicKey;
  
  // Position state for leverage tests
  let positionPda: PublicKey;
  let positionBump: number;
  let positionTokenAccount: PublicKey;

  // Pool initial values
  const INITIAL_TOKEN_AMOUNT = 1_000_000 * 10 ** 6; // 1 million tokens with 6 decimals
  const VIRTUAL_SOL_RESERVE = 1_000 * LAMPORTS_PER_SOL; // 1,000 SOL (in lamports)
  const DECIMALS = 6;
  const METADATA_URI = "https://ipfs.io/ipfs/QmVySXmdq9qNG7H98tW5v8KTSUqPsLBYfo3EaKgR2shJex";
  const SWAP_AMOUNT = 1 * LAMPORTS_PER_SOL; // 1 SOL
  const TOKEN_SWAP_AMOUNT = 100 * 10 ** 6; // 100 tokens with 6 decimals
  const LEVERAGE = 5; // 5x leverage

  before(async () => {
    // Use the keypair's public key
    tokenMint = tokenMintKeypair.publicKey;
    console.log("Token Mint Pubkey:", tokenMint.toBase58());

    // Calculate token account address - will be created after initialization
    tokenAccount = await getAssociatedTokenAddress(
      tokenMint,
      wallet.publicKey
    );
    console.log("Token Account will be:", tokenAccount.toBase58());

    // Derive the pool PDA
    [poolPda, poolBump] = await PublicKey.findProgramAddressSync(
      [
        Buffer.from("pool"),
        tokenMint.toBuffer(),
      ],
      program.programId
    );
    console.log("Pool PDA:", poolPda.toBase58(), "with bump:", poolBump);

    // Calculate the token vault address
    tokenVault = await getAssociatedTokenAddress(
      tokenMint,
      poolPda,
      true // allowOwnerOffCurve
    );
    
    console.log("Token Vault:", tokenVault.toBase58());

    // Derive the metadata PDA
    [metadataPda] = PublicKey.findProgramAddressSync(
      [
        Buffer.from("metadata"),
        TOKEN_METADATA_PROGRAM_ID.toBuffer(),
        tokenMint.toBuffer(),
      ],
      TOKEN_METADATA_PROGRAM_ID
    );
    console.log("Metadata PDA:", metadataPda.toBase58());
    
    // Derive the position PDA for leverage swap tests
    [positionPda, positionBump] = await PublicKey.findProgramAddressSync(
      [
        Buffer.from("position"),
        poolPda.toBuffer(),
        wallet.publicKey.toBuffer(),
      ],
      program.programId
    );
    console.log("Position PDA:", positionPda.toBase58(), "with bump:", positionBump);
    
    // Calculate the position token account address
    positionTokenAccount = await getAssociatedTokenAddress(
      tokenMint,
      positionPda,
      true // allowOwnerOffCurve
    );
    console.log("Position Token Account:", positionTokenAccount.toBase58());
  });

  it("Initializes a pool", async () => {

    try {
      // Initialize the pool
      const tx = await program.methods
        .launch(
          new BN(VIRTUAL_SOL_RESERVE),
          "Test Token",
          "TEST",
          METADATA_URI
        )
        .accounts({
          authority: wallet.publicKey,
          tokenMint: tokenMint,
          pool: poolPda,
          tokenVault: tokenVault,
          metadata: metadataPda,
          systemProgram: SystemProgram.programId,
          tokenProgram: TOKEN_PROGRAM_ID,
          associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
          rent: SYSVAR_RENT_PUBKEY,
          tokenMetadataProgram: TOKEN_METADATA_PROGRAM_ID,
        })
        .signers([wallet.payer, tokenMintKeypair])
        .rpc();
      
      console.log("Initialize pool transaction signature", tx);

      // Wait for confirmation
      await provider.connection.confirmTransaction(tx);

      // Create token account for wallet after minting tokens
      await provider.connection.confirmTransaction(
        await provider.connection.requestAirdrop(
          wallet.publicKey,
          0.1 * LAMPORTS_PER_SOL
        )
      );

      // Create associated token account for the user
      const createATAIx = createAssociatedTokenAccountInstruction(
        wallet.publicKey,
        tokenAccount,
        wallet.publicKey,
        tokenMint
      );

      const transaction = new anchor.web3.Transaction().add(
        createATAIx
      );
      
      await provider.sendAndConfirm(transaction);
      console.log("Created user token account");

      // Verify pool state
      const poolAccount = await program.account.pool.fetch(poolPda);
      expect(poolAccount.authority.toString()).to.equal(wallet.publicKey.toString());
      expect(poolAccount.tokenYMint.toString()).to.equal(tokenMint.toString());
      expect(poolAccount.tokenYVault.toString()).to.equal(tokenVault.toString());
      
      // If VIRTUAL_SOL_RESERVE was changed for testing, update the expectation
      expect(poolAccount.virtualSolAmount.toNumber()).to.equal(VIRTUAL_SOL_RESERVE);
      expect(poolAccount.bump).to.equal(poolBump);
    } catch (error) {
      console.error("Launch Error:", error);
      throw error;
    }
  });

  it("Swaps SOL for tokens", async () => {
    try {
      // Get initial balances
      const initialTokenAccountInfo = await getAccount(provider.connection, tokenAccount);
      const initialTokenBalance = Number(initialTokenAccountInfo.amount);

      // Get initial pool state
      const initialPoolAccount = await program.account.pool.fetch(poolPda);
      const initialPoolTokenAmount = initialPoolAccount.tokenYAmount.toNumber();
      const initialVirtualSolAmount = initialPoolAccount.virtualSolAmount.toNumber();

      // Calculate expected output amount based on constant product formula
      // output_amount = (reserve_out * input_amount) / (reserve_in + input_amount)
      const expectedOutputAmount = Math.floor(
        (initialPoolTokenAmount * SWAP_AMOUNT) / (initialVirtualSolAmount + SWAP_AMOUNT)
      );
      
      console.log(`Expected output amount from SOL->Token swap: ${expectedOutputAmount}`);

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
          tokenYVault: tokenVault,
          userTokenIn: wallet.publicKey, // For SOL swap, this is just the user's wallet
          userTokenOut: tokenAccount,
          tokenProgram: TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
        })
        .rpc();
      
      console.log("Swap SOL->Token transaction signature", tx);

      // Wait for confirmation
      await provider.connection.confirmTransaction(tx);

      // Verify balances after swap
      const finalTokenAccountInfo = await getAccount(provider.connection, tokenAccount);
      const finalTokenBalance = Number(finalTokenAccountInfo.amount);

      // Verify pool state
      const finalPoolAccount = await program.account.pool.fetch(poolPda);
      const finalPoolTokenAmount = finalPoolAccount.tokenYAmount.toNumber();
      const finalPoolLamports = finalPoolAccount.lamports.toNumber();
      const finalVirtualSolAmount = finalPoolAccount.virtualSolAmount.toNumber();

      // Check user tokens changed correctly
      expect(finalTokenBalance).to.be.above(initialTokenBalance);
      expect(finalTokenBalance - initialTokenBalance).to.be.at.least(minOutputAmount);

      // Check pool reserves changed correctly
      expect(finalPoolTokenAmount).to.equal(initialPoolTokenAmount - (finalTokenBalance - initialTokenBalance));
      // In a SOL->Token swap, the SOL goes to the pool's lamports, not virtual amount
      expect(finalPoolLamports).to.equal(SWAP_AMOUNT);
      // Virtual SOL amount should remain unchanged
      expect(finalVirtualSolAmount).to.equal(initialVirtualSolAmount);

      // Verify constant product formula is maintained (with small rounding difference allowed)
      // Use total reserves (real + virtual) for the constant product check
      const initialTotalSol = initialVirtualSolAmount; // Initially no real SOL
      const initialTotalTokens = initialPoolTokenAmount;
      
      const finalTotalSol = finalVirtualSolAmount + finalPoolLamports;
      const finalTotalTokens = finalPoolTokenAmount;
      
      const initialK = initialTotalSol * initialTotalTokens;
      const finalK = finalTotalSol * finalTotalTokens;
      
      // Allow for very small rounding differences
      const kDiffRatio = Math.abs(finalK - initialK) / initialK;
      expect(kDiffRatio).to.be.lessThan(0.0001); // 0.01% tolerance for rounding
    } catch (error) {
      console.error("Swap Error:", error);
      throw error;
    }
  });
  
  it("Swaps tokens for SOL", async () => {
    try {
      // Get initial balances
      const initialTokenAccountInfo = await getAccount(provider.connection, tokenAccount);
      const initialTokenBalance = Number(initialTokenAccountInfo.amount);
      const initialUserSolBalance = await provider.connection.getBalance(wallet.publicKey);

      // Get initial pool state
      const initialPoolAccount = await program.account.pool.fetch(poolPda);
      const initialPoolTokenAmount = initialPoolAccount.tokenYAmount.toNumber();
      const initialPoolLamports = initialPoolAccount.lamports.toNumber();
      const initialVirtualSolAmount = initialPoolAccount.virtualSolAmount.toNumber();
      const initialTotalSolAmount = initialVirtualSolAmount + initialPoolLamports;

      // Calculate expected output amount based on constant product formula
      // output_amount = (reserve_out * input_amount) / (reserve_in + input_amount)
      const expectedOutputAmount = Math.floor(
        (initialTotalSolAmount * TOKEN_SWAP_AMOUNT) / (initialPoolTokenAmount + TOKEN_SWAP_AMOUNT)
      );
      
      console.log(`Expected output amount from Token->SOL swap: ${expectedOutputAmount}`);

      // Allow 1% slippage
      const minOutputAmount = Math.floor(expectedOutputAmount * 0.99);

      // Perform the swap
      const tx = await program.methods
        .swap(
          new BN(TOKEN_SWAP_AMOUNT),     // amountIn
          new BN(minOutputAmount)        // minAmountOut
        )
        .accounts({
          user: wallet.publicKey,
          pool: poolPda,
          tokenYVault: tokenVault,
          userTokenIn: tokenAccount,      // For token swap, this is the user's token account
          userTokenOut: wallet.publicKey, // For token->SOL swap, this is the user's wallet
          tokenProgram: TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
        })
        .rpc();
      
      console.log("Swap Token->SOL transaction signature", tx);

      // Wait for confirmation
      await provider.connection.confirmTransaction(tx);

      // Verify balances after swap
      const finalTokenAccountInfo = await getAccount(provider.connection, tokenAccount);
      const finalTokenBalance = Number(finalTokenAccountInfo.amount);
      const finalUserSolBalance = await provider.connection.getBalance(wallet.publicKey);

      // Verify pool state
      const finalPoolAccount = await program.account.pool.fetch(poolPda);
      const finalPoolTokenAmount = finalPoolAccount.tokenYAmount.toNumber();
      const finalPoolLamports = finalPoolAccount.lamports.toNumber();
      const finalVirtualSolAmount = finalPoolAccount.virtualSolAmount.toNumber();

      // Check user tokens changed correctly
      expect(finalTokenBalance).to.be.below(initialTokenBalance);
      expect(initialTokenBalance - finalTokenBalance).to.equal(TOKEN_SWAP_AMOUNT);
      
      // Check user SOL balance increased (accounting for gas fees)
      // We can't predict exact gas fees, so we check if it's at least minOutputAmount minus a buffer for gas
      const SOL_BALANCE_INCREASE = finalUserSolBalance - initialUserSolBalance;
      expect(SOL_BALANCE_INCREASE).to.be.at.least(minOutputAmount - 10000); // Buffer for gas fees

      // Check pool reserves changed correctly
      expect(finalPoolTokenAmount).to.equal(initialPoolTokenAmount + TOKEN_SWAP_AMOUNT);
      // In a Token->SOL swap, the SOL leaves the pool's lamports
      expect(finalPoolLamports).to.be.below(initialPoolLamports);
      expect(initialPoolLamports - finalPoolLamports).to.be.at.least(minOutputAmount);
      // Virtual SOL amount should remain unchanged
      expect(finalVirtualSolAmount).to.equal(initialVirtualSolAmount);

      // Verify constant product formula is maintained (with small rounding difference allowed)
      const initialK = initialTotalSolAmount * initialPoolTokenAmount;
      const finalTotalSol = finalVirtualSolAmount + finalPoolLamports;
      const finalK = finalTotalSol * finalPoolTokenAmount;
      
      // Allow for very small rounding differences
      const kDiffRatio = Math.abs(finalK - initialK) / initialK;
      expect(kDiffRatio).to.be.lessThan(0.0001); // 0.01% tolerance for rounding
    } catch (error) {
      console.error("Token->SOL Swap Error:", error);
      throw error;
    }
  });
  
  it("Performs a leverage swap SOL->Token (long position)", async () => {
    try {
      // Get initial balances
      const initialUserSolBalance = await provider.connection.getBalance(wallet.publicKey);

      // Get initial pool state
      const initialPoolAccount = await program.account.pool.fetch(poolPda);
      const initialPoolTokenAmount = initialPoolAccount.tokenYAmount.toNumber();
      const initialPoolLamports = initialPoolAccount.lamports.toNumber();
      const initialVirtualSolAmount = initialPoolAccount.virtualSolAmount.toNumber();
      const initialTotalSolAmount = initialVirtualSolAmount + initialPoolLamports;

      // Calculate expected output amount based on constant product formula with leverage
      // For leverage, multiply input amount by leverage/10 as per the code
      const leveragedAmount = SWAP_AMOUNT * LEVERAGE / 10;
      const expectedOutputAmount = Math.floor(
        (initialPoolTokenAmount * leveragedAmount) / (initialTotalSolAmount + leveragedAmount)
      );
      
      console.log(`Expected output amount from leveraged SOL->Token swap: ${expectedOutputAmount}`);

      // Allow 1% slippage
      const minOutputAmount = Math.floor(expectedOutputAmount * 0.99);

      // Perform the leveraged swap
      const tx = await program.methods
        .leverageSwap(
          new BN(SWAP_AMOUNT),       // amountIn (collateral)
          new BN(minOutputAmount),   // minAmountOut
          LEVERAGE                   // leverage factor
        )
        .accounts({
          user: wallet.publicKey,
          pool: poolPda,
          tokenYVault: tokenVault,
          userTokenIn: wallet.publicKey,  // For SOL swap, this is just the user's wallet
          userTokenOut: tokenAccount,     // Still need to specify this for the struct
          position: positionPda,
          positionTokenAccount: positionTokenAccount,
          positionTokenMint: tokenMint,   // The token that will be held in position
          tokenProgram: TOKEN_PROGRAM_ID,
          associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
          rent: SYSVAR_RENT_PUBKEY,
        })
        .rpc();
      
      console.log("Leverage Swap SOL->Token transaction signature", tx);

      // Wait for confirmation
      await provider.connection.confirmTransaction(tx);

      // Verify user SOL balance decreased by the collateral amount (plus fees)
      const finalUserSolBalance = await provider.connection.getBalance(wallet.publicKey);
      expect(initialUserSolBalance - finalUserSolBalance).to.be.at.least(SWAP_AMOUNT);

      // Verify position token account received the tokens
      const positionTokenAccountInfo = await getAccount(provider.connection, positionTokenAccount);
      const positionTokenBalance = Number(positionTokenAccountInfo.amount);
      expect(positionTokenBalance).to.be.at.least(minOutputAmount);

      // Verify position account data
      const positionAccount = await program.account.position.fetch(positionPda);
      expect(positionAccount.authority.toString()).to.equal(wallet.publicKey.toString());
      expect(positionAccount.pool.toString()).to.equal(poolPda.toString());
      expect(positionAccount.positionVault.toString()).to.equal(positionTokenAccount.toString());
      expect(positionAccount.isLong).to.be.true; // Should be a long position (SOL->Token)
      expect(positionAccount.collateral.toNumber()).to.equal(SWAP_AMOUNT);
      expect(positionAccount.leverage).to.equal(LEVERAGE);
      expect(positionAccount.size.toNumber()).to.equal(positionTokenBalance);

      // Verify pool state updated correctly
      const finalPoolAccount = await program.account.pool.fetch(poolPda);
      const finalPoolTokenAmount = finalPoolAccount.tokenYAmount.toNumber();
      const finalPoolLamports = finalPoolAccount.lamports.toNumber();
      
      // Pool should have received the collateral SOL
      expect(finalPoolLamports - initialPoolLamports).to.equal(SWAP_AMOUNT);
      
      // Pool tokens should have decreased by the amount sent to position
      expect(initialPoolTokenAmount - finalPoolTokenAmount).to.equal(positionTokenBalance);
      
      // Verify that the virtual SOL amount didn't change
      expect(finalPoolAccount.virtualSolAmount.toNumber()).to.equal(initialVirtualSolAmount);
    } catch (error) {
      console.error("Leverage Swap SOL->Token Error:", error);
      throw error;
    }
  });
  
  // Re-enable the short position test
  it("Performs a leverage swap Token->SOL (short position)", async () => {
    try {
      // Use a fixed keypair for the short position user
      const shortPositionUser = Keypair.generate();
      // Fund this account so it can pay for transactions
      await provider.connection.confirmTransaction(
        await provider.connection.requestAirdrop(
          shortPositionUser.publicKey,
          0.1 * LAMPORTS_PER_SOL
        )
      );

      // We need another position PDA for this test as we already used the one for the long position
      const [shortPositionPda, shortPositionBump] = await PublicKey.findProgramAddressSync(
        [
          Buffer.from("position"),
          poolPda.toBuffer(),
          shortPositionUser.publicKey.toBuffer(),
        ],
        program.programId
      );
      
      console.log("Short Position PDA:", shortPositionPda.toBase58());
      
      // Create token account for the short position user
      const shortPositionUserTokenAccount = await getAssociatedTokenAddress(
        tokenMint,
        shortPositionUser.publicKey
      );

      // For a short position (Token->SOL), we still need a token account for the position
      // due to how the program is designed, even though we're swapping to SOL
      const shortPositionTokenAccount = await getAssociatedTokenAddress(
        tokenMint,
        shortPositionPda,
        true // allowOwnerOffCurve
      );
      console.log("Short Position Token Account:", shortPositionTokenAccount.toBase58());
      
      // Create the token account for the short position user
      const createATAIx = createAssociatedTokenAccountInstruction(
        wallet.publicKey,
        shortPositionUserTokenAccount,
        shortPositionUser.publicKey,
        tokenMint
      );
      
      // Send some SOL to the short position user for transaction fees
      const transferTokenIx = anchor.web3.SystemProgram.transfer({
        fromPubkey: wallet.publicKey,
        toPubkey: shortPositionUser.publicKey,
        lamports: 0.05 * LAMPORTS_PER_SOL,
      });
      
      const setupTx = new anchor.web3.Transaction().add(
        transferTokenIx,
        createATAIx
      );
      
      await provider.sendAndConfirm(setupTx);
      
      // Transfer tokens from wallet's token account to short position user's token account
      const userTokenAccount = await getAssociatedTokenAddress(
        tokenMint,
        wallet.publicKey
      );
      
      const transferTokensTx = new anchor.web3.Transaction().add(
        createTransferInstruction(
          userTokenAccount,
          shortPositionUserTokenAccount,
          wallet.publicKey,
          TOKEN_SWAP_AMOUNT * 2 // Double the amount to ensure enough for test
        )
      );
      
      await provider.sendAndConfirm(transferTokensTx);
      console.log("Transferred tokens to short position user");
      
      // Get initial balances
      const initialTokenAccountInfo = await getAccount(provider.connection, shortPositionUserTokenAccount);
      const initialTokenBalance = Number(initialTokenAccountInfo.amount);
      
      // Get initial pool state
      const initialPoolAccount = await program.account.pool.fetch(poolPda);
      const initialPoolTokenAmount = initialPoolAccount.tokenYAmount.toNumber();
      const initialPoolLamports = initialPoolAccount.lamports.toNumber();
      const initialVirtualSolAmount = initialPoolAccount.virtualSolAmount.toNumber();
      const initialTotalSolAmount = initialVirtualSolAmount + initialPoolLamports;

      // Calculate expected output amount based on constant product formula with leverage
      const leveragedAmount = TOKEN_SWAP_AMOUNT * LEVERAGE / 10;
      const expectedOutputAmount = Math.floor(
        (initialTotalSolAmount * leveragedAmount) / (initialPoolTokenAmount + leveragedAmount)
      );
      
      console.log(`Expected output amount from leveraged Token->SOL swap: ${expectedOutputAmount}`);

      // Allow 1% slippage
      const minOutputAmount = Math.floor(expectedOutputAmount * 0.99);

      // Perform the leveraged swap
      const tx = await program.methods
        .leverageSwap(
          new BN(TOKEN_SWAP_AMOUNT),  // amountIn (collateral)
          new BN(minOutputAmount),    // minAmountOut
          LEVERAGE                    // leverage factor
        )
        .accounts({
          user: shortPositionUser.publicKey,
          pool: poolPda,
          tokenYVault: tokenVault,
          userTokenIn: shortPositionUserTokenAccount,      // Token account with Y tokens
          userTokenOut: shortPositionUser.publicKey,       // User wallet for receiving SOL
          position: shortPositionPda,
          positionTokenAccount: shortPositionTokenAccount, // Associated token account for the position
          positionTokenMint: tokenMint,                    // The token mint
          tokenProgram: TOKEN_PROGRAM_ID,
          associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
          rent: SYSVAR_RENT_PUBKEY,
        })
        .signers([shortPositionUser]) // Need to use the short position user as signer
        .rpc();
      
      console.log("Leverage Swap Token->SOL transaction signature", tx);

      // Wait for confirmation
      await provider.connection.confirmTransaction(tx);

      // Verify user token balance decreased by the collateral amount
      const finalTokenAccountInfo = await getAccount(provider.connection, shortPositionUserTokenAccount);
      const finalTokenBalance = Number(finalTokenAccountInfo.amount);
      expect(initialTokenBalance - finalTokenBalance).to.equal(TOKEN_SWAP_AMOUNT);

      // Verify position account was created
      const positionAccount = await program.account.position.fetch(shortPositionPda);
      expect(positionAccount.authority.toString()).to.equal(shortPositionUser.publicKey.toString());
      expect(positionAccount.pool.toString()).to.equal(poolPda.toString());
      expect(positionAccount.positionVault.toString()).to.equal(shortPositionTokenAccount.toString());
      expect(positionAccount.isLong).to.be.false; // Should be a short position (Token->SOL)
      expect(positionAccount.collateral.toNumber()).to.equal(TOKEN_SWAP_AMOUNT);
      expect(positionAccount.leverage).to.equal(LEVERAGE);
      expect(positionAccount.size.toNumber()).to.be.at.least(minOutputAmount);

      // Verify pool state updated correctly
      const finalPoolAccount = await program.account.pool.fetch(poolPda);
      const finalPoolTokenAmount = finalPoolAccount.tokenYAmount.toNumber();
      const finalPoolLamports = finalPoolAccount.lamports.toNumber();
      
      // Pool tokens should have increased by the collateral
      expect(finalPoolTokenAmount - initialPoolTokenAmount).to.equal(TOKEN_SWAP_AMOUNT);
      
      // Pool SOL should have decreased by the SOL sent to position
      expect(initialPoolLamports - finalPoolLamports).to.equal(positionAccount.size.toNumber());
      
      // Verify that the virtual SOL amount didn't change
      expect(finalPoolAccount.virtualSolAmount.toNumber()).to.equal(initialVirtualSolAmount);
      
      // For short positions, SOL is transferred to the user's wallet directly
      const finalUserSolBalance = await provider.connection.getBalance(shortPositionUser.publicKey);
      console.log("Position SOL amount:", positionAccount.size.toNumber());
      console.log("Final user SOL balance:", finalUserSolBalance);
    } catch (error) {
      console.error("Leverage Swap Token->SOL Error:", error);
      throw error;
    }
  });
}); 