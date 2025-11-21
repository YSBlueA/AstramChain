use crate::block::{Block, BlockHeader, compute_header_hash, compute_merkle_root};
use crate::db::{open_db, put_batch};
use crate::transaction::{Transaction, TransactionInput, TransactionOutput};
use crate::utxo::Utxo;
use anyhow::{Result, anyhow};
use bincode::config;
use chrono::Utc;
use once_cell::sync::Lazy;
use rocksdb::{DB, WriteBatch};

pub static BINCODE_CONFIG: Lazy<config::Configuration> = Lazy::new(|| config::standard());

/// Blockchain structure (disk-based RocksDB + in-memory cache)
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

    /// Create genesis block (with a single coinbase transaction)
    pub fn create_genesis(&mut self, address: &str) -> Result<String> {
        if self.chain_tip.is_some() {
            return Err(anyhow!("chain already exists"));
        }
        let cb = Transaction::coinbase(address, 50);

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
        // header
        let header_blob = bincode::encode_to_vec(&block.header, *BINCODE_CONFIG)?;
        batch.put(format!("h:{}", hash).as_bytes(), &header_blob);
        // tx
        let tx_blob = bincode::encode_to_vec(&cb, *BINCODE_CONFIG)?;
        batch.put(format!("t:{}", cb.txid).as_bytes(), &tx_blob);

        for (i, out) in cb.outputs.iter().enumerate() {
            let utxo = Utxo {
                txid: cb.txid.clone(),
                vout: i as u32,
                to: out.to.clone(),
                amount: out.amount,
            };

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
            let prev_hash = &block.header.previous_hash;
            let key = format!("h:{}", prev_hash);
            if self.db.get(key.as_bytes())?.is_none() {
                return Err(anyhow!("previous header not found: {}", prev_hash));
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
            let ok = tx.verify_signatures()?;
            if !ok {
                return Err(anyhow!("tx signature invalid: {}", tx.txid));
            }

            // coinbase skip UTXO referencing checks
            if i == 0 {
                // persist tx and utxos
                let tx_blob = bincode::encode_to_vec(tx, *BINCODE_CONFIG)?;
                batch.put(format!("t:{}", tx.txid).as_bytes(), &tx_blob);
                for (v, out) in tx.outputs.iter().enumerate() {
                    let utxo = Utxo {
                        txid: tx.txid.clone(),
                        vout: v as u32,
                        to: out.to.clone(),
                        amount: out.amount,
                    };
                    let ublob = bincode::encode_to_vec(&utxo, *BINCODE_CONFIG)?;
                    batch.put(format!("u:{}:{}", tx.txid, v).as_bytes(), &ublob);
                }
                continue;
            }

            // for non-coinbase tx, check each input exists in UTXO and sum amounts
            let mut input_sum: u128 = 0;
            for inp in &tx.inputs {
                let ukey = format!("u:{}:{}", inp.txid, inp.vout);
                match self.db.get(ukey.as_bytes())? {
                    Some(blob) => {
                        let (u, _): (Utxo, usize) =
                            bincode::decode_from_slice(&blob, *BINCODE_CONFIG)?;
                        input_sum += u.amount as u128;
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
            let mut output_sum: u128 = 0;
            for out in &tx.outputs {
                output_sum += out.amount as u128;
            }
            if output_sum > input_sum {
                return Err(anyhow!("outputs exceed inputs in tx {}", tx.txid));
            }

            // persist tx and create new utxos
            let tx_blob = bincode::encode_to_vec(tx, *BINCODE_CONFIG)?;
            batch.put(format!("t:{}", tx.txid).as_bytes(), &tx_blob);
            for (v, out) in tx.outputs.iter().enumerate() {
                let utxo = Utxo {
                    txid: tx.txid.clone(),
                    vout: v as u32,
                    to: out.to.clone(),
                    amount: out.amount,
                };
                let ublob = bincode::encode_to_vec(&utxo, *BINCODE_CONFIG)?;
                batch.put(format!("u:{}:{}", tx.txid, v).as_bytes(), &ublob);
            }
        }

        // persist header, index, tip
        let header_blob = bincode::encode_to_vec(&block.header, *BINCODE_CONFIG)?;
        batch.put(format!("h:{}", block.hash).as_bytes(), &header_blob);
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
        if let Some(blob) = self.db.get(format!("h:{}", hash).as_bytes())? {
            let (h, _): (BlockHeader, usize) = bincode::decode_from_slice(&blob, *BINCODE_CONFIG)?;
            return Ok(Some(h));
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

    /// get balance by scanning UTXO set (inefficient but correct)
    pub fn get_balance(&self, address: &str) -> Result<u128, Box<dyn std::error::Error>> {
        let mut iter = self.db.iterator(rocksdb::IteratorMode::Start);
        let mut sum: u128 = 0;

        while let Some(item) = iter.next() {
            let (k, v) = item?;

            let key = String::from_utf8_lossy(&k).to_string();
            if key.starts_with("u:") {
                let (utxo, _): (Utxo, usize) = bincode::decode_from_slice(&v, *BINCODE_CONFIG)
                    .map_err(|e| format!("deserialize failed: {}", e))?;

                if utxo.to == address {
                    sum += utxo.amount as u128;
                }
            }
        }

        Ok(sum)
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
            let key_str = String::from_utf8(key.to_vec())?;

            // UTXO key: u:{txid}:{vout}
            if key_str.starts_with("u:") {
                let (_u, _): (Utxo, usize) = bincode::decode_from_slice(&value, *BINCODE_CONFIG)?;
                if _u.to == address {
                    utxos.push(_u);
                }
            }
        }

        Ok(utxos)
    }
}
