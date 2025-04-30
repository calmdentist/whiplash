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
  createAssociatedTokenAccountInstruction
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

  // Pool initial values
  const INITIAL_TOKEN_AMOUNT = 1_000_000 * 10 ** 6; // 1 million tokens with 6 decimals
  const VIRTUAL_SOL_RESERVE = 1_000 * LAMPORTS_PER_SOL; // 1,000 SOL (in lamports)
  const DECIMALS = 6;
  const METADATA_URI = "https://ipfs.io/ipfs/QmVySXmdq9qNG7H98tW5v8KTSUqPsLBYfo3EaKgR2shJex";
  const SWAP_AMOUNT = 1 * LAMPORTS_PER_SOL; // 1 SOL

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
}); 