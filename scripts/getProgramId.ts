
import * as fs from 'fs';
import * as anchor from '@coral-xyz/anchor';

const keypairPath = './target/deploy/whiplash-keypair.json';

async function getProgramId() {
    try {
        const keypair = JSON.parse(fs.readFileSync(keypairPath, 'utf-8'));
        // The keypair is an array of numbers, which needs to be converted to a Uint8Array before creating a Keypair.
        const programId = anchor.web3.Keypair.fromSecretKey(Uint8Array.from(keypair)).publicKey;
        console.log('Program ID:', programId.toBase58());
        return programId.toBase58();
    } catch (error) {
        console.error('Error getting program ID:', error);
        return null;
    }
}

getProgramId();
