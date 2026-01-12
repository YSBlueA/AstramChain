use secp256k1::{Secp256k1, SecretKey, PublicKey};
use rand::rngs::OsRng;
use rand::RngCore;
use hex;
use tiny_keccak::{Hasher, Keccak};

pub struct Wallet {
    pub secret_key: SecretKey,
    pub public_key: PublicKey,
    pub address: String,
}

impl Wallet {
    /// 새 지갑 생성 (secp256k1, Ethereum 호환)
    pub fn new() -> Self {
        let secp = Secp256k1::new();
        let mut rng = OsRng;
        let mut secret_bytes = [0u8; 32];
        rng.fill_bytes(&mut secret_bytes);
        
        let secret_key = SecretKey::from_slice(&secret_bytes)
            .expect("32 bytes, within curve order");
        let public_key = PublicKey::from_secret_key(&secp, &secret_key);
        let address = Self::address_from_public(&public_key);
        
        Self {
            secret_key,
            public_key,
            address,
        }
    }

    fn address_from_public(pubkey: &PublicKey) -> String {
        let pubkey_bytes = pubkey.serialize_uncompressed();
        let hash = keccak256(&pubkey_bytes[1..]); // Skip 0x04 prefix
        format!("0x{}", hex::encode(&hash[12..32])) // Last 20 bytes with 0x prefix
    }

    pub fn secret_hex(&self) -> String {
        hex::encode(self.secret_key.secret_bytes())
    }

    pub fn public_hex(&self) -> String {
        hex::encode(self.public_key.serialize_uncompressed())
    }
    
    /// Ethereum 체크섬 주소 (0x 접두사 포함)
    pub fn checksummed_address(&self) -> String {
        to_checksum_address(&self.address)
    }

    /// 16진수 개인키로부터 복원
    pub fn from_hex(hex_str: &str) -> Self {
        let secp = Secp256k1::new();
        let secret_bytes = hex::decode(hex_str).expect("Invalid hex string");
        let secret_key = SecretKey::from_slice(&secret_bytes)
            .expect("Invalid secret key");
        let public_key = PublicKey::from_secret_key(&secp, &secret_key);
        let address = Self::address_from_public(&public_key);

        Self {
            secret_key,
            public_key,
            address,
        }
    }
}

fn keccak256(data: &[u8]) -> [u8; 32] {
    let mut hasher = Keccak::v256();
    let mut output = [0u8; 32];
    hasher.update(data);
    hasher.finalize(&mut output);
    output
}

fn to_checksum_address(address: &str) -> String {
    let address = address.trim_start_matches("0x").to_lowercase();
    let hash = hex::encode(keccak256(address.as_bytes()));
    
    let mut result = String::from("0x");
    for (i, ch) in address.chars().enumerate() {
        if ch.is_numeric() {
            result.push(ch);
        } else {
            let hash_char = hash.chars().nth(i).unwrap();
            if hash_char >= '8' {
                result.push(ch.to_uppercase().next().unwrap());
            } else {
                result.push(ch);
            }
        }
    }
    result
}
