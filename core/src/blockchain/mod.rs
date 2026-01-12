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
        Ok(Blockchain {
            db,
            chain_tip,
            difficulty: 2, /*16*/
            block_interval: 60,
        }) // default difficulty (bits like count leading zeros)
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

        // 2) merkle check
        let txids: Vec<String> = block.transactions.iter().map(|t| t.txid.clone()).collect();
        let merkle = compute_merkle_root(&txids);
        if merkle != block.header.merkle_root {
            return Err(anyhow!("merkle mismatch"));
        }

        // 3) previous exists (unless genesis)
        if block.header.index > 0 {
            let prev_key = format!("b:{}", block.header.previous_hash);
            if self.db.get(prev_key.as_bytes())?.is_none() {
                return Err(anyhow!(
                    "previous header not found: {}",
                    block.header.previous_hash
                ));
            }
        }

        // 4) transactions validation: signatures + UTXO references
        // We'll create a WriteBatch and atomically apply changes
        let mut batch = WriteBatch::default();

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
            for inp in &tx.inputs {
                let ukey = format!("u:{}:{}", inp.txid, inp.vout);
                match self.db.get(ukey.as_bytes())? {
                    Some(blob) => {
                        let (u, _): (Utxo, usize) =
                            bincode::decode_from_slice(&blob, *BINCODE_CONFIG)?;
                        input_sum = input_sum + u.amount();
                        // mark as spent by deleting in batch
                        batch.delete(ukey.as_bytes());
                    }
                    None => {
                        return Err(anyhow!(
                            "referenced utxo not found {}:{}",
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
            if output_sum > input_sum {
                return Err(anyhow!("outputs exceed inputs in tx {}", tx.txid));
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
                            log::debug!(
                                "Found UTXO for {}: {} (total so far: {})",
                                address,
                                amount,
                                balance + amount
                            );
                            balance = balance + amount;
                        }
                    }
                    Err(e) => {
                        log::warn!("Failed to decode UTXO at {}: {}", key_str, e);
                    }
                }
            }
        }

        log::info!(
            "Address {} balance: {} (from {} UTXOs)",
            address,
            balance,
            utxo_count
        );
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
}
