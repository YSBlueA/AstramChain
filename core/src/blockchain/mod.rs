use crate::block::{Block, BlockHeader, compute_header_hash, compute_merkle_root};
use crate::db::{open_db, put_batch};
use crate::transaction::Transaction;
use crate::utxo::Utxo;
use anyhow::{Result, anyhow};
use bincode::config;
use chrono::Utc;
use log;
use once_cell::sync::Lazy;
use primitive_types::U256;
use rocksdb::{DB, WriteBatch};

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
    pub block_interval: i64, // Target block generation interval (seconds)
}

impl Blockchain {
    pub fn new(db_path: &str) -> Result<Self> {
        let db = open_db(db_path)?;
        // load tip if exists
        let tip = db.get(b"tip")?;
        let chain_tip = tip.map(|v| String::from_utf8(v).unwrap());

        // Load current difficulty from chain tip
        let difficulty = if let Some(ref tip_hash) = chain_tip {
            // Try to load the tip block header
            if let Ok(Some(blob)) = db.get(format!("b:{}", tip_hash).as_bytes()) {
                if let Ok((block, _)) =
                    bincode::decode_from_slice::<Block, _>(&blob, *BINCODE_CONFIG)
                {
                    // Calculate difficulty for the next block based on current chain state
                    let next_index = block.header.index + 1;
                    // Create temporary instance WITHOUT opening DB again
                    // We'll calculate difficulty manually here to avoid double-opening DB

                    // Simple difficulty adjustment logic inline
                    if next_index % 30 == 0 && next_index > 0 {
                        // Adjustment point - get last 30 blocks to calculate
                        let start_index = next_index.saturating_sub(30);
                        if let Ok(Some(start_blob)) =
                            db.get(format!("i:{}", start_index).as_bytes())
                        {
                            if let Ok(start_hash) = String::from_utf8(start_blob) {
                                if let Ok(Some(start_header_blob)) =
                                    db.get(format!("b:{}", start_hash).as_bytes())
                                {
                                    if let Ok((start_block, _)) =
                                        bincode::decode_from_slice::<Block, _>(
                                            &start_header_blob,
                                            *BINCODE_CONFIG,
                                        )
                                    {
                                        let time_taken =
                                            block.header.timestamp - start_block.header.timestamp;
                                        let expected_time = 120 * 30; // block_interval * 30

                                        if time_taken < expected_time / 2 {
                                            block.header.difficulty + 1
                                        } else if time_taken > expected_time * 2 {
                                            block.header.difficulty.saturating_sub(1).max(1)
                                        } else {
                                            block.header.difficulty
                                        }
                                    } else {
                                        block.header.difficulty
                                    }
                                } else {
                                    block.header.difficulty
                                }
                            } else {
                                block.header.difficulty
                            }
                        } else {
                            block.header.difficulty
                        }
                    } else {
                        block.header.difficulty
                    }
                } else {
                    log::warn!("Failed to decode tip block, using default difficulty");
                    2
                }
            } else {
                log::warn!("Tip block not found, using default difficulty");
                2
            }
        } else {
            // No chain exists yet, use default
            2
        };

        log::info!("Blockchain initialized with difficulty: {}", difficulty);

        Ok(Blockchain {
            db,
            chain_tip,
            difficulty,
            block_interval: 120, // Target: 2 minutes per block
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

    /// validate and insert block (core of migration/consensus)
    pub fn validate_and_insert_block(&mut self, block: &Block) -> Result<()> {
        // 1) header hash match
        let computed = compute_header_hash(&block.header)?;
        if computed != block.hash {
            return Err(anyhow!(
                "header hash mismatch: computed {} != block.hash {}",
                computed,
                block.hash
            ));
        }

        // 2) Proof-of-Work: verify hash meets difficulty requirement
        let required_prefix = "0".repeat(block.header.difficulty as usize);
        if !block.hash.starts_with(&required_prefix) {
            return Err(anyhow!(
                "invalid PoW: hash {} does not meet difficulty {} (needs {} leading zeros)",
                block.hash,
                block.header.difficulty,
                block.header.difficulty
            ));
        }

        // 3) Difficulty adjustment: verify block uses correct difficulty
        let expected_difficulty = self.calculate_adjusted_difficulty(block.header.index)?;
        if block.header.difficulty != expected_difficulty {
            return Err(anyhow!(
                "incorrect difficulty at block {}: got {}, expected {} (hash rate adjustment)",
                block.header.index,
                block.header.difficulty,
                expected_difficulty
            ));
        }

        // 4) merkle check
        let txids: Vec<String> = block.transactions.iter().map(|t| t.txid.clone()).collect();
        let merkle = compute_merkle_root(&txids);
        if merkle != block.header.merkle_root {
            return Err(anyhow!("merkle mismatch"));
        }

        // 5) previous exists (unless genesis)
        if block.header.index > 0 {
            let prev_key = format!("b:{}", block.header.previous_hash);
            if self.db.get(prev_key.as_bytes())?.is_none() {
                return Err(anyhow!(
                    "previous header not found: {}",
                    block.header.previous_hash
                ));
            }
        }

        // 6) transactions validation: signatures + UTXO references
        // We'll create a WriteBatch and atomically apply changes
        let mut batch = WriteBatch::default();

        // 🔒 Security: Validate block-level constraints
        crate::security::validate_block_security(&block)?;

        // For coinbase check
        if block.transactions.is_empty() {
            return Err(anyhow!("empty block"));
        }

        // coinbase must be first tx and inputs empty
        let coinbase = &block.transactions[0];
        if !coinbase.inputs.is_empty() {
            return Err(anyhow!("coinbase must have no inputs"));
        }

        // iterate non-coinbase txs
        for (i, tx) in block.transactions.iter().enumerate() {
            // 🔒 Security: Validate transaction-level constraints
            crate::security::validate_transaction_security(tx, block.header.timestamp)?;

            // verify signature(s)
            if !tx.verify_signatures()? {
                return Err(anyhow!("tx signature invalid: {}", tx.txid));
            }

            // coinbase skip UTXO referencing checks
            if i == 0 {
                // persist tx and utxos
                let tx_blob = bincode::encode_to_vec(tx, *BINCODE_CONFIG)?;
                batch.put(format!("t:{}", tx.txid).as_bytes(), &tx_blob);
                for (v, out) in tx.outputs.iter().enumerate() {
                    // Normalize address to lowercase for consistent storage
                    let normalized_address = out.to.to_lowercase();
                    let utxo =
                        Utxo::new(tx.txid.clone(), v as u32, normalized_address, out.amount());
                    let ublob = bincode::encode_to_vec(&utxo, *BINCODE_CONFIG)?;
                    batch.put(format!("u:{}:{}", tx.txid, v).as_bytes(), &ublob);
                }
                continue;
            }

            // for non-coinbase tx, check each input exists in UTXO and sum amounts
            let mut input_sum = U256::zero();
            let mut used_utxos = std::collections::HashSet::new();

            for inp in &tx.inputs {
                let ukey = format!("u:{}:{}", inp.txid, inp.vout);

                // 🔒 Security: Prevent double-spending within same transaction
                if !used_utxos.insert(ukey.clone()) {
                    return Err(anyhow!(
                        "duplicate input in tx {}: {}:{}",
                        tx.txid,
                        inp.txid,
                        inp.vout
                    ));
                }

                match self.db.get(ukey.as_bytes())? {
                    Some(blob) => {
                        let (u, _): (Utxo, usize) =
                            bincode::decode_from_slice(&blob, *BINCODE_CONFIG)?;

                        // 🔒 Security: CRITICAL - Verify UTXO ownership
                        // Derive address from input's public key and compare with UTXO owner
                        let input_address = crate::crypto::eth_address_from_pubkey_hex(&inp.pubkey)
                            .map_err(|e| anyhow!("invalid pubkey in input: {}", e))?;

                        let utxo_owner = u.to.to_lowercase();
                        let input_addr_lower = input_address.to_lowercase();

                        if input_addr_lower != utxo_owner {
                            return Err(anyhow!(
                                "UTXO ownership verification failed for {}:{} - expected {}, got {}",
                                inp.txid,
                                inp.vout,
                                utxo_owner,
                                input_addr_lower
                            ));
                        }

                        input_sum = input_sum + u.amount();
                        // mark as spent by deleting in batch
                        batch.delete(ukey.as_bytes());
                    }
                    None => {
                        return Err(anyhow!(
                            "referenced utxo not found {}:{} (already spent or never existed)",
                            inp.txid,
                            inp.vout
                        ));
                    }
                }
            }

            let mut output_sum = U256::zero();
            for out in &tx.outputs {
                output_sum = output_sum + out.amount();
            }

            // 🔒 Security: Validate fee is reasonable (outputs <= inputs)
            if output_sum > input_sum {
                return Err(anyhow!(
                    "invalid transaction {}: outputs ({}) exceed inputs ({})",
                    tx.txid,
                    output_sum,
                    input_sum
                ));
            }

            // 🔒 Security: Enforce minimum fee based on transaction size (prevent DDoS)
            // Uses Anti-DDoS fee policy from config.rs: BASE_MIN_FEE + (size × rate)
            let fee = input_sum - output_sum;
            let tx_blob = bincode::encode_to_vec(tx, *BINCODE_CONFIG)?;
            let min_fee = crate::config::calculate_min_fee(tx_blob.len());

            if fee < min_fee {
                return Err(anyhow!(
                    "transaction fee too low {}: got {} natoshi, need {} natoshi (base 100 Twei + {} bytes × 200 Gwei/byte)",
                    tx.txid,
                    fee,
                    min_fee,
                    tx_blob.len()
                ));
            }

            // persist tx and create new utxos
            let tx_blob = bincode::encode_to_vec(tx, *BINCODE_CONFIG)?;
            batch.put(format!("t:{}", tx.txid).as_bytes(), &tx_blob);
            for (v, out) in tx.outputs.iter().enumerate() {
                // Normalize address to lowercase for consistent storage
                let normalized_address = out.to.to_lowercase();
                let utxo = Utxo::new(tx.txid.clone(), v as u32, normalized_address, out.amount());
                let ublob = bincode::encode_to_vec(&utxo, *BINCODE_CONFIG)?;
                batch.put(format!("u:{}:{}", tx.txid, v).as_bytes(), &ublob);
            }
        }

        // persist complete block, index, tip
        let block_blob = bincode::encode_to_vec(&block, *BINCODE_CONFIG)?;
        batch.put(format!("b:{}", block.hash).as_bytes(), &block_blob);
        batch.put(
            format!("i:{}", block.header.index).as_bytes(),
            block.hash.as_bytes(),
        );
        batch.put(b"tip", block.hash.as_bytes());

        // commit
        put_batch(&self.db, batch)?;
        self.chain_tip = Some(block.hash.clone());

        // Adjust difficulty every 30 blocks
        let next_index = block.header.index + 1;
        if let Ok(new_difficulty) = self.calculate_adjusted_difficulty(next_index) {
            if new_difficulty != self.difficulty {
                log::info!(
                    "Difficulty updated for next block ({}): {} -> {}",
                    next_index,
                    self.difficulty,
                    new_difficulty
                );
                // Update in-memory difficulty for next mining round
                self.difficulty = new_difficulty;
            }
        }

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

    /// Calculate adjusted difficulty based on recent block times
    /// Adjustment period: every 30 blocks
    /// Target: 120 seconds per block (2 minutes)
    /// Bitcoin-style: Conservative adjustment with max ±1 change per period
    pub fn calculate_adjusted_difficulty(&self, current_index: u64) -> Result<u32> {
        const ADJUSTMENT_INTERVAL: u64 = 30; // Adjust every 30 blocks
        const MIN_DIFFICULTY: u32 = 1;
        const MAX_DIFFICULTY: u32 = 10; // Reduced from 32

        // No adjustment needed for early blocks
        if current_index < ADJUSTMENT_INTERVAL {
            return Ok(self.difficulty);
        }

        // Only adjust at interval boundaries
        if current_index % ADJUSTMENT_INTERVAL != 0 {
            return Ok(self.difficulty);
        }

        // Get the block at adjustment boundary (30 blocks ago)
        let start_index = current_index - ADJUSTMENT_INTERVAL;
        let start_hash = self.db.get(format!("i:{}", start_index).as_bytes())?;
        let end_hash = self.db.get(format!("i:{}", current_index - 1).as_bytes())?;

        if start_hash.is_none() || end_hash.is_none() {
            log::warn!("Cannot find blocks for difficulty adjustment");
            return Ok(self.difficulty);
        }

        let start_hash_str = String::from_utf8(start_hash.unwrap())?;
        let end_hash_str = String::from_utf8(end_hash.unwrap())?;

        let start_header = self.load_header(&start_hash_str)?;
        let end_header = self.load_header(&end_hash_str)?;

        if start_header.is_none() || end_header.is_none() {
            log::warn!("Cannot load headers for difficulty adjustment");
            return Ok(self.difficulty);
        }

        let start_time = start_header.unwrap().timestamp;
        let end_time = end_header.unwrap().timestamp;

        // Calculate actual time taken for the last 30 blocks
        let actual_time = end_time - start_time;
        let target_time = self.block_interval * ADJUSTMENT_INTERVAL as i64; // 120s × 30 = 3600s (1 hour)

        log::info!(
            "Difficulty adjustment at block {}: actual={}s, target={}s, avg={:.1}s/block",
            current_index,
            actual_time,
            target_time,
            actual_time as f64 / ADJUSTMENT_INTERVAL as f64
        );

        // Bitcoin-style conservative adjustment
        // Problem: difficulty is "leading zeros count", so each +1 = 16x harder!
        // Solution: Only adjust by ±1, with damping based on ratio
        let current_difficulty = self.difficulty;

        // Calculate how far off we are from target
        let ratio = actual_time as f64 / target_time as f64;

        let new_difficulty = if ratio < 0.5 {
            // Blocks are >2x too fast - increase difficulty by 1
            current_difficulty.saturating_add(1)
        } else if ratio < 0.75 {
            // Blocks are 25-50% too fast - increase difficulty by 1
            current_difficulty.saturating_add(1)
        } else if ratio > 2.0 {
            // Blocks are >2x too slow - decrease difficulty by 1
            current_difficulty.saturating_sub(1).max(1)
        } else if ratio > 1.5 {
            // Blocks are 50-100% too slow - decrease difficulty by 1
            current_difficulty.saturating_sub(1).max(1)
        } else {
            // Within ±50% of target - no change
            current_difficulty
        };

        let final_difficulty = new_difficulty.clamp(MIN_DIFFICULTY, MAX_DIFFICULTY);

        if final_difficulty != current_difficulty {
            log::info!(
                "Difficulty adjusted: {} -> {} (ratio: {:.2}x target, avg: {:.1}s/block vs target: {}s/block)",
                current_difficulty,
                final_difficulty,
                ratio,
                actual_time as f64 / ADJUSTMENT_INTERVAL as f64,
                self.block_interval
            );
        } else {
            log::info!(
                "Difficulty unchanged: {} (ratio: {:.2}x, within acceptable range)",
                current_difficulty,
                ratio
            );
        }

        Ok(final_difficulty)
    }

    /// Find a valid nonce by updating header.nonce and computing header hash.
    /// Returns (nonce, hash).
    pub fn find_valid_nonce(
        &self,
        header: &mut BlockHeader,
        difficulty: u32,
    ) -> Result<(u64, String)> {
        let target_prefix = "0".repeat(difficulty as usize);
        let mut nonce: u64 = header.nonce;

        loop {
            header.nonce = nonce;
            let hash = compute_header_hash(header)?;
            if hash.starts_with(&target_prefix) {
                return Ok((nonce, hash));
            }

            nonce = nonce.wrapping_add(1);
            // Periodic yield can be added by caller if needed (to avoid busy-wait in single-threaded contexts)
            // For large scale mining, this loop would be replaced with GPU/parallel miners.
        }
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

    /// Get transaction by eth_hash (EVM-compatible hash)
    pub fn get_transaction_by_eth_hash(
        &self,
        eth_hash: &str,
    ) -> anyhow::Result<Option<(Transaction, usize)>> {
        let blocks = self.get_all_blocks()?;

        // Normalize eth_hash (add 0x if missing)
        let normalized_hash = if eth_hash.starts_with("0x") {
            eth_hash.to_string()
        } else {
            format!("0x{}", eth_hash)
        };

        for block in blocks {
            for tx in block.transactions {
                if tx.eth_hash == normalized_hash {
                    return Ok(Some((tx, block.header.index as usize)));
                }
            }
        }

        Ok(None)
    }

    /// Calculate total transaction volume from all outputs in DB (in natoshi)
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
        let mut utxo_count = 0;
        let iter = self.db.iterator(rocksdb::IteratorMode::Start);

        for item in iter {
            let (key, value) = item?;
            let key_str = String::from_utf8_lossy(&key);

            // UTXO key: u:{txid}:{vout}
            if key_str.starts_with("u:") {
                match bincode::decode_from_slice::<Utxo, _>(&value, *BINCODE_CONFIG) {
                    Ok((utxo, _)) => {
                        if utxo.to == address {
                            utxo_count += 1;
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
        let mut total_work = 0u64;
        let mut current_hash = block_hash.to_string();

        loop {
            let block = self.load_block(&current_hash)?;
            if block.is_none() {
                break;
            }

            let block = block.unwrap();
            // Each difficulty level represents 16x more work (hexadecimal)
            // Work = 16^difficulty
            let block_work = 16u64.saturating_pow(block.header.difficulty);
            total_work = total_work.saturating_add(block_work);

            if block.header.index == 0 {
                break; // Reached genesis
            }

            current_hash = block.header.previous_hash.clone();
        }

        Ok(total_work)
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
        let current_tip = match &self.chain_tip {
            Some(tip) => tip.clone(),
            None => {
                // No current chain, accept any valid block
                return Ok(false);
            }
        };

        // Calculate chain work for both tips
        let current_work = self.calculate_chain_work(&current_tip)?;
        let new_work = self.calculate_chain_work(new_block_hash)?;

        log::info!(
            "Chain work comparison: current={} (hash={}), new={} (hash={})",
            current_work,
            &current_tip[..16],
            new_work,
            &new_block_hash[..16]
        );

        // Keep current chain if it has equal or more work
        if current_work >= new_work {
            log::info!("Current chain has more work, keeping it");
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

        log::warn!(
            "✅ Reorganization complete: new tip = {}",
            &new_block_hash[..16]
        );

        Ok(true)
    }

    /// Rollback UTXO changes from a list of blocks (reverse order)
    fn rollback_blocks(&mut self, blocks: &[Block]) -> Result<()> {
        let mut batch = WriteBatch::default();

        for block in blocks {
            log::info!("Rolling back block {}", block.header.index);

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
            }
        }

        put_batch(&self.db, batch)?;
        Ok(())
    }

    /// Replay blocks to apply UTXO changes (forward order)
    fn replay_blocks(&mut self, blocks: &[Block]) -> Result<()> {
        for block in blocks {
            log::info!("Replaying block {}", block.header.index);

            // We already have the block stored, just need to update UTXO set
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

            put_batch(&self.db, batch)?;
        }

        Ok(())
    }
}
