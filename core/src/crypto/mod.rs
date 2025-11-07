use ed25519_dalek::{Keypair, Signer, Verifier, Signature};
use rand::rngs::OsRng;


pub struct WalletKeypair {
    pub keypair: Keypair,
}


impl WalletKeypair {
    pub fn new() -> Self {
        let mut csprng = OsRng;
        Self { keypair: Keypair::generate(&mut csprng) }
    }


    pub fn sign(&self, msg: &[u8]) -> Signature {
        self.keypair.sign(msg)
    }
}
