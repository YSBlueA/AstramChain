use crate::block::{
    Block, BlockHeader, compute_header_hash, compute_merkle_root, serialize_header,
};
use crate::transaction::Transaction;
use anyhow::{Result, anyhow};
use chrono::Utc;
use cust::launch;
use cust::prelude::*;
use hex;
use primitive_types::U256;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

const DEFAULT_BATCH_SIZE: u64 = 50_000_000; // 50M hashes per batch for better GPU utilization
const THREADS_PER_BLOCK: u32 = 256;
const MAX_BLOCKS: u32 = 4096;

fn encode_field<T: bincode::Encode>(value: &T) -> Result<Vec<u8>> {
    let config = bincode::config::standard()
        .with_fixed_int_encoding(); // Use fixed-length encoding
    Ok(bincode::encode_to_vec(value, config)?)
}

fn build_header_bytes(prefix: &[u8], nonce: u64, suffix: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(prefix.len() + 8 + suffix.len());
    out.extend_from_slice(prefix);
    // Fixed-length encoding: u64 = 8 bytes little-endian
    out.extend_from_slice(&nonce.to_le_bytes());
    out.extend_from_slice(suffix);
    out
}

pub fn mine_block_with_coinbase_cuda(
    index: u64,
    prev_hash: String,
    difficulty: u32,
    txs: Vec<Transaction>,
    miner_addr: &str,
    reward: U256,
    cancel_flag: Arc<AtomicBool>,
    hashrate: Option<Arc<std::sync::Mutex<f64>>>,
) -> Result<Block> {
    let _ctx = cust::quick_init()
        .map_err(|e| anyhow!("Failed to initialize CUDA context: {}. Make sure you have an NVIDIA GPU and proper drivers installed.", e))?;

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

    let prefix = {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&encode_field(&header.index)?);
        bytes.extend_from_slice(&encode_field(&header.previous_hash)?);
        bytes.extend_from_slice(&encode_field(&header.merkle_root)?);
        bytes.extend_from_slice(&encode_field(&header.timestamp)?);
        bytes
    };
    let suffix = encode_field(&header.difficulty)?;

    // Sanity check: reconstructed bytes must match bincode header encoding.
    let sample_nonce = 0u64;
    let recomposed = build_header_bytes(&prefix, sample_nonce, &suffix);
    header.nonce = sample_nonce;
    let serialized = serialize_header(&header)?;
    if recomposed != serialized {
        return Err(anyhow!(
            "CUDA header serialization mismatch; aborting GPU mining"
        ));
    }

    let ptx = include_str!(concat!(env!("OUT_DIR"), "/miner.ptx"));
    let module = Module::from_ptx(ptx, &[])
        .map_err(|e| anyhow!("Failed to load CUDA PTX module: {}", e))?;
    let stream = Stream::new(StreamFlags::NON_BLOCKING, None)
        .map_err(|e| anyhow!("Failed to create CUDA stream: {}", e))?;
    let function = module.get_function("mine_kernel")
        .map_err(|e| anyhow!("Failed to get CUDA kernel function 'mine_kernel': {}", e))?;

    let prefix_dev = DeviceBuffer::from_slice(&prefix)?;
    let suffix_dev = DeviceBuffer::from_slice(&suffix)?;

    let mut found_flag = DeviceBuffer::from_slice(&[0u32])?;
    let found_nonce = DeviceBuffer::from_slice(&[0u64])?;
    let found_hash = DeviceBuffer::from_slice(&[0u8; 32])?;

    let batch_size = std::env::var("CUDA_BATCH_SIZE")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(DEFAULT_BATCH_SIZE)
        .max(1);

    let blocks = ((batch_size + THREADS_PER_BLOCK as u64 - 1) / THREADS_PER_BLOCK as u64)
        .min(MAX_BLOCKS as u64) as u32;

    let mut start_nonce: u64 = 0;
    let mut last_rate_update = std::time::Instant::now();
    let mut hashes_since_update: u64 = 0;

    loop {
        if cancel_flag.load(Ordering::Relaxed) {
            return Err(anyhow!("Mining cancelled due to new peer block"));
        }

        found_flag.copy_from(&[0u32])?;

        unsafe {
            launch!(function<<<blocks, THREADS_PER_BLOCK, 0, stream>>>(
                prefix_dev.as_device_ptr(),
                prefix.len() as i32,
                suffix_dev.as_device_ptr(),
                suffix.len() as i32,
                start_nonce,
                batch_size,
                difficulty as i32,
                found_flag.as_device_ptr(),
                found_nonce.as_device_ptr(),
                found_hash.as_device_ptr()
            ))?;
        }

        stream.synchronize()?;

        let mut flag_host = [0u32];
        found_flag.copy_to(&mut flag_host)?;

        hashes_since_update = hashes_since_update.saturating_add(batch_size);

        let elapsed = last_rate_update.elapsed();
        // Update hashrate more frequently (every 100ms) for more accurate reporting
        if elapsed.as_millis() >= 100 {
            let rate = hashes_since_update as f64 / elapsed.as_secs_f64();
            if let Some(ref hr) = hashrate {
                if let Ok(mut hr_lock) = hr.try_lock() {
                    *hr_lock = rate;
                }
            }
            hashes_since_update = 0;
            last_rate_update = std::time::Instant::now();
        }

        if flag_host[0] != 0 {
            let mut nonce_host = [0u64];
            let mut hash_host = [0u8; 32];
            found_nonce.copy_to(&mut nonce_host)?;
            found_hash.copy_to(&mut hash_host)?;

            // CPU verification using the same method as GPU
            let nonce = nonce_host[0];
            let recomposed = build_header_bytes(&prefix, nonce, &suffix);
            let cpu_hash = crate::block::sha256d(&recomposed);
            let cpu_hash_hex = hex::encode(cpu_hash);
            let gpu_hash_hex = hex::encode(hash_host);
            
            if cpu_hash_hex != gpu_hash_hex {
                return Err(anyhow!("GPU hash mismatch - GPU: {}, CPU: {}", gpu_hash_hex, cpu_hash_hex));
            }
            if !cpu_hash_hex.starts_with(&"0".repeat(difficulty as usize)) {
                return Err(anyhow!("GPU found nonce did not satisfy target"));
            }

            // Update final hashrate before returning
            let final_elapsed = last_rate_update.elapsed();
            if final_elapsed.as_secs_f64() > 0.0 {
                let final_rate = hashes_since_update as f64 / final_elapsed.as_secs_f64();
                if let Some(ref hr) = hashrate {
                    if let Ok(mut hr_lock) = hr.try_lock() {
                        *hr_lock = final_rate;
                    }
                }
            }

            header.nonce = nonce;
            
            // Compute the canonical hash using bincode serialization for block storage
            let canonical_hash = compute_header_hash(&header)?;
            
            let block = Block {
                header: header.clone(),
                transactions: all_txs,
                hash: canonical_hash,
            };
            return Ok(block);
        }

        start_nonce = start_nonce.wrapping_add(batch_size);
    }
}
