// core/consensus.rs
use crate::block::{Block, BlockHeader, compute_header_hash, compute_merkle_root};
use crate::transaction::Transaction;
use anyhow::{Result, anyhow};
use chrono::Utc;
use primitive_types::U256;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

/// Find a valid nonce by updating header.nonce and returning (nonce, hash).
/// Simple CPU single-threaded loop. Caller should run this in spawn_blocking.
pub fn find_valid_nonce(header: &mut BlockHeader, difficulty: u32) -> Result<(u64, String)> {
    let target_prefix = "0".repeat(difficulty as usize);
    let mut nonce: u64 = header.nonce;

    loop {
        header.nonce = nonce;
        let hash = compute_header_hash(header)?;
        if hash.starts_with(&target_prefix) {
            return Ok((nonce, hash));
        }

        nonce = nonce.wrapping_add(1);
        // yield occasionally so OS scheduler can run other threads
        if nonce % 1_000_000 == 0 {
            std::thread::yield_now();
        }
    }
}

/// High-level miner function that prepends a coinbase tx, computes merkle, and runs PoW.
/// - `index`: block index (must be provided by caller; index is part of header/hash)
/// - `previous_hash`: previous block hash hex
/// - `difficulty`: number of leading-hex-nibble zero characters to require (simple model)
/// - `transactions`: non-coinbase transactions (txids should already be set)
/// - `miner_address`: address to receive coinbase reward
///
/// Returns mined Block (header.nonce and hash set).
pub fn mine_block_with_coinbase(
    index: u64,
    prev_hash: String,
    difficulty: u32,
    txs: Vec<Transaction>,
    miner_addr: &str,
    reward: U256,
    cancel_flag: Arc<AtomicBool>,
) -> Result<Block> {
    let coinbase = Transaction::coinbase(miner_addr, reward).with_hashes();
    let mut all_txs = vec![coinbase];
    all_txs.extend(txs);

    let txids: Vec<String> = all_txs.iter().map(|t| t.txid.clone()).collect();
    let merkle_root = compute_merkle_root(&txids);

    let mut header = BlockHeader {
        index,
        previous_hash: prev_hash.clone(),
        merkle_root,
        timestamp: Utc::now().timestamp(),
        nonce: 0,
        difficulty,
    };

    let target_prefix = "0".repeat(difficulty as usize);
    let mut nonce: u64 = 0;

    // ⛏️ CPU mining loop
    loop {
        // ⛔ network cancellation check
        if cancel_flag.load(Ordering::Relaxed) {
            return Err(anyhow!("Mining cancelled due to new peer block"));
        }

        header.nonce = nonce;
        let hash = compute_header_hash(&header)?;
        if hash.starts_with(&target_prefix) {
            let block = Block {
                header: header.clone(),
                transactions: all_txs,
                hash,
            };
            return Ok(block);
        }

        nonce += 1;

        // ⏸️ 100,000 nonces, check cancellation flag and show progress
        if nonce % 100_000 == 0 {
            if cancel_flag.load(Ordering::Relaxed) {
                return Err(anyhow!("Mining cancelled"));
            }
            // Show mining progress every 100k hashes
            if nonce % 1_000_000 == 0 {
                log::debug!("⛏️  Mining progress: {} hashes tried (difficulty: {})", nonce, difficulty);
            }
        }
    }
}
