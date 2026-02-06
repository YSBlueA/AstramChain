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

const DEFAULT_BATCH_SIZE: u64 = 1_000_000;
const THREADS_PER_BLOCK: u32 = 256;
const MAX_BLOCKS: u32 = 4096;

fn encode_field<T: bincode::Encode>(value: &T) -> Result<Vec<u8>> {
    let config = bincode::config::standard();
    Ok(bincode::encode_to_vec(value, config)?)
}

fn encode_varint_u64(mut value: u64) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let mut byte = (value & 0x7F) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if value == 0 {
            break;
        }
    }
    out
}

fn build_header_bytes(prefix: &[u8], nonce: u64, suffix: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(prefix.len() + 16 + suffix.len());
    out.extend_from_slice(prefix);
    out.extend_from_slice(&encode_varint_u64(nonce));
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
    let _ctx = cust::quick_init()?;

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
    let module = Module::from_ptx(ptx, &[])?;
    let stream = Stream::new(StreamFlags::NON_BLOCKING, None)?;
    let function = module.get_function("mine_kernel")?;

    let prefix_dev = DeviceBuffer::from_slice(&prefix)?;
    let suffix_dev = DeviceBuffer::from_slice(&suffix)?;

    let mut found_flag = DeviceBuffer::from_slice(&[0u32])?;
    let mut found_nonce = DeviceBuffer::from_slice(&[0u64])?;
    let mut found_hash = DeviceBuffer::from_slice(&[0u8; 32])?;

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
        if elapsed.as_secs() >= 1 {
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

            header.nonce = nonce_host[0];
            let hash = compute_header_hash(&header)?;
            let hash_hex = hex::encode(hash_host);
            if hash != hash_hex {
                return Err(anyhow!("GPU hash mismatch against CPU verification"));
            }
            if !hash.starts_with(&"0".repeat(difficulty as usize)) {
                return Err(anyhow!("GPU found nonce did not satisfy target"));
            }

            let block = Block {
                header: header.clone(),
                transactions: all_txs,
                hash,
            };
            return Ok(block);
        }

        start_nonce = start_nonce.wrapping_add(batch_size);
    }
}
