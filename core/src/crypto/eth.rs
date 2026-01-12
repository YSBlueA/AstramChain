/// Ethereum-compatible cryptography for MetaMask integration
use anyhow::Result;
use hex;
use secp256k1::ecdsa::Signature;
use secp256k1::{Message, PublicKey, Secp256k1, SecretKey};
use sha2::{Digest, Sha256};
use tiny_keccak::{Hasher, Keccak};

/// Ethereum-style wallet using secp256k1
pub struct EthWallet {
    pub secret_key: SecretKey,
    pub public_key: PublicKey,
    pub address: String, // 0x prefixed Ethereum address
}

impl EthWallet {
    /// Generate new Ethereum-compatible wallet
    pub fn new() -> Result<Self> {
        let secp = Secp256k1::new();
        let (secret_key, public_key) = secp.generate_keypair(&mut rand::thread_rng());
        let address = eth_address_from_public_key(&public_key);

        Ok(EthWallet {
            secret_key,
            public_key,
            address,
        })
    }

    /// Create wallet from private key hex string
    pub fn from_private_key(private_key_hex: &str) -> Result<Self> {
        let private_key_hex = private_key_hex
            .strip_prefix("0x")
            .unwrap_or(private_key_hex);
        let secret_bytes = hex::decode(private_key_hex)?;
        let secret_key = SecretKey::from_slice(&secret_bytes)?;

        let secp = Secp256k1::new();
        let public_key = PublicKey::from_secret_key(&secp, &secret_key);
        let address = eth_address_from_public_key(&public_key);

        Ok(EthWallet {
            secret_key,
            public_key,
            address,
        })
    }

    /// Get private key as hex string (with 0x prefix)
    pub fn private_key_hex(&self) -> String {
        format!("0x{}", hex::encode(self.secret_key.secret_bytes()))
    }

    /// Sign message with Ethereum's standard
    pub fn sign_message(&self, message: &[u8]) -> Result<String> {
        let secp = Secp256k1::new();

        // Ethereum uses Keccak256 for message hashing
        let hash = keccak256(message);
        let msg = Message::from_digest_slice(&hash)?;

        let signature = secp.sign_ecdsa(&msg, &self.secret_key);

        // Serialize to 65 bytes (r + s + v)
        let sig_bytes = signature.serialize_compact();
        let recovery_id = 0u8; // In production, calculate proper recovery ID

        let mut full_sig = [0u8; 65];
        full_sig[..64].copy_from_slice(&sig_bytes);
        full_sig[64] = recovery_id + 27; // Ethereum v value

        Ok(format!("0x{}", hex::encode(full_sig)))
    }

    /// Verify signature
    pub fn verify_signature(
        message: &[u8],
        signature_hex: &str,
        expected_address: &str,
    ) -> Result<bool> {
        let signature_hex = signature_hex.strip_prefix("0x").unwrap_or(signature_hex);
        let sig_bytes = hex::decode(signature_hex)?;

        if sig_bytes.len() != 65 {
            return Ok(false);
        }

        let hash = keccak256(message);
        let msg = Message::from_digest_slice(&hash)?;

        // Extract r, s from signature
        let signature = Signature::from_compact(&sig_bytes[..64])?;

        let secp = Secp256k1::new();

        // Recover public key from signature
        let recovery_id =
            secp256k1::ecdsa::RecoveryId::from_i32(((sig_bytes[64] - 27) % 4) as i32)?;
        let recoverable_sig =
            secp256k1::ecdsa::RecoverableSignature::from_compact(&sig_bytes[..64], recovery_id)?;

        let recovered_pubkey = secp.recover_ecdsa(&msg, &recoverable_sig)?;
        let recovered_address = eth_address_from_public_key(&recovered_pubkey);

        Ok(recovered_address.eq_ignore_ascii_case(expected_address))
    }
}

/// Generate Ethereum address from secp256k1 public key
/// Address = 0x + last 20 bytes of Keccak256(public_key)
pub fn eth_address_from_public_key(public_key: &PublicKey) -> String {
    // Get uncompressed public key (65 bytes: 0x04 + 32 bytes X + 32 bytes Y)
    let pubkey_bytes = public_key.serialize_uncompressed();

    // Remove the 0x04 prefix, hash the remaining 64 bytes
    let hash = keccak256(&pubkey_bytes[1..]);

    // Take last 20 bytes and add 0x prefix
    format!("0x{}", hex::encode(&hash[12..]))
}

/// Keccak256 hash function (Ethereum standard)
pub fn keccak256(data: &[u8]) -> [u8; 32] {
    let mut hasher = Keccak::v256();
    hasher.update(data);
    let mut output = [0u8; 32];
    hasher.finalize(&mut output);
    output
}

/// Convert Ethereum address to checksum address (EIP-55)
pub fn to_checksum_address(address: &str) -> String {
    let address = address.strip_prefix("0x").unwrap_or(address).to_lowercase();
    let hash = hex::encode(keccak256(address.as_bytes()));

    let mut checksum_address = String::from("0x");
    for (i, ch) in address.chars().enumerate() {
        if ch.is_numeric() {
            checksum_address.push(ch);
        } else {
            let hash_char = hash.chars().nth(i).unwrap();
            if hash_char >= '8' {
                checksum_address.push(ch.to_ascii_uppercase());
            } else {
                checksum_address.push(ch);
            }
        }
    }

    checksum_address
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_eth_wallet_creation() {
        let wallet = EthWallet::new().unwrap();
        assert!(wallet.address.starts_with("0x"));
        assert_eq!(wallet.address.len(), 42); // 0x + 40 hex chars
    }

    #[test]
    fn test_private_key_import() {
        let wallet1 = EthWallet::new().unwrap();
        let private_key = wallet1.private_key_hex();

        let wallet2 = EthWallet::from_private_key(&private_key).unwrap();
        assert_eq!(wallet1.address, wallet2.address);
    }

    #[test]
    fn test_sign_and_verify() {
        let wallet = EthWallet::new().unwrap();
        let message = b"Hello, Ethereum!";

        let signature = wallet.sign_message(message).unwrap();
        let is_valid = EthWallet::verify_signature(message, &signature, &wallet.address).unwrap();

        assert!(is_valid);
    }

    #[test]
    fn test_checksum_address() {
        let address = "0x5aaeb6053f3e94c9b9a09f33669435e7ef1beaed";
        let checksum = to_checksum_address(address);
        assert_eq!(checksum, "0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed");
    }
}
