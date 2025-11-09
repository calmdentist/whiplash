// Migrations are an early feature. Currently, they're nothing more than this
// single deploy script that's invoked from the CLI, injecting a provider
// configured from the workspace's Anchor.toml.

import * as anchor from "@coral-xyz/anchor";
import { Program, Idl } from "@coral-xyz/anchor";
import { PublicKey, SystemProgram, SYSVAR_RENT_PUBKEY, Keypair } from "@solana/web3.js";
import { TOKEN_PROGRAM_ID, ASSOCIATED_TOKEN_PROGRAM_ID } from "@solana/spl-token";
import * as fs from "fs";
import * as path from "path";
import { execSync } from "child_process";

// Define the Metaplex Token Metadata Program ID
const TOKEN_METADATA_PROGRAM_ID = new PublicKey("metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s");

async function main() {
  try {
    console.log("Starting deployment script...");
    
    // Get the provider from the environment
    const provider = anchor.AnchorProvider.env();
    console.log("Provider wallet:", provider.wallet.publicKey.toString());
    
    // Configure client to use the provider.
    anchor.setProvider(provider);

    // // Deploy Metaplex Token Metadata program first
    // console.log("Reading Metaplex program file...");
    // const metaplexProgramPath = path.join(__dirname, "../tests/metaplex_token_metadata_program.so");
    // console.log("Metaplex program path:", metaplexProgramPath);
    
    // if (!fs.existsSync(metaplexProgramPath)) {
    //   throw new Error(`Metaplex program file not found at ${metaplexProgramPath}`);
    // }
    
    // console.log("Metaplex program file found");
    
    // console.log("Deploying Metaplex Token Metadata program...");
    console.log("Requesting airdrop for provider wallet...");
    const airdropSig = await provider.connection.requestAirdrop(provider.wallet.publicKey, 10000000000);
    await provider.connection.confirmTransaction(airdropSig);
    console.log("Airdrop confirmed for provider wallet");

    // // Generate a new keypair for the Metaplex program
    // const metaplexKeypair = Keypair.generate();
    // console.log("Generated new keypair for Metaplex program:", metaplexKeypair.publicKey.toString());

    // // Save the keypair to a file
    // const keypairPath = path.join(__dirname, "../target/deploy/metaplex-keypair.json");
    // fs.writeFileSync(keypairPath, JSON.stringify(Array.from(metaplexKeypair.secretKey)));
    // console.log("Saved Metaplex keypair to:", keypairPath);

    // // Deploy Metaplex program using solana program deploy
    // const metaplexProgramId = metaplexKeypair.publicKey;
    // console.log("Deploying Metaplex program...");
    // const { execSync } = require('child_process');
    // execSync(`solana program deploy ${metaplexProgramPath} --program-id ${keypairPath} --url localhost`, { stdio: 'inherit' });
    // console.log("Metaplex program deployed!");

    // // Wait for deployment to be confirmed
    // console.log("Waiting for Metaplex program deployment to be confirmed...");
    // await new Promise(resolve => setTimeout(resolve, 5000)); // Wait 5 seconds

    // // Verify Metaplex program is deployed
    // const metaplexProgramInfo = await provider.connection.getAccountInfo(metaplexProgramId);
    // if (!metaplexProgramInfo || !metaplexProgramInfo.executable) {
    //   throw new Error("Metaplex program is not deployed or not executable");
    // }
    // console.log("Metaplex program deployment verified");

    // Deploy Facemelt program
    console.log("Deploying Facemelt program...");
    execSync('anchor deploy', { stdio: 'inherit' });
    console.log("Facemelt program deployed successfully!");

    // Wait for deployment to be confirmed
    console.log("Waiting for Facemelt program deployment to be confirmed...");
    await new Promise(resolve => setTimeout(resolve, 5000)); // Wait 5 seconds

    // Initialize Facemelt program client
    console.log("Initializing Facemelt program client...");
    
    // Load the IDL
    const idlPath = path.join(__dirname, "../target/idl/facemelt.json");
    if (!fs.existsSync(idlPath)) {
      throw new Error(`IDL file not found at ${idlPath}`);
    }
    const idl = JSON.parse(fs.readFileSync(idlPath, "utf8"));
    
    // Get program ID from IDL (this is the actual deployed program ID)
    const facemeltProgramId = new PublicKey(idl.metadata.address);
    console.log("Program ID from IDL:", facemeltProgramId.toString());
    
    // Verify Facemelt program is deployed
    const facemeltProgramInfo = await provider.connection.getAccountInfo(facemeltProgramId);
    if (!facemeltProgramInfo || !facemeltProgramInfo.executable) {
      throw new Error("Facemelt program is not deployed or not executable");
    }
    console.log("Facemelt program deployment verified");
    
    const facemeltProgram = new Program(
      idl,
      facemeltProgramId,
      provider
    );
    console.log("Facemelt program client initialized");

    // Launch SHIB token
    console.log("Launching SHIB token...");
    const virtualSolReserve = new anchor.BN(100000000000); // 100 SOL in lamports
    const tokenName = "SHIBA INU";
    const tokenTicker = "SHIB";
    const metadataUri = "https://ipfs.io/ipfs/QmVySXmdq9qNG7H98tW5v8KTSUqPsLBYfo3EaKgR2shJex";

    // Create token mint
    const tokenMint = anchor.web3.Keypair.generate();
    console.log("Token mint created:", tokenMint.publicKey.toString());

    // Find pool PDA
    const [poolPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("pool"), tokenMint.publicKey.toBuffer()],
      facemeltProgramId
    );
    console.log("Pool PDA:", poolPda.toString());

    // Find token vault PDA
    const [tokenVaultPda] = PublicKey.findProgramAddressSync(
      [
        poolPda.toBuffer(),
        TOKEN_PROGRAM_ID.toBuffer(),
        tokenMint.publicKey.toBuffer(),
      ],
      ASSOCIATED_TOKEN_PROGRAM_ID
    );
    console.log("Token vault PDA:", tokenVaultPda.toString());

    // Find metadata PDA
    const [metadataPda] = PublicKey.findProgramAddressSync(
      [
        Buffer.from("metadata"),
        TOKEN_METADATA_PROGRAM_ID.toBuffer(),
        tokenMint.publicKey.toBuffer(),
      ],
      TOKEN_METADATA_PROGRAM_ID
    );
    console.log("Metadata PDA:", metadataPda.toString());

    // Optional parameters for pool configuration
    // Pass null to use defaults:
    // - funding_constant_c: PRECISION / 10000 = 0.0001/sec
    // - liquidation_divergence_threshold: 10 (10%)
    const fundingConstantC = null; // Use default
    const liquidationDivergenceThreshold = null; // Use default
    
    const tx = await facemeltProgram.methods
      .launch(
        virtualSolReserve,
        tokenName,
        tokenTicker,
        metadataUri,
        fundingConstantC,
        liquidationDivergenceThreshold
      )
      .accounts({
        authority: provider.wallet.publicKey,
        tokenMint: tokenMint.publicKey,
        pool: poolPda,
        tokenVault: tokenVaultPda,
        metadata: metadataPda,
        systemProgram: SystemProgram.programId,
        tokenProgram: TOKEN_PROGRAM_ID,
        associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        rent: SYSVAR_RENT_PUBKEY,
        tokenMetadataProgram: TOKEN_METADATA_PROGRAM_ID,
      })
      .signers([tokenMint])
      .rpc();
    
    console.log("SHIB token launched successfully!");
    console.log("Transaction signature:", tx);
  } catch (error) {
    console.error("Deployment failed with error:", error);
    throw error;
  }
}

main().then(
  () => process.exit(0),
  (err) => {
    console.error(err);
    process.exit(1);
  }
);
