use crate::block::{
    Block, BlockHeader, compute_merkle_root, serialize_header,
};
use crate::transaction::Transaction;
use anyhow::{Result, anyhow};
use chrono::Utc;
use cust::launch;
use cust::prelude::*;
use hex;
use primitive_types::U256;
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicBool, Ordering},
};

/// Global DAG cache: (epoch, dag_bytes). Avoids 4GB regeneration on every job change.
static DAG_CACHE: OnceLock<std::sync::Mutex<(u64, Option<Arc<Vec<u8>>>)>> = OnceLock::new();

fn get_or_generate_dag(epoch: u64, block_index: u64) -> Result<Arc<Vec<u8>>> {
    let mutex = DAG_CACHE.get_or_init(|| std::sync::Mutex::new((u64::MAX, None)));
    let mut lock = mutex.lock().unwrap();
    if lock.0 == epoch {
        if let Some(ref arc) = lock.1 {
            println!("[CUDA] Block {} (epoch {}), reusing cached DAG.", block_index, epoch);
            return Ok(arc.clone());
        }
    }
    println!("[CUDA] Block {} (epoch {}), generating 4GB DAG...", block_index, epoch);
    let dag = crate::consensus::dag::generate_full_dag(epoch)?;
    let arc = Arc::new(dag);
    *lock = (epoch, Some(arc.clone()));
    Ok(arc)
}

const DEFAULT_BATCH_SIZE: u64 = 524_288; // 512K hashes (increased from 256K)
const THREADS_PER_BLOCK: u32 = 256; // Stable setting
const MAX_BLOCKS: u32 = 768; // Increased from 512 (GTX 1050 Ti has 768 cores)

/// Mine a pre-built block header (for pool/stratum mining where coinbase is set by pool).
/// Returns (nonce, block_hash_hex) on success.
///
/// `target_override` – if provided, mining stops when `hash < target_override` instead of
/// the target derived from `header.difficulty`.  Use this for pool share mining so the
/// miner submits shares at pool difficulty while keeping `header.difficulty` at network
/// difficulty (so the header hash remains valid).
pub fn mine_header_cuda(
    header: BlockHeader,
    cancel_flag: Arc<AtomicBool>,
    hashrate: Option<Arc<std::sync::Mutex<f64>>>,
    target_override: Option<[u8; 32]>,
) -> Result<(u64, String)> {
    let _ctx = cust::quick_init()
        .map_err(|e| anyhow!("Failed to initialize CUDA context: {}. Make sure you have an NVIDIA GPU and proper drivers installed.", e))?;

    let difficulty = header.difficulty;
    let index = header.index;

    let epoch = crate::consensus::dag::get_epoch(index);
    let dag_arc = get_or_generate_dag(epoch, index)?;
    let dag_dev = DeviceBuffer::from_slice(dag_arc.as_ref())
        .map_err(|e| anyhow!("Failed to upload DAG to GPU: {}. Need at least 4GB VRAM.", e))?;

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
    let mut check_header = header.clone();
    check_header.nonce = sample_nonce;
    let serialized = serialize_header(&check_header)?;
    if recomposed != serialized {
        return Err(anyhow!("CUDA header serialization mismatch; aborting GPU mining"));
    }

    let ptx = include_str!(concat!(env!("OUT_DIR"), "/miner.ptx"));
    let module = Module::from_ptx(ptx, &[])
        .map_err(|e| anyhow!("Failed to load CUDA PTX module: {}", e))?;
    let stream = Stream::new(StreamFlags::NON_BLOCKING, None)
        .map_err(|e| anyhow!("Failed to create CUDA stream: {}", e))?;
    let function = module
        .get_function("mine_kernel")
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

    let network_target = compact_bits_to_target_bytes(difficulty);
    let target = target_override.unwrap_or(network_target);
    let target_dev = DeviceBuffer::from_slice(&target)?;

    let mut start_nonce: u64 = header.nonce;
    let mut last_rate_update = std::time::Instant::now();
    let mut hashes_since_update: u64 = 0;
    let mut last_console_update = std::time::Instant::now();
    let mut total_hashes: u64 = 0;

    loop {
        if cancel_flag.load(Ordering::Relaxed) {
            return Err(anyhow!("Mining cancelled"));
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
                target_dev.as_device_ptr(),
                found_flag.as_device_ptr(),
                found_nonce.as_device_ptr(),
                found_hash.as_device_ptr(),
                dag_dev.as_device_ptr(),
                dag_arc.len() as u64
            ))
            .map_err(|e| anyhow!("CUDA kernel launch failed: {}", e))?;
        }

        stream
            .synchronize()
            .map_err(|e| anyhow!("CUDA stream synchronization failed: {}", e))?;

        let mut flag_host = [0u32];
        found_flag.copy_to(&mut flag_host)?;

        hashes_since_update = hashes_since_update.saturating_add(batch_size);
        total_hashes = total_hashes.saturating_add(batch_size);

        let elapsed = last_rate_update.elapsed();
        if elapsed.as_millis() >= 50 {
            let rate = hashes_since_update as f64 / elapsed.as_secs_f64();
            if let Some(ref hr) = hashrate {
                if let Ok(mut hr_lock) = hr.try_lock() {
                    *hr_lock = rate;
                }
            }
            if last_console_update.elapsed().as_secs() >= 5 {
                let mhs = rate / 1_000_000.0;
                println!(
                    "[CUDA] Mining: {:.2} MH/s | Total: {} MH | Nonce: {}-{}",
                    mhs, total_hashes / 1_000_000, start_nonce, start_nonce + batch_size
                );
                last_console_update = std::time::Instant::now();
            }
            hashes_since_update = 0;
            last_rate_update = std::time::Instant::now();
        }

        if flag_host[0] != 0 {
            let mut nonce_host = [0u64];
            let mut gpu_header_hash = [0u8; 32];
            found_nonce.copy_to(&mut nonce_host)?;
            found_hash.copy_to(&mut gpu_header_hash)?;

            let nonce = nonce_host[0];

            if !hash_meets_target(&gpu_header_hash, &target) {
                // GPU sanity check failed (shouldn't happen)
                start_nonce = nonce.wrapping_add(1);
                continue;
            }

            let final_elapsed = last_rate_update.elapsed();
            if final_elapsed.as_secs_f64() > 0.0 {
                let final_rate = hashes_since_update as f64 / final_elapsed.as_secs_f64();
                if let Some(ref hr) = hashrate {
                    if let Ok(mut hr_lock) = hr.try_lock() {
                        *hr_lock = final_rate;
                    }
                }
            }

            return Ok((nonce, hex::encode(gpu_header_hash)));
        }

        start_nonce = start_nonce.wrapping_add(batch_size);
    }
}

fn compact_bits_to_target_bytes(bits: u32) -> [u8; 32] {
    let exponent = bits >> 24;
    let mantissa = bits & 0x007f_ffff;
    if mantissa == 0 {
        return [0u8; 32];
    }

    let target = if exponent <= 3 {
        U256::from(mantissa >> (8 * (3 - exponent)))
    } else {
        U256::from(mantissa) << (8 * (exponent - 3))
    };

    let mut out = [0u8; 32];
    target.to_big_endian(&mut out);
    out
}

fn hash_meets_target(hash: &[u8; 32], target: &[u8; 32]) -> bool {
    for i in 0..32 {
        if hash[i] < target[i] {
            return true;
        }
        if hash[i] > target[i] {
            return false;
        }
    }
    false  // hash == target is NOT valid; must be hash < target
}

fn encode_field<T: bincode::Encode>(value: &T) -> Result<Vec<u8>> {
    let config = bincode::config::standard().with_fixed_int_encoding(); // Use fixed-length encoding
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
    let dag_arc = get_or_generate_dag(epoch, index)?;

    println!("[CUDA] Uploading 4GB DAG to GPU and starting mining...");

    // Upload DAG to GPU (this is expensive - 4GB!)
    let dag_dev = DeviceBuffer::from_slice(dag_arc.as_ref()).map_err(|e| {
        anyhow!(
            "Failed to upload DAG to GPU: {}. Make sure you have at least 4GB VRAM.",
            e
        )
    })?;

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
    let module =
        Module::from_ptx(ptx, &[]).map_err(|e| anyhow!("Failed to load CUDA PTX module: {}", e))?;
    let stream = Stream::new(StreamFlags::NON_BLOCKING, None)
        .map_err(|e| anyhow!("Failed to create CUDA stream: {}", e))?;
    let function = module
        .get_function("mine_kernel")
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

    // Bitcoin-style target from compact bits (nBits)
    let target = compact_bits_to_target_bytes(difficulty);
    let target_dev = DeviceBuffer::from_slice(&target)?;

    let mut start_nonce: u64 = 0;
    let mut last_rate_update = std::time::Instant::now();
    let mut hashes_since_update: u64 = 0;
    let mut last_console_update = std::time::Instant::now();
    let mut total_hashes: u64 = 0;

    #[cfg(debug_assertions)]
    {
        println!(
            "[CUDA] Mining: {} blocks × {} threads = {} parallel",
            blocks, THREADS_PER_BLOCK, blocks * THREADS_PER_BLOCK
        );
        println!("[CUDA] Difficulty: 0x{:08x}, Target: {}", difficulty, hex::encode(&target[..8]));
    }

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
                target_dev.as_device_ptr(),
                found_flag.as_device_ptr(),
                found_nonce.as_device_ptr(),
                found_hash.as_device_ptr(),
                dag_dev.as_device_ptr(),
                dag_arc.len() as u64
            ))
            .map_err(|e| anyhow!("CUDA kernel launch failed: {}", e))?;
        }

        stream
            .synchronize()
            .map_err(|e| anyhow!("CUDA stream synchronization failed: {}", e))?;

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
                println!(
                    "[CUDA] Mining: {:.2} MH/s | Total: {} MH | Nonce range: {}-{}",
                    mhs,
                    total_hashes / 1_000_000,
                    start_nonce,
                    start_nonce + batch_size
                );
                last_console_update = std::time::Instant::now();
            }

            hashes_since_update = 0;
            last_rate_update = std::time::Instant::now();
        }

        if flag_host[0] != 0 {
            let mut nonce_host = [0u64];
            let mut gpu_header_hash = [0u8; 32];
            found_nonce.copy_to(&mut nonce_host)?;
            found_hash.copy_to(&mut gpu_header_hash)?;

            let nonce = nonce_host[0];

            if !hash_meets_target(&gpu_header_hash, &target) {
                return Err(anyhow!(
                    "GPU found nonce did not satisfy compact target bits=0x{:08x}",
                    difficulty
                ));
            }

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

            let block = Block {
                header: header.clone(),
                transactions: all_txs,
                hash: hex::encode(gpu_header_hash),
            };
            return Ok(block);
        }

        start_nonce = start_nonce.wrapping_add(batch_size);
    }
}
