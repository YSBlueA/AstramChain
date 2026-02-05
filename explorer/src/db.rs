use crate::state::{AddressInfo, BlockInfo, TransactionInfo};
use anyhow::Result;
use chrono::Utc;
use log::{error, info};
use primitive_types::U256;
use rocksdb::{DB, Options, WriteBatch};
use std::collections::HashMap;
use std::sync::Arc;

/// Explorer Database - 블록체인 데이터를 인덱싱하여 저장
pub struct ExplorerDB {
    db: Arc<DB>,
}

impl ExplorerDB {
    /// 새 데이터베이스 열기 또는 생성
    pub fn new(path: &str) -> Result<Self> {
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);

        let db = DB::open(&opts, path)?;

        info!("✅ Explorer database opened at {}", path);

        Ok(ExplorerDB { db: Arc::new(db) })
    }

    /// 블록 저장
    /// Key: b:<height> -> BlockInfo (JSON)
    /// Key: bh:<hash> -> height
    pub fn save_block(&self, block: &BlockInfo) -> Result<()> {
        let mut batch = WriteBatch::default();

        // b:<height> -> BlockInfo
        let block_key = format!("b:{}", block.height);
        let block_json = serde_json::to_string(block)?;
        batch.put(block_key.as_bytes(), block_json.as_bytes());

        // bh:<hash> -> height (해시로 블록 찾기)
        let hash_key = format!("bh:{}", block.hash);
        batch.put(hash_key.as_bytes(), block.height.to_string().as_bytes());

        self.db.write(batch)?;
        Ok(())
    }

    /// 블록 조회 (높이로)
    pub fn get_block_by_height(&self, height: u64) -> Result<Option<BlockInfo>> {
        let key = format!("b:{}", height);
        match self.db.get(key.as_bytes())? {
            Some(data) => {
                let mut block: BlockInfo = serde_json::from_slice(&data)?;
                // Calculate confirmations based on current chain height
                let current_height = self.get_block_count()?;
                block.confirmations = if current_height > height {
                    current_height - height
                } else {
                    0
                };
                Ok(Some(block))
            }
            None => Ok(None),
        }
    }

    /// 블록 조회 (해시로)
    pub fn get_block_by_hash(&self, hash: &str) -> Result<Option<BlockInfo>> {
        let hash_key = format!("bh:{}", hash);
        match self.db.get(hash_key.as_bytes())? {
            Some(height_bytes) => {
                let height_str = String::from_utf8(height_bytes.to_vec())?;
                let height: u64 = height_str.parse()?;
                self.get_block_by_height(height)
            }
            None => Ok(None),
        }
    }

    /// 모든 블록 조회 (페이징)
    pub fn get_blocks(&self, page: u32, limit: u32) -> Result<Vec<BlockInfo>> {
        let total_blocks = self.get_block_count()?;

        if total_blocks == 0 {
            return Ok(Vec::new());
        }

        let mut blocks = Vec::new();

        // 페이지네이션: 최신 블록부터 역순으로
        // page 1 = 최신 블록들, page 2 = 그 다음 오래된 블록들
        let skip = ((page - 1) * limit) as u64;

        if skip >= total_blocks {
            return Ok(Vec::new());
        }

        // 최신 블록 높이부터 시작
        let start_height = total_blocks.saturating_sub(1 + skip);
        let end_height = start_height.saturating_sub(limit as u64 - 1);

        // 최신 블록부터 역순으로 (높은 높이 -> 낮은 높이)
        for height in (end_height..=start_height).rev() {
            if let Some(block) = self.get_block_by_height(height)? {
                blocks.push(block);
            }
        }

        Ok(blocks)
    }

    /// 트랜잭션 저장
    /// Key: t:<hash> -> TransactionInfo
    /// Key: ta:<address>:<timestamp>:<hash> -> "" (주소별 트랜잭션 인덱스)
    /// Key: tb:<height>:<index> -> hash (블록별 트랜잭션 인덱스)
    pub fn save_transaction(&self, tx: &TransactionInfo) -> Result<()> {
        let mut batch = WriteBatch::default();

        // t:<hash> -> TransactionInfo
        let tx_key = format!("t:{}", tx.hash);
        let tx_json = serde_json::to_string(tx)?;
        batch.put(tx_key.as_bytes(), tx_json.as_bytes());

        // ta:<address>:<timestamp>:<hash> -> "" (from 주소)
        let from_key = format!("ta:{}:{}:{}", tx.from, tx.timestamp.timestamp(), tx.hash);
        batch.put(from_key.as_bytes(), b"");

        // ta:<address>:<timestamp>:<hash> -> "" (to 주소)
        let to_key = format!("ta:{}:{}:{}", tx.to, tx.timestamp.timestamp(), tx.hash);
        batch.put(to_key.as_bytes(), b"");

        // tb:<height>:<index> -> hash (블록별 인덱스)
        if let Some(height) = tx.block_height {
            let block_tx_key = format!("tb:{}:{}", height, tx.hash);
            batch.put(block_tx_key.as_bytes(), tx.hash.as_bytes());
        }

        self.db.write(batch)?;
        Ok(())
    }

    /// 트랜잭션 조회
    pub fn get_transaction(&self, hash: &str) -> Result<Option<TransactionInfo>> {
        let key = format!("t:{}", hash);
        match self.db.get(key.as_bytes())? {
            Some(data) => {
                let mut tx: TransactionInfo = serde_json::from_slice(&data)?;
                // Calculate confirmations if transaction is in a block
                if let Some(block_height) = tx.block_height {
                    let current_height = self.get_block_count()?;
                    tx.confirmations = if current_height > block_height {
                        Some(current_height - block_height)
                    } else {
                        Some(0)
                    };
                } else {
                    tx.confirmations = None; // Pending transaction
                }
                Ok(Some(tx))
            }
            None => Ok(None),
        }
    }

    /// 모든 트랜잭션 조회 (페이징)
    pub fn get_transactions(&self, page: u32, limit: u32) -> Result<Vec<TransactionInfo>> {
        let start_key = "t:".as_bytes();
        let mut iter = self.db.raw_iterator();
        iter.seek(start_key);

        let skip = ((page - 1) * limit) as usize;
        let mut transactions = Vec::new();
        let mut count = 0;

        while iter.valid() {
            if let Some(key) = iter.key() {
                if !key.starts_with(b"t:") {
                    break;
                }

                if count >= skip && transactions.len() < limit as usize {
                    if let Some(value) = iter.value() {
                        if let Ok(tx) = serde_json::from_slice::<TransactionInfo>(value) {
                            transactions.push(tx);
                        }
                    }
                }
                count += 1;
            }
            iter.next();
        }

        // 최신순으로 정렬
        transactions.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

        Ok(transactions)
    }

    /// 주소별 트랜잭션 조회
    pub fn get_transactions_by_address(&self, address: &str) -> Result<Vec<TransactionInfo>> {
        let prefix = format!("ta:{}:", address);
        let mut iter = self.db.raw_iterator();
        iter.seek(prefix.as_bytes());

        let mut tx_hashes = Vec::new();

        while iter.valid() {
            if let Some(key) = iter.key() {
                let key_str = String::from_utf8_lossy(key);
                if !key_str.starts_with(&prefix) {
                    break;
                }

                // ta:<address>:<timestamp>:<hash> 형식에서 hash 추출
                let parts: Vec<&str> = key_str.split(':').collect();
                if parts.len() == 4 {
                    tx_hashes.push(parts[3].to_string());
                }
            }
            iter.next();
        }

        // 중복 제거 및 트랜잭션 조회
        let mut transactions = Vec::new();
        let mut seen = std::collections::HashSet::new();

        for hash in tx_hashes {
            if seen.insert(hash.clone()) {
                if let Some(tx) = self.get_transaction(&hash)? {
                    transactions.push(tx);
                }
            }
        }

        // 최신순으로 정렬
        transactions.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

        Ok(transactions)
    }

    /// 주소 정보 저장
    /// Key: addr:<address> -> AddressInfo
    pub fn save_address_info(&self, info: &AddressInfo) -> Result<()> {
        let key = format!("addr:{}", info.address);
        let json = serde_json::to_string(info)?;
        self.db.put(key.as_bytes(), json.as_bytes())?;
        Ok(())
    }

    /// 주소 정보 조회
    pub fn get_address_info(&self, address: &str) -> Result<Option<AddressInfo>> {
        let key = format!("addr:{}", address);
        match self.db.get(key.as_bytes())? {
            Some(data) => {
                let info: AddressInfo = serde_json::from_slice(&data)?;
                Ok(Some(info))
            }
            None => Ok(None),
        }
    }

    /// 주소 정보 계산 및 저장
    pub fn update_address_info(&self, address: &str) -> Result<AddressInfo> {
        let transactions = self.get_transactions_by_address(address)?;

        let mut sent = U256::zero();
        let mut received = U256::zero();
        let mut last_transaction = None;

        for tx in &transactions {
            if tx.from == address {
                sent += tx.amount + tx.fee;
            }
            if tx.to == address {
                received += tx.amount;
            }

            if last_transaction.is_none() || tx.timestamp > last_transaction.unwrap() {
                last_transaction = Some(tx.timestamp);
            }
        }

        let balance = if received > sent {
            received - sent
        } else {
            U256::zero()
        };

        let info = AddressInfo {
            address: address.to_string(),
            balance,
            sent,
            received,
            transaction_count: transactions.len(),
            last_transaction,
        };

        self.save_address_info(&info)?;

        Ok(info)
    }

    /// 블록 개수 조회
    pub fn get_block_count(&self) -> Result<u64> {
        let key = "meta:block_count";
        match self.db.get(key.as_bytes())? {
            Some(data) => {
                let count_str = String::from_utf8(data.to_vec())?;
                Ok(count_str.parse()?)
            }
            None => Ok(0),
        }
    }

    /// 블록 개수 업데이트
    pub fn set_block_count(&self, count: u64) -> Result<()> {
        let key = "meta:block_count";
        self.db.put(key.as_bytes(), count.to_string().as_bytes())?;
        Ok(())
    }

    /// 트랜잭션 개수 조회
    pub fn get_transaction_count(&self) -> Result<u64> {
        let key = "meta:tx_count";
        match self.db.get(key.as_bytes())? {
            Some(data) => {
                let count_str = String::from_utf8(data.to_vec())?;
                Ok(count_str.parse()?)
            }
            None => Ok(0),
        }
    }

    /// 트랜잭션 개수 업데이트
    pub fn set_transaction_count(&self, count: u64) -> Result<()> {
        let key = "meta:tx_count";
        self.db.put(key.as_bytes(), count.to_string().as_bytes())?;
        Ok(())
    }

    /// 마지막 동기화된 블록 높이 조회
    pub fn get_last_synced_height(&self) -> Result<u64> {
        let key = "meta:last_synced";
        match self.db.get(key.as_bytes())? {
            Some(data) => {
                let height_str = String::from_utf8(data.to_vec())?;
                Ok(height_str.parse()?)
            }
            None => Ok(0),
        }
    }

    /// 마지막 동기화된 블록 높이 업데이트
    pub fn set_last_synced_height(&self, height: u64) -> Result<()> {
        let key = "meta:last_synced";
        self.db.put(key.as_bytes(), height.to_string().as_bytes())?;
        Ok(())
    }

    /// 데이터베이스 통계
    pub fn get_stats(&self) -> Result<(u64, u64, U256)> {
        let block_count = self.get_block_count()?;
        let tx_count = self.get_transaction_count()?;

        // 총 거래량 계산
        let mut total_volume = U256::zero();
        let transactions = self.get_transactions(1, 10000)?; // 최대 10000개
        for tx in transactions {
            total_volume += tx.amount;
        }

        Ok((block_count, tx_count, total_volume))
    }

    /// 데이터베이스 초기화 (재동기화용)
    pub fn clear_all(&self) -> Result<()> {
        info!("⚠️ Clearing all explorer data...");

        // 모든 키 삭제
        let mut iter = self.db.raw_iterator();
        iter.seek_to_first();

        let mut batch = WriteBatch::default();
        while iter.valid() {
            if let Some(key) = iter.key() {
                batch.delete(key);
            }
            iter.next();
        }

        self.db.write(batch)?;

        info!("✅ All explorer data cleared");
        Ok(())
    }
}
