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

const DEFAULT_BATCH_SIZE: u64 = 524_288; // 512K hashes (increased from 256K)
const THREADS_PER_BLOCK: u32 = 256; // Stable setting
const MAX_BLOCKS: u32 = 768; // Increased from 512 (GTX 1050 Ti has 768 cores)

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

    // Generate/load DAG for current epoch (memory-hard PoW)
    let epoch = crate::consensus::dag::get_epoch(index);
    println!("[CUDA] Block {} is in epoch {}, generating 4GB DAG...", index, epoch);
    
    // TODO: Cache DAG to avoid regeneration
    let dag = crate::consensus::dag::generate_full_dag(epoch)?;
    
    // DEBUG: Verify DAG contents
    println!("[DEBUG] DAG size: {} bytes", dag.len());
    println!("[DEBUG] DAG first 16 bytes: {}", hex::encode(&dag[..16]));
    println!("[DEBUG] DAG[0] (first item, first 16 bytes): {}", hex::encode(&dag[0..16]));
    
    // Verify specific indices from first test
    let idx0_offset = 16275540 * 128;
    let idx1_offset = 22845103 * 128;
    let idx2_offset = 32858808 * 128;
    if idx2_offset + 16 <= dag.len() {
        println!("[DEBUG] DAG item at idx 16275540: {}", hex::encode(&dag[idx0_offset..idx0_offset+16]));
        println!("[DEBUG] DAG item at idx 22845103: {}", hex::encode(&dag[idx1_offset..idx1_offset+16]));
        println!("[DEBUG] DAG item at idx 32858808: {}", hex::encode(&dag[idx2_offset..idx2_offset+16]));
    }
    
    println!("[CUDA] DAG generation complete, uploading to GPU memory (4GB)...");
    
    // Upload DAG to GPU (this is expensive - 4GB!)
    let dag_dev = DeviceBuffer::from_slice(&dag)
        .map_err(|e| anyhow!("Failed to upload DAG to GPU: {}. Make sure you have at least 4GB VRAM.", e))?;
    println!("[CUDA] DAG uploaded to GPU, starting mining...");

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

    // Convert compact difficulty format to leading zeros count for CUDA kernel
    let leading_zeros = crate::consensus::compact_to_leading_zeros(difficulty);
    
    let mut start_nonce: u64 = 0;
    let mut last_rate_update = std::time::Instant::now();
    let mut hashes_since_update: u64 = 0;
    let mut last_console_update = std::time::Instant::now();
    let mut total_hashes: u64 = 0;

    println!("[CUDA] Starting mining loop: {} blocks × {} threads = {} parallel threads", blocks, THREADS_PER_BLOCK, blocks * THREADS_PER_BLOCK);
    println!("[CUDA] Batch size: {} hashes per kernel call", batch_size);
    println!("[CUDA] Difficulty: 0x{:08x} → {} leading hex zeros (1/{} chance)", difficulty, leading_zeros, 16_u64.pow(leading_zeros));
    println!("[CUDA] Mining with 4GB DAG (memory-hard PoW)...");

    loop {
        if cancel_flag.load(Ordering::Relaxed) {
            return Err(anyhow!("Mining cancelled due to new peer block"));
        }

        if start_nonce % (batch_size * 10) == 0 && start_nonce > 0 {
            println!("[CUDA] Processed {} batches, current nonce: {}", start_nonce / batch_size, start_nonce);
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
                leading_zeros as i32,  // Pass converted leading zeros, not raw compact bits!
                found_flag.as_device_ptr(),
                found_nonce.as_device_ptr(),
                found_hash.as_device_ptr(),
                dag_dev.as_device_ptr(),
                dag.len() as u64
            ))
                .map_err(|e| anyhow!("CUDA kernel launch failed: {}", e))?;
        }

        if start_nonce == 0 {
            println!("[CUDA] First kernel launched successfully, waiting for completion...");
        }

        stream.synchronize()
            .map_err(|e| anyhow!("CUDA stream synchronization failed: {}", e))?;

        if start_nonce == 0 {
            println!("[CUDA] First batch completed! Mining is working.");
        }

        let mut flag_host = [0u32];
        found_flag.copy_to(&mut flag_host)?;

        hashes_since_update = hashes_since_update.saturating_add(batch_size);
        total_hashes = total_hashes.saturating_add(batch_size);

        let elapsed = last_rate_update.elapsed();
        // Update hashrate more frequently (every 50ms) for accurate reporting on GTX 1050 Ti
        if elapsed.as_millis() >= 50 {
            let rate = hashes_since_update as f64 / elapsed.as_secs_f64();
            if let Some(ref hr) = hashrate {
                if let Ok(mut hr_lock) = hr.try_lock() {
                    *hr_lock = rate;
                }
            }
            
            // Console output every 5 seconds
            if last_console_update.elapsed().as_secs() >= 5 {
                let mhs = rate / 1_000_000.0;
                println!("[CUDA] Mining: {:.2} MH/s | Total: {} MH | Nonce range: {}-{}", 
                    mhs, total_hashes / 1_000_000, start_nonce, start_nonce + batch_size);
                last_console_update = std::time::Instant::now();
            }
            
            hashes_since_update = 0;
            last_rate_update = std::time::Instant::now();
        }

        if flag_host[0] != 0 {
            let mut nonce_host = [0u64];
            let mut gpu_pow_hash = [0u8; 32];
            found_nonce.copy_to(&mut nonce_host)?;
            found_hash.copy_to(&mut gpu_pow_hash)?;

            let nonce = nonce_host[0];
            
            // Verify GPU PoW hash meets difficulty
            let gpu_pow_hex = hex::encode(gpu_pow_hash);
            
            // Convert compact difficulty format to leading zeros count
            let leading_zeros = crate::consensus::compact_to_leading_zeros(difficulty);
            if !gpu_pow_hex.starts_with(&"0".repeat(leading_zeros as usize)) {
                return Err(anyhow!("GPU found nonce did not satisfy target (requires {} leading zeros)", leading_zeros));
            }
            
            println!("[CUDA] ✅ Valid PoW found! Nonce: {}, GPU PoW: {}", nonce, gpu_pow_hex);

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
            
            // Compute canonical header hash (block identifier)
            let recomposed = build_header_bytes(&prefix, nonce, &suffix);
            let header_hash = crate::block::blake3_hash(&recomposed);
            let header_hash_hex = hex::encode(header_hash);
            
            // Block hash = header hash (NOT GPU PoW hash)
            // GPU PoW verification confirmed nonce is valid above
            let block = Block {
                header: header.clone(),
                transactions: all_txs,
                hash: header_hash_hex,
            };
            return Ok(block);
        }

        start_nonce = start_nonce.wrapping_add(batch_size);
    }
}
