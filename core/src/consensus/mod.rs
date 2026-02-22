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

pub mod dag;

#[cfg(feature = "cuda-miner")]
pub mod cuda;

#[cfg(feature = "cuda-miner")]
pub use cuda::mine_block_with_coinbase_cuda;

/// Convert compact difficulty format (bits) to required leading zero count
/// Compact format: [exponent (1 byte)][mantissa (3 bytes)]
/// Relationship: difficulty increases by factor of 16 for each additional leading hex-zero
/// Formula: average_attempts = 16^(leading_zeros) = 2^(4 * leading_zeros)
pub fn compact_to_leading_zeros(bits: u32) -> u32 {
    let exponent = bits >> 24;
    
    if exponent == 0 {
        return 0;
    }
    
    // For simplicity in this implementation:
    // exponent 32 (0x20) = 8 leading zeros (2-min @ 40MH/s)
    // exponent 31 (0x1f) = 6 leading zeros (1-min @ 40MH/s)
    // exponent 30 (0x1e) = 4 leading zeros (5-sec @ 40MH/s)
    
    // Map exponent to leading zeros
    // This is a custom mapping for our consensus rules
    match exponent {
        0x20 => 8,   // Mainnet (2 min @ 40MH/s)
        0x1f => 6,   // 1 min @ 40MH/s
        0x1e => 4,   // 5 sec @ 40MH/s
        0x1d => 2,   // Testing
        _ => {
            // Fallback: rough estimation
            if exponent > 0x20 { 8 } else { 0 }
        }
    }
}

/// Find a valid nonce by updating header.nonce and returning (nonce, hash).
/// Now uses memory-hard DAG mixing for ASIC resistance.
pub fn find_valid_nonce_with_dag(
    header: &mut BlockHeader, 
    difficulty: u32,
    dag: &[u8]
) -> Result<(u64, String)> {
    // Convert compact difficulty format to leading zeros count
    let leading_zeros = compact_to_leading_zeros(difficulty);
    let target_prefix = "0".repeat(leading_zeros as usize);
    let mut nonce: u64 = header.nonce;

    // Compute header hash once (without nonce)
    let header_hash = compute_header_hash(header)?;
    let mut header_hash_bytes = [0u8; 32];
    hex::decode_to_slice(&header_hash, &mut header_hash_bytes)?;

    loop {
        // Memory-hard mixing with DAG
        let dag_hash = dag::hash_with_dag(&header_hash_bytes, nonce, dag);
        let hash_str = hex::encode(&dag_hash);
        
        if hash_str.starts_with(&target_prefix) {
            return Ok((nonce, hash_str));
        }

        nonce = nonce.wrapping_add(1);
        // yield occasionally so OS scheduler can run other threads
        if nonce % 100_000 == 0 {
            std::thread::yield_now();
        }
    }
}

/// Legacy CPU miner without DAG (fast but not memory-hard)
pub fn find_valid_nonce(header: &mut BlockHeader, difficulty: u32) -> Result<(u64, String)> {
    // Convert compact difficulty format to leading zeros count
    let leading_zeros = compact_to_leading_zeros(difficulty);
    let target_prefix = "0".repeat(leading_zeros as usize);
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
    hashrate: Option<Arc<std::sync::Mutex<f64>>>,
) -> Result<Block> {
    println!("[DEBUG] Mining: mine_block_with_coinbase called with difficulty=0x{:08x}", difficulty);
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

    // Generate/load DAG for current epoch (memory-hard PoW)
    let epoch = dag::get_epoch(index);
    println!("[DEBUG] Mining: Block {} is in epoch {}, checking DAG...", index, epoch);
    
    // TODO: Cache DAG to avoid regeneration
    // For now, generate it each time (expensive!)
    let dag = dag::generate_full_dag(epoch)?;
    println!("[DEBUG] Mining: DAG ready (4GB), starting mining...");

    // Compute header hash (without nonce)
    let header_hash = compute_header_hash(&header)?;
    let mut header_hash_bytes = [0u8; 32];
    hex::decode_to_slice(&header_hash, &mut header_hash_bytes)
        .map_err(|e| anyhow!("Failed to decode header hash: {}", e))?;

    // Convert compact difficulty format to leading zeros count
    let leading_zeros = compact_to_leading_zeros(difficulty);
    let target_prefix = "0".repeat(leading_zeros as usize);
    let mut nonce: u64 = 0;
    let mining_start = std::time::Instant::now();
    let mut last_hashrate_update = mining_start;
    let mut hashes_since_update: u64 = 0;
    
    println!("[DEBUG] Mining: Entering memory-hard mining loop, difficulty=0x{:08x} requires {} leading zeros", difficulty, target_prefix.len());

    // ⛏️ CPU mining loop with DAG
    loop {
        // ⛔ network cancellation check
        if cancel_flag.load(Ordering::Relaxed) {
            return Err(anyhow!("Mining cancelled due to new peer block"));
        }

        // Log first iteration only
        if nonce == 0 {
            println!("[DEBUG] Mining loop: STARTING iteration with nonce=0 (DAG-based)");
        }

        // Memory-hard mixing with DAG
        let dag_hash = dag::hash_with_dag(&header_hash_bytes, nonce, &dag);
        let hash = hex::encode(&dag_hash);
        
        if hash.starts_with(&target_prefix) {
            println!("[DEBUG] Mining: FOUND valid hash! nonce={}, hash_prefix={}", nonce, &hash[..20]);
            
            // Set the nonce in header
            header.nonce = nonce;
            
            // Update final hashrate before returning
            let final_elapsed = last_hashrate_update.elapsed();
            if final_elapsed.as_secs_f64() > 0.0 {
                let final_rate = hashes_since_update as f64 / final_elapsed.as_secs_f64();
                if let Some(ref hr) = hashrate {
                    if let Ok(mut hr_lock) = hr.try_lock() {
                        *hr_lock = final_rate;
                    }
                }
            }
            
            println!("[DEBUG] Mining: Creating block with {} transactions", all_txs.len());
            let block = Block {
                header: header.clone(),
                transactions: all_txs,
                hash,
            };
            println!("[DEBUG] Mining: Returning mined block!");
            return Ok(block);
        }

        nonce += 1;
        hashes_since_update += 1;

        // ⏸️ 100,000 nonces, check cancellation flag and show progress
        if nonce % 100_000 == 0 {
            if cancel_flag.load(Ordering::Relaxed) {
                return Err(anyhow!("Mining cancelled"));
            }

            // Update hashrate more frequently (every 100ms) for more accurate reporting
            let elapsed = last_hashrate_update.elapsed();
            if elapsed.as_millis() >= 100 {
                let current_hashrate = hashes_since_update as f64 / elapsed.as_secs_f64();

                // Update shared hashrate if provided
                if let Some(ref hr) = hashrate {
                    if let Ok(mut hr_lock) = hr.try_lock() {
                        *hr_lock = current_hashrate;
                    }
                }

                hashes_since_update = 0;
                last_hashrate_update = std::time::Instant::now();
            }
        }
    }
}
