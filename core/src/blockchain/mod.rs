use crate::block::{Block, BlockHeader, compute_header_hash, compute_merkle_root};
use crate::db::{open_db, put_batch};
use crate::transaction::Transaction;
use crate::utxo::Utxo;
use anyhow::{Result, anyhow};
use bincode::config;
use chrono::Utc;
use hex;
use log;
use once_cell::sync::Lazy;
use primitive_types::U256;
use rocksdb::{DB, WriteBatch};
use crate::config::HALVING_INTERVAL;
use crate::config::initial_block_reward;

pub static BINCODE_CONFIG: Lazy<config::Configuration> = Lazy::new(|| config::standard());

/// Blockchain structure (disk-based RocksDB storage)
///
/// This structure manages the blockchain state including:
/// - Block storage and retrieval
/// - Transaction validation and UTXO management
/// - Chain tip tracking
/// - Balance and transaction queries
pub struct Blockchain {
    pub db: DB,
    pub chain_tip: Option<String>, // tip hash hex
    pub difficulty: u32,
    pub block_interval: i64,  // Target block generation interval (seconds)
    pub max_reorg_depth: u64, // Maximum allowed reorganization depth (security)
    pub max_future_block_time: i64, // Maximum seconds a block can be in the future
    pub enable_deep_reorg_alerts: bool, // Alert on deep reorgs (vs hard reject)
}

impl Blockchain {
    const POW_LIMIT_BITS: u32 = 0x1f7fffff; // Bitcoin-style compact bits: maximum target (easiest difficulty for testing)
    const RETARGET_WINDOW: u64 = 24; // DWG3 window: use last 24 blocks

    fn compact_to_target(bits: u32) -> U256 {
        let exponent = bits >> 24;
        let mantissa = bits & 0x007f_ffff;
        if mantissa == 0 {
            return U256::zero();
        }

        if exponent <= 3 {
            U256::from(mantissa >> (8 * (3 - exponent)))
        } else {
            U256::from(mantissa) << (8 * (exponent - 3))
        }
    }

    fn target_to_compact(target: U256) -> u32 {
        if target.is_zero() {
            return 0;
        }

        let mut bytes = [0u8; 32];
        target.to_big_endian(&mut bytes);
        let first_non_zero = bytes.iter().position(|&b| b != 0).unwrap_or(31);
        let mut size = (32 - first_non_zero) as u32;

        let mut mantissa: u32 = if size <= 3 {
            let mut v: u32 = 0;
            for i in first_non_zero..32 {
                v = (v << 8) | bytes[i] as u32;
            }
            v << (8 * (3 - size))
        } else {
            ((bytes[first_non_zero] as u32) << 16)
                | ((bytes[first_non_zero + 1] as u32) << 8)
                | (bytes[first_non_zero + 2] as u32)
        };

        if (mantissa & 0x0080_0000) != 0 {
            mantissa >>= 8;
            size += 1;
        }

        (size << 24) | (mantissa & 0x007f_ffff)
    }

    fn hash_to_u256(hash_hex: &str) -> Result<U256> {
        let normalized = hash_hex.strip_prefix("0x").unwrap_or(hash_hex);
        let bytes = hex::decode(normalized)?;
        if bytes.len() != 32 {
            return Err(anyhow!(
                "invalid hash length for PoW comparison: expected 32 bytes, got {}",
                bytes.len()
            ));
        }
        Ok(U256::from_big_endian(&bytes))
    }

    fn pow_limit_target() -> U256 {
        Self::compact_to_target(Self::POW_LIMIT_BITS)
    }

    fn is_valid_pow(hash_hex: &str, bits: u32) -> Result<bool> {
        let hash = Self::hash_to_u256(hash_hex)?;
        let target = Self::compact_to_target(bits);
        if target.is_zero() {
            return Ok(false);
        }
        Ok(hash < target)
    }

    pub fn new(db_path: &str) -> Result<Self> {
        let db = open_db(db_path)?;
        // load tip if exists
        let tip = db.get(b"tip")?;
        let chain_tip = tip.map(|v| String::from_utf8(v).unwrap());

        // Debug: Log chain tip information
        if let Some(ref tip_hash) = chain_tip {
            log::info!("Found chain tip in DB: {}", tip_hash);
        } else {
            log::info!("No chain tip found in DB (fresh database)");
        }

        // Load current difficulty from chain tip
        let difficulty = if let Some(ref tip_hash) = chain_tip {
            // Try to load the tip block header
            match db.get(format!("b:{}", tip_hash).as_bytes()) {
                Ok(Some(blob)) => {
                    match bincode::decode_from_slice::<Block, _>(&blob, *BINCODE_CONFIG) {
                        Ok((block, _)) => {
                            log::info!(
                                "Loaded tip block #{} (hash: {})",
                                block.header.index,
                                tip_hash
                            );
                            block.header.difficulty
                        }
                        Err(e) => {
                            log::error!("Failed to decode tip block: {}", e);
                            Self::POW_LIMIT_BITS
                        }
                    }
                }
                Ok(None) => {
                    log::error!("Tip block '{}' not found in database!", tip_hash);
                    Self::POW_LIMIT_BITS
                }
                Err(e) => {
                    log::error!("Failed to read tip block from DB: {}", e);
                    Self::POW_LIMIT_BITS
                }
            }
        } else {
            // No chain exists yet, use default
            Self::POW_LIMIT_BITS
        };

        log::info!("Blockchain initialized with difficulty: {}", difficulty);

        Ok(Blockchain {
            db,
            chain_tip,
            difficulty,
            block_interval: 60, // Target: 60 seconds per block (auto-adjusts based on network hashrate)
            max_reorg_depth: 100, // Maximum 100 blocks deep reorganization (security limit)
            max_future_block_time: 7200, // Max 2 hours in the future (clock drift tolerance)
            enable_deep_reorg_alerts: true, // Alert on suspicious reorgs
        })
    }

    /// Helper: Iterate over all blocks efficiently
    fn get_all_blocks_cached(&self) -> Result<Vec<Block>> {
        // This could be further optimized with caching in production
        self.get_all_blocks()
    }

    /// Create genesis block (with a single coinbase transaction)
    pub fn create_genesis(&mut self, address: &str) -> Result<String> {
        if self.chain_tip.is_some() {
            return Err(anyhow!("chain already exists"));
        }
        let cb = Transaction::coinbase(address, U256::from(50));

        let merkle = compute_merkle_root(&vec![cb.txid.clone()]);
        let header = BlockHeader {
            index: 0,
            previous_hash: "0".repeat(64),
            merkle_root: merkle,
            timestamp: Utc::now().timestamp(),
            nonce: 0,
            difficulty: self.difficulty,
        };
        let hash = compute_header_hash(&header)?;
        let block = Block {
            header,
            transactions: vec![cb.clone()],
            hash: hash.clone(),
        };

        // commit atomically
        let mut batch = WriteBatch::default();
        // Store complete block (header + transactions)
        let block_blob = bincode::encode_to_vec(&block, *BINCODE_CONFIG)?;
        batch.put(format!("b:{}", hash).as_bytes(), &block_blob);
        // tx
        let tx_blob = bincode::encode_to_vec(&cb, *BINCODE_CONFIG)?;
        batch.put(format!("t:{}", cb.txid).as_bytes(), &tx_blob);

        for (i, out) in cb.outputs.iter().enumerate() {
            let utxo = Utxo::new(cb.txid.clone(), i as u32, out.to.clone(), out.amount());

            let utxo_blob = bincode::encode_to_vec(&utxo, *BINCODE_CONFIG)?;
            batch.put(format!("u:{}:{}", cb.txid, i).as_bytes(), &utxo_blob);
        }

        // index
        batch.put(format!("i:0").as_bytes(), hash.as_bytes());
        batch.put(b"tip", hash.as_bytes());

        put_batch(&self.db, batch)?;
        self.chain_tip = Some(hash.clone());
        Ok(hash)
    }

    /// Validate and insert a fork block (allows fork without chain_tip check)
    /// This is used for chain reorganization scenarios
    pub fn validate_fork_block(&mut self, block: &Block) -> Result<()> {
        // 1) header hash match
        let computed = compute_header_hash(&block.header)?;
        if computed != block.hash {
            return Err(anyhow!(
                "header hash mismatch: computed {} != block.hash {}",
                computed,
                block.hash
            ));
        }

        // 1.5) Checkpoint policy anchors (official chain protection)
        if !crate::checkpoint::validate_against_checkpoints(block.header.index, &block.hash) {
            return Err(anyhow!(
                "checkpoint policy violation at height {} for hash {}",
                block.header.index,
                block.hash
            ));
        }

        // 2) Proof-of-Work verification
        if !Self::is_valid_pow(&block.hash, block.header.difficulty)? {
            return Err(anyhow!(
                "invalid PoW at block {}: hash does not satisfy bits 0x{:08x}",
                block.header.index,
                block.header.difficulty
            ));
        }

        // 3) Expected difficulty check
        if block.header.index > 0 {
            let expected = self.calculate_adjusted_difficulty(block.header.index)?;
            if block.header.difficulty != expected {
                return Err(anyhow!(
                    "difficulty mismatch at height {}: expected 0x{:08x}, got 0x{:08x}",
                    block.header.index,
                    expected,
                    block.header.difficulty
                ));
            }
        }

        // 4) previous exists (but allow fork - no chain_tip check)
        if block.header.index > 0 {
            let prev_key = format!("b:{}", block.header.previous_hash);
            if self.db.get(prev_key.as_bytes())?.is_none() {
                return Err(anyhow!(
                    "previous header not found: {}",
                    block.header.previous_hash
                ));
            }
            // NOTE: We DO NOT check against chain_tip here - that's the point of fork blocks
        }

        // 5) Future timestamp check
        let now = Utc::now().timestamp();
        if block.header.timestamp > now + self.max_future_block_time {
            return Err(anyhow!(
                "block timestamp too far in future: {} > {}",
                block.header.timestamp,
                now + self.max_future_block_time
            ));
        }

        // 6) Difficulty sanity progression check
        if block.header.index > 0 {
            let prev_key = format!("b:{}", block.header.previous_hash);
            if let Ok(Some(prev_bytes)) = self.db.get(prev_key.as_bytes()) {
                if let Ok((prev_block, _)) =
                    bincode::decode_from_slice::<Block, _>(&prev_bytes, *BINCODE_CONFIG)
                {
                    let prev_target = Self::compact_to_target(prev_block.header.difficulty);
                    let current_target = Self::compact_to_target(block.header.difficulty);

                    if current_target.is_zero()
                        || (!prev_target.is_zero()
                            && ((current_target > prev_target
                                && (current_target / prev_target) > U256::from(4u8))
                                || (current_target < prev_target
                                    && (prev_target / current_target) > U256::from(4u8))))
                    {
                        return Err(anyhow!(
                            "difficulty target changed too aggressively at block {}",
                            block.header.index
                        ));
                    }
                }
            }
        }

        // 7) Merkle check
        let txids: Vec<String> = block.transactions.iter().map(|t| t.txid.clone()).collect();
        let merkle = compute_merkle_root(&txids);
        if merkle != block.header.merkle_root {
            return Err(anyhow!("merkle mismatch"));
        }

        // 8) Median-Time-Past
        if block.header.index > 0 {
            self.validate_median_time_past(block)?;
        }

        // 9) Store the fork block in DB (without updating chain_tip)
        let mut batch = WriteBatch::default();
        
        // Store complete block
        let block_blob = bincode::encode_to_vec(block, *BINCODE_CONFIG)?;
        batch.put(format!("b:{}", block.hash).as_bytes(), &block_blob);

        // Store transactions
        for tx in &block.transactions {
            let tx_blob = bincode::encode_to_vec(tx, *BINCODE_CONFIG)?;
            batch.put(format!("t:{}", tx.txid).as_bytes(), &tx_blob);
        }
        
        // Note: We DO NOT update i:{index} here because that would conflict with main chain
        // The index will be updated during reorganization if this becomes the main chain
        // Note: We do NOT update UTXO set here - that happens during reorganization
        
        put_batch(&self.db, batch)?;
        
        log::debug!("Fork block #{} stored in DB (hash: {})", block.header.index, &block.hash[..16]);
        
        Ok(())
    }

    /// validate and insert block (core of migration/consensus)
    pub fn validate_and_insert_block(&mut self, block: &Block) -> Result<()> {
        // 0) Duplicate block check: skip if already stored
        let block_key = format!("b:{}", block.hash);
        if self.db.get(block_key.as_bytes())?.is_some() {
            log::debug!(
                "Block #{} ({}) already exists, skipping insertion",
                block.header.index,
                &block.hash[..16]
            );
            return Ok(());
        }

        // 1) header hash match
        let computed = compute_header_hash(&block.header)?;
        if computed != block.hash {
            return Err(anyhow!(
                "header hash mismatch: computed {} != block.hash {}",
                computed,
                block.hash
            ));
        }

        // 1.5) Checkpoint policy anchors (official chain protection)
        if !crate::checkpoint::validate_against_checkpoints(block.header.index, &block.hash) {
            return Err(anyhow!(
                "checkpoint policy violation at height {} for hash {}",
                block.header.index,
                block.hash
            ));
        }

        // 2) Proof-of-Work verification
        if !Self::is_valid_pow(&block.hash, block.header.difficulty)? {
            return Err(anyhow!(
                "invalid PoW at block {}: hash does not satisfy bits 0x{:08x}",
                block.header.index,
                block.header.difficulty
            ));
        }

        // 3) Expected difficulty check
        if block.header.index > 0 {
            let expected = self.calculate_adjusted_difficulty(block.header.index)?;
            if block.header.difficulty != expected {
                return Err(anyhow!(
                    "difficulty mismatch at height {}: expected 0x{:08x}, got 0x{:08x}",
                    block.header.index,
                    expected,
                    block.header.difficulty
                ));
            }
        }

        // 4) previous exists + longest chain rule
        if block.header.index > 0 {
            let prev_key = format!("b:{}", block.header.previous_hash);
            if self.db.get(prev_key.as_bytes())?.is_none() {
                return Err(anyhow!(
                    "previous header not found: {}",
                    block.header.previous_hash
                ));
            }

            // longest chain rule
            if let Some(tip_hash) = self.chain_tip.clone() {
                if self.load_header(&tip_hash)?.is_none() {
                    log::warn!(
                        "⚠️  Tip {} is missing before block validation; recovering tip first...",
                        tip_hash
                    );
                    self.recover_tip()?;
                }

                if let Some(ref recovered_tip_hash) = self.chain_tip {
                    if &block.header.previous_hash != recovered_tip_hash {
                        return Err(anyhow!(
                            "fork detected: prev {} != tip {}",
                            block.header.previous_hash,
                            recovered_tip_hash
                        ));
                    }
                }
            }
        }

        // 5) Future timestamp check
        let now = Utc::now().timestamp();
        if block.header.timestamp > now + self.max_future_block_time {
            return Err(anyhow!(
                "block timestamp too far in future: {} > {}",
                block.header.timestamp,
                now + self.max_future_block_time
            ));
        }

        // 6) Difficulty sanity progression check
        if block.header.index > 0 {
            let prev_key = format!("b:{}", block.header.previous_hash);
            if let Ok(Some(prev_bytes)) = self.db.get(prev_key.as_bytes()) {

                if let Ok((prev_block, _)) =
                    bincode::decode_from_slice::<Block, _>(&prev_bytes, *BINCODE_CONFIG)
                {
                    let prev_target = Self::compact_to_target(prev_block.header.difficulty);
                    let current_target = Self::compact_to_target(block.header.difficulty);

                    if current_target.is_zero()
                        || (!prev_target.is_zero()
                            && ((current_target > prev_target
                                && (current_target / prev_target) > U256::from(4u8))
                                || (current_target < prev_target
                                    && (prev_target / current_target) > U256::from(4u8))))
                    {
                        return Err(anyhow!(
                            "difficulty target changed too aggressively at block {}",
                            block.header.index
                        ));
                    }
                }
            }
        }

        // 7) Merkle check
        let txids: Vec<String> = block.transactions.iter().map(|t| t.txid.clone()).collect();
        let merkle = compute_merkle_root(&txids);
        if merkle != block.header.merkle_root {
            return Err(anyhow!("merkle mismatch"));
        }

        // 8) Median-Time-Past
        if block.header.index > 0 {
            self.validate_median_time_past(block)?;
        }

        // 9) transaction validation (기존 로직 유지)
        let mut batch = WriteBatch::default();

        if block.transactions.is_empty() {
            return Err(anyhow!("empty block"));
        }

        let coinbase = &block.transactions[0];
        if !coinbase.inputs.is_empty() {
            return Err(anyhow!("coinbase must have no inputs"));
        }

        let mut total_fees = U256::zero();
        // Track UTXOs created by earlier transactions in this block so that
        // chained transactions (tx B spends output of tx A in the same block)
        // can be validated before the batch is committed to the DB.
        let mut block_utxos: std::collections::HashMap<String, Utxo> =
            std::collections::HashMap::new();

        for (i, tx) in block.transactions.iter().enumerate() {
            if !tx.verify_signatures()? {
                return Err(anyhow!("tx signature invalid: {}", tx.txid));
            }

            if i == 0 {
                // coinbase 저장
                let tx_blob = bincode::encode_to_vec(tx, *BINCODE_CONFIG)?;
                batch.put(format!("t:{}", tx.txid).as_bytes(), &tx_blob);
                for (v, out) in tx.outputs.iter().enumerate() {
                    let utxo = Utxo::new(tx.txid.clone(), v as u32, out.to.to_lowercase(), out.amount());
                    let ublob = bincode::encode_to_vec(&utxo, *BINCODE_CONFIG)?;
                    batch.put(format!("u:{}:{}", tx.txid, v).as_bytes(), &ublob);
                    block_utxos.insert(format!("u:{}:{}", tx.txid, v), utxo);
                }
                continue;
            }

            let mut input_sum = U256::zero();
            let mut used_utxos = std::collections::HashSet::new();

            for inp in &tx.inputs {
                let ukey = format!("u:{}:{}", inp.txid, inp.vout);

                if !used_utxos.insert(ukey.clone()) {
                    return Err(anyhow!("duplicate input in tx {}", tx.txid));
                }

                // Check UTXOs created by earlier transactions in this block first,
                // then fall back to committed DB (handles chained mempool transactions).
                let u = if let Some(pending) = block_utxos.remove(&ukey) {
                    // UTXO was created by a previous tx in this same block
                    pending
                } else {
                    match self.db.get(ukey.as_bytes())? {
                        Some(blob) => {
                            let (u, _): (Utxo, usize) =
                                bincode::decode_from_slice(&blob, *BINCODE_CONFIG)?;
                            batch.delete(ukey.as_bytes());
                            u
                        }
                        None => {
                            return Err(anyhow!("referenced utxo not found"));
                        }
                    }
                };

                let input_address =
                    crate::crypto::address_from_pubkey_hex(&inp.pubkey)
                        .map_err(|e| anyhow!("invalid pubkey address: {}", e))?;

                if input_address.to_lowercase() != u.to.to_lowercase() {
                    return Err(anyhow!("UTXO ownership verification failed"));
                }

                input_sum = input_sum + u.amount();
            }

            let mut output_sum = U256::zero();
            for out in &tx.outputs {
                output_sum = output_sum + out.amount();
            }

            if output_sum > input_sum {
                return Err(anyhow!("outputs exceed inputs"));
            }

            let fee = input_sum - output_sum;
            total_fees = total_fees + fee;

            let tx_blob = bincode::encode_to_vec(tx, *BINCODE_CONFIG)?;
            let min_fee = crate::config::calculate_min_fee(tx_blob.len());
            if fee < min_fee {
                return Err(anyhow!("transaction fee too low"));
            }

            batch.put(format!("t:{}", tx.txid).as_bytes(), &tx_blob);
            for (v, out) in tx.outputs.iter().enumerate() {
                let utxo = Utxo::new(tx.txid.clone(), v as u32, out.to.to_lowercase(), out.amount());
                let ublob = bincode::encode_to_vec(&utxo, *BINCODE_CONFIG)?;
                batch.put(format!("u:{}:{}", tx.txid, v).as_bytes(), &ublob);
                block_utxos.insert(format!("u:{}:{}", tx.txid, v), utxo);
            }
        }

        // ⭐ Block reward validation
        let coinbase_output: U256 = coinbase.outputs.iter().map(|o| o.amount()).fold(U256::zero(), |a, b| a + b);
        let expected_reward = self.get_block_reward(block.header.index);
        if coinbase_output > expected_reward + total_fees {
            return Err(anyhow!(
                "invalid coinbase reward: got {}, max {}",
                coinbase_output,
                expected_reward + total_fees
            ));
        }

        // persist block
        let block_blob = bincode::encode_to_vec(&block, *BINCODE_CONFIG)?;
        batch.put(format!("b:{}", block.hash).as_bytes(), &block_blob);
        batch.put(format!("i:{}", block.header.index).as_bytes(), block.hash.as_bytes());
        batch.put(b"tip", block.hash.as_bytes());

        put_batch(&self.db, batch)?;
        self.chain_tip = Some(block.hash.clone());
        // Keep bc.difficulty in sync with the accepted chain tip so /status
        // and calculate_adjusted_difficulty(next) both see the latest value.
        self.difficulty = block.header.difficulty;

        Ok(())
    }

    /// helper: load block header by hash
    pub fn load_header(&self, hash: &str) -> Result<Option<BlockHeader>> {
        if let Some(blob) = self.db.get(format!("b:{}", hash).as_bytes())? {
            let (block, _): (Block, usize) = bincode::decode_from_slice(&blob, *BINCODE_CONFIG)?;
            return Ok(Some(block.header));
        }
        Ok(None)
    }

    /// load tx by id
    pub fn load_tx(&self, txid: &str) -> Result<Option<Transaction>> {
        if let Some(blob) = self.db.get(format!("t:{}", txid).as_bytes())? {
            let (t, _): (Transaction, usize) = bincode::decode_from_slice(&blob, *BINCODE_CONFIG)?;
            return Ok(Some(t));
        }
        Ok(None)
    }

    /// get balance by scanning UTXO set (use get_address_balance_from_db instead)
    #[deprecated(note = "Use get_address_balance_from_db instead")]
    pub fn get_balance(&self, address: &str) -> Result<U256, Box<dyn std::error::Error>> {
        Ok(self.get_address_balance_from_db(address)?)
    }

    /// Determine next block index based on current tip
    pub fn get_next_index(&self) -> Result<u64> {
        if let Some(ref tip_hash) = self.chain_tip {
            if let Some(prev) = self.load_header(tip_hash)? {
                // assume BlockHeader.index is u64 or can be cast; adjust if different
                return Ok(prev.index + 1);
            }
        }
        Ok(0)
    }

    /// Validate Median-Time-Past (MTP) - block timestamp must be greater than median of last 11 blocks
    /// This prevents miners from lying about timestamps to manipulate difficulty
    fn validate_median_time_past(&self, block: &Block) -> Result<()> {
        const MTP_SPAN: usize = 11; // Bitcoin uses 11 blocks

        let mut timestamps = Vec::new();
        let mut current_hash = block.header.previous_hash.clone();

        // Collect up to 11 previous block timestamps
        for _ in 0..MTP_SPAN {
            if let Some(blk) = self.load_block(&current_hash)? {
                timestamps.push(blk.header.timestamp);
                if blk.header.index == 0 {
                    break; // Reached genesis
                }
                current_hash = blk.header.previous_hash.clone();
            } else {
                break;
            }
        }

        if timestamps.is_empty() {
            // No previous blocks, skip MTP check
            return Ok(());
        }

        // Calculate median
        timestamps.sort_unstable();
        let median = if timestamps.len() % 2 == 0 {
            (timestamps[timestamps.len() / 2 - 1] + timestamps[timestamps.len() / 2]) / 2
        } else {
            timestamps[timestamps.len() / 2]
        };

        // Block timestamp must be strictly greater than MTP
        if block.header.timestamp <= median {
            return Err(anyhow!(
                "Block timestamp {} violates Median-Time-Past {} (must be > MTP)",
                block.header.timestamp,
                median
            ));
        }

        Ok(())
    }

    /// Calculate next difficulty using DWG3 (Dark Gravity Wave v3 style)
    /// - Recalculates every block
    /// - Uses the last `RETARGET_WINDOW` blocks
    /// - Averages historical targets, then scales by measured timespan
    pub fn calculate_adjusted_difficulty(&self, current_index: u64) -> Result<u32> {
        // No adjustment until enough history is available.
        // `current_index` is the height to be mined next.
        if current_index < Self::RETARGET_WINDOW {
            return Ok(self.difficulty);
        }

        let pow_limit = Self::pow_limit_target();
        let mut past_target_avg = U256::zero();
        let mut newest_time: Option<i64> = None;
        let mut oldest_time: Option<i64> = None;

        for i in 1..=Self::RETARGET_WINDOW {
            let height = current_index - i;
            let hash_bytes = match self.db.get(format!("i:{}", height).as_bytes())? {
                Some(v) => v,
                None => {
                    log::warn!("DWG3: missing index entry at height {}", height);
                    return Ok(self.difficulty);
                }
            };

            let hash = String::from_utf8(hash_bytes)?;
            let header = match self.load_header(&hash)? {
                Some(h) => h,
                None => {
                    log::warn!("DWG3: missing header at height {}", height);
                    return Ok(self.difficulty);
                }
            };

            let mut target = Self::compact_to_target(header.difficulty);
            if target.is_zero() {
                target = pow_limit;
            }

            if i == 1 {
                past_target_avg = target;
                newest_time = Some(header.timestamp);
            } else {
                past_target_avg = (past_target_avg.saturating_mul(U256::from((i - 1) as u64))
                    .saturating_add(target))
                    / U256::from(i as u64);
            }

            oldest_time = Some(header.timestamp);
        }

        let newest_time = newest_time.unwrap_or(0);
        let oldest_time = oldest_time.unwrap_or(newest_time);

        // DWG3-style timespan clamping to reduce oscillation.
        let raw_actual_timespan = (newest_time - oldest_time).max(1);
        let target_timespan = (self.block_interval * Self::RETARGET_WINDOW as i64).max(1);
        let clamped_actual_timespan = raw_actual_timespan.clamp(target_timespan / 3, target_timespan * 3);

        let mut new_target = past_target_avg
            .saturating_mul(U256::from(clamped_actual_timespan as u64))
            / U256::from(target_timespan as u64);

        if new_target.is_zero() {
            new_target = U256::one();
        }
        if new_target > pow_limit {
            new_target = pow_limit;
        }

        let previous_bits = match self.db.get(format!("i:{}", current_index - 1).as_bytes())? {
            Some(v) => {
                let hash = String::from_utf8(v)?;
                self.load_header(&hash)?
                    .map(|h| h.difficulty)
                    .unwrap_or(self.difficulty)
            }
            None => self.difficulty,
        };

        // Clamp the result to the same 4× limit that block validation enforces,
        // so DWG3 can never produce a value that the validator would reject.
        let prev_target = Self::compact_to_target(previous_bits);
        if !prev_target.is_zero() {
            let max_increase = prev_target.saturating_mul(U256::from(4u8));
            let min_decrease = prev_target / U256::from(4u8);
            if new_target > max_increase {
                new_target = max_increase;
            } else if new_target < min_decrease {
                new_target = min_decrease;
            }
            if new_target.is_zero() {
                new_target = U256::one();
            }
        }

        let next_bits = Self::target_to_compact(new_target);

        log::debug!(
            "DWG3 retarget @{}: bits 0x{:08x} -> 0x{:08x}, actual={}s, target={}s, avg={:.1}s/block",
            current_index,
            previous_bits,
            next_bits,
            raw_actual_timespan,
            target_timespan,
            raw_actual_timespan as f64 / Self::RETARGET_WINDOW as f64
        );

        Ok(next_bits)
    }

    pub fn get_utxos(&self, address: &str) -> Result<Vec<Utxo>> {
        let mut utxos = Vec::new();
        let iter = self.db.iterator(rocksdb::IteratorMode::Start);

        for item in iter {
            let (key, value) = item?;
            let key_str = String::from_utf8_lossy(&key);

            // UTXO key: u:{txid}:{vout}
            if key_str.starts_with("u:") {
                let (utxo, _): (Utxo, usize) = bincode::decode_from_slice(&value, *BINCODE_CONFIG)?;
                if utxo.to == address {
                    utxos.push(utxo);
                }
            }
        }

        Ok(utxos)
    }

    /// Count transactions stored in DB (keys starting with `t:`)
    pub fn count_transactions(&self) -> Result<usize> {
        let mut count: usize = 0;
        let iter = self.db.iterator(rocksdb::IteratorMode::Start);
        for item in iter {
            let (k, _v) = item?;
            let key_str = String::from_utf8_lossy(&k);
            if key_str.starts_with("t:") {
                count += 1;
            }
        }
        Ok(count)
    }

    /// Load all blocks from DB by iterating through block indices
    pub fn get_all_blocks(&self) -> Result<Vec<Block>> {
        let mut blocks = Vec::new();
        let mut index = 0u64;

        loop {
            let key = format!("i:{}", index);
            match self.db.get(key.as_bytes())? {
                Some(hash_bytes) => {
                    let hash = String::from_utf8(hash_bytes)?;

                    // Load complete block (with transactions) by hash
                    if let Some(blob) = self.db.get(format!("b:{}", hash).as_bytes())? {
                        let (block, _): (Block, usize) =
                            bincode::decode_from_slice(&blob, *BINCODE_CONFIG)?;
                        blocks.push(block);
                    }
                    index += 1;
                }
                None => {
                    // No more blocks at this index
                    break;
                }
            }
        }

        Ok(blocks)
    }

    /// Get blocks in a specific height range (inclusive)
    pub fn get_blocks_range(&self, from_height: u64, to_height: Option<u64>) -> Result<Vec<Block>> {
        let mut blocks = Vec::new();
        let mut index = from_height;

        loop {
            // Stop if we've reached the to_height limit
            if let Some(to) = to_height {
                if index > to {
                    break;
                }
            }

            let key = format!("i:{}", index);
            match self.db.get(key.as_bytes())? {
                Some(hash_bytes) => {
                    let hash = String::from_utf8(hash_bytes)?;

                    // Load complete block (with transactions) by hash
                    if let Some(blob) = self.db.get(format!("b:{}", hash).as_bytes())? {
                        let (block, _): (Block, usize) =
                            bincode::decode_from_slice(&blob, *BINCODE_CONFIG)?;
                        blocks.push(block);
                    }
                    index += 1;
                }
                None => {
                    // No more blocks at this index
                    break;
                }
            }
        }

        Ok(blocks)
    }

    pub fn get_transaction(&self, txid: &str) -> anyhow::Result<Option<(Transaction, usize)>> {
        let blocks = self.get_all_blocks()?;

        for block in blocks {
            for tx in block.transactions {
                if tx.txid == txid {
                    return Ok(Some((tx, block.header.index as usize)));
                }
            }
        }

        Ok(None)
    }

    /// Calculate total transaction volume from all outputs in DB (in ram)
    pub fn calculate_total_volume(&self) -> Result<U256> {
        let mut total = U256::zero();
        let iter = self.db.iterator(rocksdb::IteratorMode::Start);

        for item in iter {
            let (k, v) = item?;
            let key_str = String::from_utf8_lossy(&k);

            // Iterate through all transaction outputs: u:{txid}:{vout}
            if key_str.starts_with("u:") {
                let (utxo, _): (Utxo, usize) = bincode::decode_from_slice(&v, *BINCODE_CONFIG)?;
                total = total + utxo.amount();
            }
        }

        Ok(total)
    }

    /// Get address balance (sum of unspent outputs) from DB
    pub fn get_address_balance_from_db(&self, address: &str) -> Result<U256> {
        let mut balance = U256::zero();
        let iter = self.db.iterator(rocksdb::IteratorMode::Start);

        for item in iter {
            let (key, value) = item?;
            let key_str = String::from_utf8_lossy(&key);

            // UTXO key: u:{txid}:{vout}
            if key_str.starts_with("u:") {
                match bincode::decode_from_slice::<Utxo, _>(&value, *BINCODE_CONFIG) {
                    Ok((utxo, _)) => {
                        if utxo.to == address {
                            let amount = utxo.amount();
                            balance = balance + amount;
                        }
                    }
                    Err(e) => {
                        log::warn!("Failed to decode UTXO at {}: {}", key_str, e);
                    }
                }
            }
        }
        Ok(balance)
    }

    /// Get all addresses with their UTXO balances (for richlist)
    pub fn get_all_address_balances(&self) -> Result<Vec<(String, U256)>> {
        let mut balances: std::collections::HashMap<String, U256> = std::collections::HashMap::new();
        let iter = self.db.iterator(rocksdb::IteratorMode::Start);

        for item in iter {
            let (key, value) = item?;
            let key_str = String::from_utf8_lossy(&key);

            if key_str.starts_with("u:") {
                match bincode::decode_from_slice::<Utxo, _>(&value, *BINCODE_CONFIG) {
                    Ok((utxo, _)) => {
                        let entry = balances.entry(utxo.to.clone()).or_insert_with(U256::zero);
                        *entry = *entry + utxo.amount();
                    }
                    Err(e) => {
                        log::warn!("Failed to decode UTXO at {}: {}", key_str, e);
                    }
                }
            }
        }

        let mut result: Vec<(String, U256)> = balances.into_iter().collect();
        result.sort_by(|a, b| b.1.cmp(&a.1));
        Ok(result)
    }

    /// Get total received amount for address (all outputs to this address)
    pub fn get_address_received_from_db(&self, address: &str) -> Result<U256> {
        let mut total = U256::zero();
        let blocks = self.get_all_blocks_cached()?;

        for block in blocks {
            for tx in block.transactions {
                for output in &tx.outputs {
                    if output.to == address {
                        total = total + output.amount();
                    }
                }
            }
        }

        Ok(total)
    }

    /// Get total sent amount for address (all transaction outputs, excluding coinbase inputs)
    pub fn get_address_sent_from_db(&self, address: &str) -> Result<U256> {
        let mut total = U256::zero();
        let blocks = self.get_all_blocks_cached()?;

        for block in blocks {
            for tx in block.transactions {
                // Skip coinbase transactions (first tx in block)
                if !tx.inputs.is_empty() {
                    // Check if any input comes from this address
                    let is_sender = tx.inputs.iter().any(|input| input.pubkey == address);

                    if is_sender {
                        // Sum all outputs from this transaction
                        for output in &tx.outputs {
                            total = total + output.amount();
                        }
                    }
                }
            }
        }

        Ok(total)
    }

    /// Get transaction history for address (newest first)
    /// Returns: (txid, block_height, timestamp, direction, amount_wei, counterpart)
    pub fn get_address_transactions_from_db(
        &self,
        address: &str,
    ) -> Result<Vec<(String, u64, i64, String, U256, String)>> {
        let blocks = self.get_all_blocks_cached()?;
        let mut results: Vec<(String, u64, i64, String, U256, String)> = Vec::new();
        let mut seen_txids = std::collections::HashSet::new();

        for block in blocks {
            let height = block.header.index;
            for tx in block.transactions {
                let is_receiver = tx.outputs.iter().any(|o| o.to == address);
                let is_sender = tx.inputs.iter().any(|i| i.pubkey == address);

                if !is_receiver && !is_sender {
                    continue;
                }
                if !seen_txids.insert(tx.txid.clone()) {
                    continue;
                }

                if is_sender {
                    // One entry per unique recipient (excluding change back to self)
                    for output in &tx.outputs {
                        if output.to != address {
                            results.push((
                                tx.txid.clone(),
                                height,
                                tx.timestamp,
                                "send".to_string(),
                                output.amount(),
                                output.to.clone(),
                            ));
                        }
                    }
                    // If all outputs go back to self (edge case), record as self-send
                    if tx.outputs.iter().all(|o| o.to == address) {
                        let total: U256 = tx.outputs.iter().fold(U256::zero(), |acc, o| acc + o.amount());
                        results.push((
                            tx.txid.clone(),
                            height,
                            tx.timestamp,
                            "send".to_string(),
                            total,
                            address.to_string(),
                        ));
                    }
                } else {
                    // Pure receiver
                    let received: U256 = tx
                        .outputs
                        .iter()
                        .filter(|o| o.to == address)
                        .fold(U256::zero(), |acc, o| acc + o.amount());
                    let sender = tx
                        .inputs
                        .first()
                        .map(|i| i.pubkey.clone())
                        .unwrap_or_else(|| "coinbase".to_string());
                    results.push((
                        tx.txid.clone(),
                        height,
                        tx.timestamp,
                        "receive".to_string(),
                        received,
                        sender,
                    ));
                }
            }
        }

        // Newest first
        results.sort_by(|a, b| b.1.cmp(&a.1).then(b.2.cmp(&a.2)));
        Ok(results)
    }

    /// Get transaction count for address
    pub fn get_address_transaction_count_from_db(&self, address: &str) -> Result<usize> {
        let blocks = self.get_all_blocks_cached()?;
        let mut seen_txids = std::collections::HashSet::new();

        for block in blocks {
            for tx in block.transactions {
                // Check if address is involved (sender or receiver)
                let is_receiver = tx.outputs.iter().any(|output| output.to == address);
                let is_sender = tx.inputs.iter().any(|input| input.pubkey == address);

                // Count each unique transaction only once
                if (is_receiver || is_sender) && seen_txids.insert(tx.txid.clone()) {
                    // Counter automatically incremented by HashSet
                }
            }
        }

        Ok(seen_txids.len())
    }

    /// Calculate total chain work (cumulative difficulty) from genesis to given block
    /// Higher difficulty blocks contribute more work
    pub fn calculate_chain_work(&self, block_hash: &str) -> Result<u64> {
        // For simplicity and overflow prevention, use height as work metric
        // In production, could use sum of (2^256 / target) but this is sufficient
        // for longest chain rule when difficulty is relatively stable
        
        let block = self.load_block(block_hash)?;
        if let Some(block) = block {
            // Chain work = block height + 1 (genesis = 1, block 1 = 2, etc.)
            // This ensures longer chain always wins when difficulty is similar
            Ok(block.header.index + 1)
        } else {
            Err(anyhow!("Block not found: {}", block_hash))
        }
    }

    /// Get block height (index) for a given block hash
    pub fn get_block_height(&self, block_hash: &str) -> Result<Option<u64>> {
        if let Some(block) = self.load_block(block_hash)? {
            Ok(Some(block.header.index))
        } else {
            Ok(None)
        }
    }

    /// Load complete block by hash
    pub fn load_block(&self, hash: &str) -> Result<Option<Block>> {
        if let Some(blob) = self.db.get(format!("b:{}", hash).as_bytes())? {
            let (block, _): (Block, usize) = bincode::decode_from_slice(&blob, *BINCODE_CONFIG)?;
            return Ok(Some(block));
        }
        Ok(None)
    }

    /// Find common ancestor between two blocks
    fn find_common_ancestor(&self, hash_a: &str, hash_b: &str) -> Result<Option<String>> {
        let mut blocks_a = Vec::new();
        let mut current = hash_a.to_string();

        // Collect all blocks from hash_a to genesis
        while let Some(block) = self.load_block(&current)? {
            blocks_a.push(current.clone());
            if block.header.index == 0 {
                break;
            }
            current = block.header.previous_hash.clone();
        }

        // Walk from hash_b to genesis and find first common block
        let mut current = hash_b.to_string();
        while let Some(block) = self.load_block(&current)? {
            if blocks_a.contains(&current) {
                return Ok(Some(current));
            }
            if block.header.index == 0 {
                break;
            }
            current = block.header.previous_hash.clone();
        }

        Ok(None)
    }

    /// Reorganize chain to new tip if it has more work
    /// Returns true if reorg happened, false if current chain is already best
    pub fn reorganize_if_needed(&mut self, new_block_hash: &str) -> Result<bool> {
        let mut current_tip = match &self.chain_tip {
            Some(tip) => tip.clone(),
            None => {
                // No current chain, accept any valid block
                return Ok(false);
            }
        };

        let mut current_header = self
            .load_header(&current_tip)?
            .ok_or_else(|| anyhow!("Cannot load current tip header"))?;

        // Safety: If tip height is suspiciously low compared to DB block count,
        // recover tip first to avoid accidental reorg to a shorter fork.
        {
            let block_count = self.count_blocks() as u64;
            if block_count > 100 && current_header.index + 1 < block_count / 2 {
                log::warn!(
                    "⚠️  Stale tip detected before reorg (tip height: {}, block_count: {}), recovering tip...",
                    current_header.index,
                    block_count
                );

                self.recover_tip()?;

                current_tip = self
                    .chain_tip
                    .clone()
                    .ok_or_else(|| anyhow!("Tip recovery failed: chain tip is missing"))?;
                current_header = self
                    .load_header(&current_tip)?
                    .ok_or_else(|| anyhow!("Tip recovery failed: recovered tip header missing"))?;

                log::info!(
                    "✅ Tip recovered before reorg check: now at height {}",
                    current_header.index
                );
            }
        }

        // Calculate chain work for both tips
        let current_work = self.calculate_chain_work(&current_tip)?;
        let new_work = self.calculate_chain_work(new_block_hash)?;

        log::debug!(
            "Chain work comparison: current={} (hash={}), new={} (hash={})",
            current_work,
            &current_tip[..16],
            new_work,
            &new_block_hash[..16]
        );

        // Keep current chain if it has equal or more work
        if current_work >= new_work {
            log::debug!("Current chain has more work, keeping it");
            return Ok(false);
        }

        log::warn!(
            "🔄 REORGANIZATION NEEDED: new chain has more work ({} vs {})",
            new_work,
            current_work
        );

        // Find common ancestor
        let ancestor = self.find_common_ancestor(&current_tip, new_block_hash)?;
        if ancestor.is_none() {
            return Err(anyhow!("No common ancestor found for reorganization"));
        }

        let ancestor = ancestor.unwrap();
        log::info!("Common ancestor: {}", &ancestor[..16]);

        // 🔒 Security: Check reorganization depth to prevent 51% attacks
        let ancestor_header = self
            .load_header(&ancestor)?
            .ok_or_else(|| anyhow!("Cannot load ancestor header"))?;

        let current_height = current_header.index;
        let fork_point_height = ancestor_header.index;
        let reorg_depth = current_height - fork_point_height;

        // 🔒 Security: Validate reorganization depth doesn't exceed consensus limit
        crate::security::validate_reorg_depth(
            current_height,
            fork_point_height,
            self.max_reorg_depth,
        )?;

        // 🔒 Policy: Check if reorg conflicts with checkpoint policy
        let (checkpoint_allowed, checkpoint_reason) =
            crate::checkpoint::check_reorg_against_checkpoints(reorg_depth, current_height);

        if !checkpoint_allowed {
            log::error!(
                "🚨 Reorganization REJECTED by checkpoint policy: {}",
                checkpoint_reason.unwrap_or_else(|| "Unknown reason".to_string())
            );
            return Err(anyhow!(
                "Reorganization violates checkpoint policy (depth: {}, current height: {})",
                reorg_depth,
                current_height
            ));
        }

        log::info!(
            "✅ Reorganization passes checkpoint policy check (depth: {}, height: {})",
            reorg_depth,
            current_height
        );

        // Collect blocks to rollback (from current tip to ancestor)
        let mut rollback_blocks = Vec::new();
        let mut current = current_tip.clone();
        while current != ancestor {
            let block = self
                .load_block(&current)?
                .ok_or_else(|| anyhow!("Block not found during reorg: {}", current))?;
            rollback_blocks.push(block.clone());
            current = block.header.previous_hash.clone();
        }

        // Collect blocks to apply (from ancestor to new tip)
        let mut apply_blocks = Vec::new();
        let mut current = new_block_hash.to_string();
        while current != ancestor {
            let block = self
                .load_block(&current)?
                .ok_or_else(|| anyhow!("Block not found during reorg: {}", current))?;
            apply_blocks.push(block.clone());
            current = block.header.previous_hash.clone();
        }
        apply_blocks.reverse(); // Apply from ancestor to new tip

        log::warn!(
            "Reorganizing: rolling back {} blocks, applying {} blocks",
            rollback_blocks.len(),
            apply_blocks.len()
        );

        // Rollback: reverse UTXO changes
        self.rollback_blocks(&rollback_blocks)?;

        // Apply: replay new chain
        self.replay_blocks(&apply_blocks)?;

        // Update chain tip
        let mut batch = WriteBatch::default();
        batch.put(b"tip", new_block_hash.as_bytes());
        put_batch(&self.db, batch)?;
        self.chain_tip = Some(new_block_hash.to_string());
        // Sync bc.difficulty after reorg
        if let Some(new_tip_block) = apply_blocks.last() {
            self.difficulty = new_tip_block.header.difficulty;
        }

        log::warn!(
            "✅ Reorganization complete: new tip = {}",
            &new_block_hash[..16]
        );

        Ok(true)
    }

    /// Rollback UTXO changes from a list of blocks (reverse order)
    /// Also deletes the rolled-back blocks from DB
    fn rollback_blocks(&mut self, blocks: &[Block]) -> Result<()> {
        let mut batch = WriteBatch::default();

        for block in blocks {
            log::info!("Rolling back block {} (hash: {})", block.header.index, &block.hash[..16]);

            // Process transactions in reverse order
            for tx in block.transactions.iter().rev() {
                // Delete UTXOs created by this transaction
                for i in 0..tx.outputs.len() {
                    let ukey = format!("u:{}:{}", tx.txid, i);
                    batch.delete(ukey.as_bytes());
                }

                // Restore UTXOs spent by this transaction (skip coinbase)
                if !tx.inputs.is_empty() {
                    for input in &tx.inputs {
                        // Restore the UTXO that was spent
                        let spent_tx = self
                            .load_tx(&input.txid)?
                            .ok_or_else(|| anyhow!("Cannot find spent tx: {}", input.txid))?;

                        if let Some(output) = spent_tx.outputs.get(input.vout as usize) {
                            let utxo = Utxo::new(
                                input.txid.clone(),
                                input.vout,
                                output.to.clone(),
                                output.amount(),
                            );
                            let ublob = bincode::encode_to_vec(&utxo, *BINCODE_CONFIG)?;
                            batch.put(
                                format!("u:{}:{}", input.txid, input.vout).as_bytes(),
                                &ublob,
                            );
                        }
                    }
                }
                
                // Delete transaction from DB
                batch.delete(format!("t:{}", tx.txid).as_bytes());
            }
            
            // Delete block from DB
            batch.delete(format!("b:{}", block.hash).as_bytes());
            
            // Delete block index (will be overwritten by new chain anyway, but clean up)
            batch.delete(format!("i:{}", block.header.index).as_bytes());
            
            log::info!("✅ Block {} deleted from DB during rollback", block.header.index);
        }

        put_batch(&self.db, batch)?;
        Ok(())
    }

    /// Replay blocks to apply UTXO changes (forward order)
    fn replay_blocks(&mut self, blocks: &[Block]) -> Result<()> {
        for block in blocks {
            log::info!("Replaying block {} (hash: {})", block.header.index, &block.hash[..16]);

            // Update UTXO set and block index
            let mut batch = WriteBatch::default();

            for tx in &block.transactions {
                // Create new UTXOs
                for (i, output) in tx.outputs.iter().enumerate() {
                    let utxo = Utxo::new(
                        tx.txid.clone(),
                        i as u32,
                        output.to.clone(),
                        output.amount(),
                    );
                    let ublob = bincode::encode_to_vec(&utxo, *BINCODE_CONFIG)?;
                    batch.put(format!("u:{}:{}", tx.txid, i).as_bytes(), &ublob);
                }

                // Spend UTXOs (skip coinbase)
                if !tx.inputs.is_empty() {
                    for input in &tx.inputs {
                        batch.delete(format!("u:{}:{}", input.txid, input.vout).as_bytes());
                    }
                }
            }
            
            // Update block index for new chain
            batch.put(format!("i:{}", block.header.index).as_bytes(), block.hash.as_bytes());

            put_batch(&self.db, batch)?;
        }

        Ok(())
    }

    pub fn get_block_reward(&self, height: u64) -> U256 {
        let halvings = (height / HALVING_INTERVAL) as u32;

        if halvings >= 33 {
            return U256::zero(); // 공급 상한 도달
        }

        initial_block_reward() >> halvings
    }

    /// Reset blockchain to empty state (for chain reorg from genesis)
    /// WARNING: This deletes all blockchain data!
    pub fn reset_chain(&mut self) -> Result<()> {
        log::warn!("🔄 Resetting blockchain database...");
        
        // Delete all blockchain-related keys
        let mut batch = WriteBatch::default();
        
        // Delete tip
        batch.delete(b"tip");
        
        // Collect all keys to delete
        let mut keys_to_delete = Vec::new();
        let iter = self.db.iterator(rocksdb::IteratorMode::Start);
        for item in iter {
            if let Ok((key, _)) = item {
                let key_str = String::from_utf8_lossy(&key);
                // Delete block, transaction, utxo, and index keys
                if key_str.starts_with("b:") || key_str.starts_with("t:") || 
                   key_str.starts_with("u:") || key_str.starts_with("i:") {
                    keys_to_delete.push(key.to_vec());
                }
            }
        }
        
        for key in keys_to_delete {
            batch.delete(&key);
        }
        
        put_batch(&self.db, batch)?;
        self.chain_tip = None;
        self.difficulty = Self::POW_LIMIT_BITS;
        
        log::info!("✅ Blockchain reset complete");
        Ok(())
    }

    /// Recover chain tip by scanning database for highest valid block
    /// Use this when tip pointer is incorrect but blocks still exist
    pub fn recover_tip(&mut self) -> Result<()> {
        log::warn!("🔧 Attempting to recover chain tip from database...");
        
        let mut highest_block: Option<(u64, String, Block)> = None;
        let mut block_count = 0;
        
        // Scan all block keys
        let iter = self.db.iterator(rocksdb::IteratorMode::Start);
        for item in iter {
            if let Ok((key, value)) = item {
                let key_str = String::from_utf8_lossy(&key);
                
                // Check if this is a block key (b:{hash})
                if key_str.starts_with("b:") {
                    block_count += 1;
                    if let Ok((block, _)) = bincode::decode_from_slice::<Block, _>(&value, *BINCODE_CONFIG) {
                        let block_height = block.header.index;
                        let block_hash = key_str.strip_prefix("b:").unwrap_or("").to_string();
                        
                        // Update if this is the highest block so far
                        if highest_block.is_none() || block_height > highest_block.as_ref().unwrap().0 {
                            highest_block = Some((block_height, block_hash, block));
                        }
                    }
                }
            }
        }
        
        log::info!("📊 Found {} blocks in database", block_count);
        
        if let Some((height, hash, _block)) = highest_block {
            log::info!("✅ Found highest block: #{} (hash: {})", height, hash);
            
            // Update tip pointer
            self.db.put(b"tip", hash.as_bytes())?;
            self.chain_tip = Some(hash.clone());
            
            log::info!("✅ Chain tip recovered successfully to block #{}", height);
            Ok(())
        } else {
            log::error!("❌ No blocks found in database");
            Err(anyhow!("No blocks found in database"))
        }
    }

    /// Count blocks in database (diagnostic utility)
    pub fn count_blocks(&self) -> usize {
        let mut count = 0;
        let iter = self.db.iterator(rocksdb::IteratorMode::Start);
        for item in iter {
            if let Ok((key, _)) = item {
                let key_str = String::from_utf8_lossy(&key);
                if key_str.starts_with("b:") {
                    count += 1;
                }
            }
        }
        count
    }
}
