use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand::rngs::OsRng;
use sha2::{Digest, Sha256};

pub struct WalletKeypair {
    pub signing_key: SigningKey,
    pub verifying_key: VerifyingKey,
}

impl WalletKeypair {
    pub fn new() -> Self {
        let mut rng = OsRng;
        let signing_key = SigningKey::generate(&mut rng);
        let verifying_key = signing_key.verifying_key();

        Self {
            signing_key,
            verifying_key,
        }
    }

    pub fn sign(&self, msg: &[u8]) -> [u8; 64] {
        let msg_hash = Sha256::digest(msg);
        let signature = self.signing_key.sign(&msg_hash);
        signature.to_bytes()
    }

    pub fn address(&self) -> String {
        address_from_public_key(&self.verifying_key)
    }

    pub fn secret_hex(&self) -> String {
        hex::encode(self.signing_key.to_bytes())
    }

    pub fn public_hex(&self) -> String {
        hex::encode(self.verifying_key.to_bytes())
    }

    pub fn from_secret_hex(hex_str: &str) -> Result<Self, String> {
        let secret_bytes = hex::decode(hex_str).map_err(|e| format!("invalid hex: {}", e))?;
        if secret_bytes.len() != 32 {
            return Err("secret key must be 32 bytes".to_string());
        }
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&secret_bytes);
        let signing_key = SigningKey::from_bytes(&bytes);
        let verifying_key = signing_key.verifying_key();
        
        Ok(Self {
            signing_key,
            verifying_key,
        })
    }
}

/// Generate address from Ed25519 public key (SHA256 hash with 0x prefix)
pub fn address_from_public_key(pubkey: &VerifyingKey) -> String {
    let pubkey_bytes = pubkey.to_bytes();
    let hash = Sha256::digest(&pubkey_bytes);
    // Use first 20 bytes like Ethereum for compatibility with existing address format
    format!("0x{}", hex::encode(&hash[..20]))
}

/// Generate address from hex-encoded public key string
pub fn address_from_pubkey_hex(pubkey_hex: &str) -> Result<String, String> {
    let pubkey_bytes = hex::decode(pubkey_hex).map_err(|e| format!("invalid hex: {}", e))?;
    if pubkey_bytes.len() != 32 {
        return Err("public key must be 32 bytes".to_string());
    }
    
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(&pubkey_bytes);
    let pubkey = VerifyingKey::from_bytes(&bytes)
        .map_err(|e| format!("invalid public key: {}", e))?;
    
    Ok(address_from_public_key(&pubkey))
}

pub fn verify_signature(pubkey_hex: &str, msg: &[u8], sig_bytes: &[u8]) -> bool {
    if sig_bytes.len() != 64 {
        return false;
    }

    let pubkey_bytes = match hex::decode(pubkey_hex) {
        Ok(bytes) => bytes,
        Err(_) => return false,
    };

    if pubkey_bytes.len() != 32 {
        return false;
    }

    let mut pubkey_array = [0u8; 32];
    pubkey_array.copy_from_slice(&pubkey_bytes);
    
    let pubkey = match VerifyingKey::from_bytes(&pubkey_array) {
        Ok(pk) => pk,
        Err(_) => return false,
    };

    let msg_hash = Sha256::digest(msg);
    
    let mut sig_array = [0u8; 64];
    sig_array.copy_from_slice(sig_bytes);
    let signature = Signature::from_bytes(&sig_array);

    pubkey.verify(&msg_hash, &signature).is_ok()
}

