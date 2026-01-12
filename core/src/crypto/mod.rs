use rand::RngCore;
use rand::rngs::OsRng;
use secp256k1::{Message, PublicKey, Secp256k1, SecretKey};
use sha2::{Digest, Sha256};
use tiny_keccak::{Hasher, Keccak};

pub struct WalletKeypair {
    pub secret_key: SecretKey,
    pub public_key: PublicKey,
}

impl WalletKeypair {
    pub fn new() -> Self {
        let secp = Secp256k1::new();
        let mut rng = OsRng;
        let mut secret_bytes = [0u8; 32];
        rng.fill_bytes(&mut secret_bytes);

        let secret_key =
            SecretKey::from_slice(&secret_bytes).expect("32 bytes, within curve order");
        let public_key = PublicKey::from_secret_key(&secp, &secret_key);

        Self {
            secret_key,
            public_key,
        }
    }

    pub fn sign(&self, msg: &[u8]) -> [u8; 64] {
        let secp = Secp256k1::new();
        let msg_hash = Sha256::digest(msg);
        let message = Message::from_digest_slice(&msg_hash).expect("32 byte hash");
        let sig = secp.sign_ecdsa(&message, &self.secret_key);
        sig.serialize_compact()
    }

    pub fn address(&self) -> String {
        eth_address_from_public_key(&self.public_key)
    }

    pub fn secret_hex(&self) -> String {
        hex::encode(self.secret_key.secret_bytes())
    }

    pub fn public_hex(&self) -> String {
        hex::encode(self.public_key.serialize_uncompressed())
    }
}

/// Generate Ethereum-style address from public key (with 0x prefix)
pub fn eth_address_from_public_key(pubkey: &PublicKey) -> String {
    let pubkey_bytes = pubkey.serialize_uncompressed();
    let hash = keccak256(&pubkey_bytes[1..]); // Skip 0x04 prefix
    format!("0x{}", hex::encode(&hash[12..32])) // Last 20 bytes with 0x prefix
}

pub fn keccak256(data: &[u8]) -> [u8; 32] {
    let mut hasher = Keccak::v256();
    let mut output = [0u8; 32];
    hasher.update(data);
    hasher.finalize(&mut output);
    output
}

pub fn verify_signature(pubkey_hex: &str, msg: &[u8], sig_bytes: &[u8]) -> bool {
    let secp = Secp256k1::new();

    let pubkey_bytes = match hex::decode(pubkey_hex) {
        Ok(bytes) => bytes,
        Err(_) => return false,
    };

    let pubkey = match PublicKey::from_slice(&pubkey_bytes) {
        Ok(pk) => pk,
        Err(_) => return false,
    };

    let msg_hash = Sha256::digest(msg);
    let message = match Message::from_digest_slice(&msg_hash) {
        Ok(m) => m,
        Err(_) => return false,
    };

    let sig = match secp256k1::ecdsa::Signature::from_compact(sig_bytes) {
        Ok(s) => s,
        Err(_) => return false,
    };

    secp.verify_ecdsa(&message, &sig, &pubkey).is_ok()
}
