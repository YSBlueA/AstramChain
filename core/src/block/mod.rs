use crate::transaction::Transaction;
use anyhow::Result;
use bincode::{Decode, Encode};
use hex;
use sha2::{Digest, Sha256};

/// block header
#[derive(Encode, Decode, Debug, Clone)]
pub struct BlockHeader {
    pub index: u64,
    pub previous_hash: String, // hex
    pub merkle_root: String,   // hex
    pub timestamp: i64,        // unix seconds
    pub nonce: u64,
    pub difficulty: u32,
}

#[derive(Encode, Decode, Debug, Clone)]
pub struct Block {
    pub header: BlockHeader,
    pub transactions: Vec<Transaction>,
    pub hash: String, // hex string (computed from serialized header)
}

pub fn sha256d(data: &[u8]) -> [u8; 32] {
    let h1 = Sha256::digest(data);
    let h2 = Sha256::digest(&h1);
    let mut out = [0u8; 32];
    out.copy_from_slice(&h2);
    out
}

pub fn to_hex(hash: &[u8; 32]) -> String {
    hex::encode(hash)
}

/// Deterministic serialization: use bincode (v2 Encode trait)
pub fn serialize_header(header: &BlockHeader) -> Result<Vec<u8>, bincode::error::EncodeError> {
    let config = bincode::config::standard();
    Ok(bincode::encode_to_vec(header, config)?)
}

/// Compute hash from the header (sha256d)
pub fn compute_header_hash(header: &BlockHeader) -> Result<String, anyhow::Error> {
    let bytes = serialize_header(header)?;
    let h = sha256d(&bytes);
    Ok(to_hex(&h))
}

/// Compute merkle root (assuming txids are in hex format)
pub fn compute_merkle_root(txids: &[String]) -> String {
    if txids.is_empty() {
        return to_hex(&sha256d(&[]));
    }

    // decode hex -> bytes array [u8; 32]
    let mut leaves: Vec<[u8; 32]> = txids
        .iter()
        .map(|h| {
            let b = hex::decode(h).unwrap_or_else(|_| vec![0u8; 32]);
            let mut a = [0u8; 32];
            if b.len() == 32 {
                a.copy_from_slice(&b);
            }
            a
        })
        .collect();

    while leaves.len() > 1 {
        if leaves.len() % 2 == 1 {
            let last = *leaves.last().unwrap();
            leaves.push(last);
        }

        let mut next = Vec::with_capacity(leaves.len() / 2);
        for i in (0..leaves.len()).step_by(2) {
            let mut concat = Vec::with_capacity(64);
            concat.extend_from_slice(&leaves[i]);
            concat.extend_from_slice(&leaves[i + 1]);
            let h = sha256d(&concat);
            next.push(h);
        }
        leaves = next;
    }

    to_hex(&leaves[0])
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn merkle_two() {
        let a = "00".repeat(32);
        let b = "11".repeat(32);
        let root = compute_merkle_root(&vec![a, b]);
        assert!(!root.is_empty());
    }

    #[test]
    fn serialize_header_and_hash() {
        let header = BlockHeader {
            index: 1,
            previous_hash: "00".repeat(32),
            merkle_root: "11".repeat(32),
            timestamp: 1234567890,
            nonce: 42,
            difficulty: 1,
        };

        let bytes = serialize_header(&header).unwrap();
        assert!(!bytes.is_empty());

        let hash = compute_header_hash(&header).unwrap();
        assert_eq!(hash.len(), 64);
    }
}
