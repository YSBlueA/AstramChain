use crate::state::{AddressInfo, BlockInfo, RichListEntry, TransactionInfo};
use anyhow::Result;
use log::info;
use primitive_types::U256;
use rocksdb::{DB, Options, WriteBatch};
use std::sync::Arc;

/// Explorer Database - 블록체인 데이터를 인덱싱하여 저장
pub struct ExplorerDB {
    db: Arc<DB>,
}

impl ExplorerDB {
    fn hash_lookup_candidates(hash: &str) -> Vec<String> {
        let trimmed = hash.trim();
        let no_prefix = trimmed
            .strip_prefix("0x")
            .or_else(|| trimmed.strip_prefix("0X"))
            .unwrap_or(trimmed);

        let mut candidates = Vec::with_capacity(4);
        let mut push_unique = |value: String| {
            if !value.is_empty() && !candidates.iter().any(|existing| existing == &value) {
                candidates.push(value);
            }
        };

        push_unique(trimmed.to_string());
        push_unique(no_prefix.to_string());
        push_unique(trimmed.to_ascii_lowercase());
        push_unique(no_prefix.to_ascii_lowercase());

        candidates
    }

    /// 새 데이터베이스 열기 또는 생성
    pub fn new(path: &str) -> Result<Self> {
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);

        let db = DB::open(&opts, path)?;

        info!("✅ Explorer database opened at {}", path);

        let explorer_db = ExplorerDB { db: Arc::new(db) };

        // 메타데이터 초기화 (첫 실행 시 기존 데이터 스캔)
        explorer_db.initialize_metadata()?;

        Ok(explorer_db)
    }

    /// 메타데이터 초기화: 기존 블록/트랜잭션 개수를 스캔해서 설정
    fn initialize_metadata(&self) -> Result<()> {
        let block_count = self.get_block_count()?;
        let tx_count = self.get_transaction_count()?;

        if block_count == 0 {
            // DB를 스캔해서 실제 최고 블록 높이 찾기
            let mut max_height = 0u64;
            let mut iter = self.db.raw_iterator();
            iter.seek(b"b:");

            while iter.valid() {
                if let Some(key) = iter.key() {
                    if !key.starts_with(b"b:") {
                        break;
                    }

                    let key_str = String::from_utf8_lossy(key);
                    if let Some(height_str) = key_str.strip_prefix("b:") {
                        if let Ok(height) = height_str.parse::<u64>() {
                            max_height = max_height.max(height);
                        }
                    }
                }
                iter.next();
            }

            if max_height > 0 {
                // Count actual number of indexed blocks (not height + 1)
                let mut actual_count = 0u64;
                let mut iter2 = self.db.raw_iterator();
                iter2.seek(b"b:");
                while iter2.valid() {
                    if let Some(k) = iter2.key() {
                        if !k.starts_with(b"b:") { break; }
                        actual_count += 1;
                    }
                    iter2.next();
                }
                self.set_block_count(actual_count)?;
                self.db.put(b"meta:last_height", max_height.to_string().as_bytes())?;
                info!("🔧 Initialized block_count={} last_height={} from existing data", actual_count, max_height);
            }
        }

        if tx_count == 0 {
            // DB를 스캔해서 실제 트랜잭션 개수 세기
            let mut count = 0u64;
            let mut iter = self.db.raw_iterator();
            iter.seek(b"t:");

            while iter.valid() {
                if let Some(key) = iter.key() {
                    if !key.starts_with(b"t:") {
                        break;
                    }
                    count += 1;
                }
                iter.next();
            }

            if count > 0 {
                self.set_transaction_count(count)?;
                info!("🔧 Initialized tx_count to {} from existing data", count);
            }
        }

        // One-time migration: set meta:last_height if missing (upgrade from old schema)
        if self.db.get(b"meta:last_height")?.is_none() {
            let mut max_height = 0u64;
            let mut actual_count = 0u64;
            let mut iter = self.db.raw_iterator();
            iter.seek(b"b:");
            while iter.valid() {
                if let Some(key) = iter.key() {
                    if !key.starts_with(b"b:") { break; }
                    let key_str = String::from_utf8_lossy(key);
                    if let Some(h_str) = key_str.strip_prefix("b:") {
                        if let Ok(h) = h_str.parse::<u64>() {
                            max_height = max_height.max(h);
                        }
                    }
                    actual_count += 1;
                }
                iter.next();
            }
            if max_height > 0 {
                let mut batch = WriteBatch::default();
                batch.put(b"meta:last_height", max_height.to_string().as_bytes());
                // Also fix block_count if it was inflated to height+1
                batch.put(b"meta:block_count", actual_count.to_string().as_bytes());
                self.db.write(batch)?;
                info!("🔧 Migration: set last_height={}, corrected block_count={}", max_height, actual_count);
            }
        }

        // One-time migration: build meta:circulating_supply from existing coinbase TXs
        if self.db.get(b"meta:supply_migrated")?.is_none() {
            info!("🔧 Migrating circulating supply from coinbase transactions...");
            let mut supply = U256::zero();
            let mut iter = self.db.raw_iterator();
            iter.seek(b"t:");
            while iter.valid() {
                if let Some(key) = iter.key() {
                    if !key.starts_with(b"t:") { break; }
                    if let Some(value) = iter.value() {
                        if let Ok(tx) = serde_json::from_slice::<TransactionInfo>(value) {
                            if tx.from == "Block_Reward" {
                                supply += tx.amount;
                            }
                        }
                    }
                }
                iter.next();
            }
            let mut batch = WriteBatch::default();
            batch.put(b"meta:circulating_supply", supply.to_string().as_bytes());
            batch.put(b"meta:supply_migrated", b"1");
            self.db.write(batch)?;
            info!("🔧 Migration complete: circulating_supply={}", supply);
        }

        // One-time migration: build meta:address_count from existing addr: records
        if self.db.get(b"meta:addr_count_migrated")?.is_none() {
            let mut count = 0u64;
            let mut iter = self.db.raw_iterator();
            iter.seek(b"addr:");
            while iter.valid() {
                if let Some(key) = iter.key() {
                    if !key.starts_with(b"addr:") { break; }
                    count += 1;
                }
                iter.next();
            }
            let mut batch = WriteBatch::default();
            batch.put(b"meta:address_count", count.to_string().as_bytes());
            batch.put(b"meta:addr_count_migrated", b"1");
            self.db.write(batch)?;
            info!("🔧 Migration complete: address_count={}", count);
        }

        // One-time migration: build ti: time-index and meta:total_volume from existing t: records
        if self.db.get(b"meta:ti_migrated")?.is_none() {
            info!("🔧 Migrating existing transactions to time-index...");
            let mut batch = WriteBatch::default();
            let mut total_volume = U256::zero();
            let mut migrated = 0u64;

            let mut iter = self.db.raw_iterator();
            iter.seek(b"t:");
            while iter.valid() {
                if let Some(key) = iter.key() {
                    if !key.starts_with(b"t:") {
                        break;
                    }
                    if let Some(value) = iter.value() {
                        if let Ok(tx) = serde_json::from_slice::<TransactionInfo>(value) {
                            let ti_key = Self::make_ti_key(&tx);
                            batch.put(ti_key.as_bytes(), tx.hash.as_bytes());
                            total_volume += tx.amount;
                            migrated += 1;
                        }
                    }
                }
                iter.next();
            }

            batch.put(b"meta:total_volume", total_volume.to_string().as_bytes());
            batch.put(b"meta:ti_migrated", b"1");
            self.db.write(batch)?;
            info!("🔧 Migration complete: {} transactions indexed, total_volume={}", migrated, total_volume);
        }

        Ok(())
    }

    /// Build the time-sorted index key for a transaction.
    /// Inverted timestamp ensures lexicographic order = newest-first.
    fn make_ti_key(tx: &TransactionInfo) -> String {
        let ts = tx.timestamp.timestamp();
        let ts_u64 = if ts < 0 { 0u64 } else { ts as u64 };
        let inverted = u64::MAX - ts_u64;
        format!("ti:{:020}:{}", inverted, tx.hash)
    }

    /// 블록 저장
    /// Key: b:<height> -> BlockInfo (JSON)
    /// Key: bh:<hash> -> height
    pub fn save_block(&self, block: &BlockInfo) -> Result<()> {
        let block_key = format!("b:{}", block.height);

        // Dedup: only count truly new blocks
        let is_new = self.db.get(block_key.as_bytes())?.is_none();

        let mut batch = WriteBatch::default();

        // b:<height> -> BlockInfo
        let block_json = serde_json::to_string(block)?;
        batch.put(block_key.as_bytes(), block_json.as_bytes());

        // bh:<hash> -> height (해시로 블록 찾기)
        let hash_key = format!("bh:{}", block.hash);
        batch.put(hash_key.as_bytes(), block.height.to_string().as_bytes());

        if is_new {
            // Increment actual indexed block count (same pattern as tx_count)
            let current_count = self.get_block_count().unwrap_or(0);
            batch.put(b"meta:block_count", (current_count + 1).to_string().as_bytes());
        }

        // Track highest indexed height separately (used for pagination)
        let prev_last = self.get_last_indexed_height().unwrap_or(0);
        if block.height >= prev_last {
            batch.put(b"meta:last_height", block.height.to_string().as_bytes());
        }

        self.db.write(batch)?;
        Ok(())
    }

    /// 가장 높게 인덱싱된 블록 높이 (페이징 기준점)
    pub fn get_last_indexed_height(&self) -> Result<u64> {
        match self.db.get(b"meta:last_height")? {
            Some(data) => Ok(String::from_utf8(data.to_vec())?.parse()?),
            None => Ok(0),
        }
    }

    /// 최신 블록 정보 조회
    pub fn get_latest_block(&self) -> Result<Option<BlockInfo>> {
        let h = self.get_last_indexed_height()?;
        if h == 0 && self.get_block_count()? == 0 {
            return Ok(None);
        }
        self.get_block_by_height(h)
    }

    /// 최근 블록들의 평균 생성 간격 (초) 계산
    pub fn compute_avg_block_time(&self, sample: u64) -> Result<f64> {
        let last = self.get_last_indexed_height()?;
        if last < 2 {
            return Ok(0.0);
        }
        let from = last.saturating_sub(sample);
        let mut times: Vec<i64> = Vec::new();
        for h in from..=last {
            if let Some(b) = self.get_block_by_height(h)? {
                times.push(b.timestamp.timestamp());
            }
        }
        if times.len() < 2 {
            return Ok(0.0);
        }
        let span = times.last().unwrap() - times.first().unwrap();
        Ok(span as f64 / (times.len() - 1) as f64)
    }

    /// 블록 조회 (높이로)
    pub fn get_block_by_height(&self, height: u64) -> Result<Option<BlockInfo>> {
        let key = format!("b:{}", height);
        match self.db.get(key.as_bytes())? {
            Some(data) => {
                let mut block: BlockInfo = serde_json::from_slice(&data)?;
                // Calculate confirmations based on chain tip height
                let tip = self.get_last_indexed_height()?;
                block.confirmations = if tip > height { tip - height } else { 0 };
                Ok(Some(block))
            }
            None => Ok(None),
        }
    }

    /// 블록 조회 (해시로)
    pub fn get_block_by_hash(&self, hash: &str) -> Result<Option<BlockInfo>> {
        for candidate in Self::hash_lookup_candidates(hash) {
            let hash_key = format!("bh:{}", candidate);
            if let Some(height_bytes) = self.db.get(hash_key.as_bytes())? {
                let height_str = String::from_utf8(height_bytes.to_vec())?;
                let height: u64 = height_str.parse()?;
                return self.get_block_by_height(height);
            }
        }

        Ok(None)
    }

    /// 모든 블록 조회 (페이징)
    pub fn get_blocks(&self, page: u32, limit: u32) -> Result<Vec<BlockInfo>> {
        let last_height = self.get_last_indexed_height()?;
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

        // 최신 블록 높이(last_height)부터 시작
        let start_height = last_height.saturating_sub(skip);
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
    /// Key: t:<hash>                        -> TransactionInfo (JSON)
    /// Key: ti:<inverted_ts_padded>:<hash>  -> hash  (newest-first time index)
    /// Key: ta:<address>:<timestamp>:<hash> -> ""    (주소별 트랜잭션 인덱스)
    /// Key: tb:<height>:<hash>              -> hash  (블록별 트랜잭션 인덱스)
    pub fn save_transaction(&self, tx: &TransactionInfo) -> Result<()> {
        let tx_key = format!("t:{}", tx.hash);

        // Dedup: only count and accumulate volume for truly new transactions
        let is_new = self.db.get(tx_key.as_bytes())?.is_none();

        let mut batch = WriteBatch::default();

        // t:<hash> -> TransactionInfo
        let tx_json = serde_json::to_string(tx)?;
        batch.put(tx_key.as_bytes(), tx_json.as_bytes());

        // ti:<inverted_ts>:<hash> -> hash  (O(1) paginated newest-first queries)
        let ti_key = Self::make_ti_key(tx);
        batch.put(ti_key.as_bytes(), tx.hash.as_bytes());

        // ta:<address>:<timestamp>:<hash> -> "" (from 주소)
        let from_key = format!("ta:{}:{}:{}", tx.from, tx.timestamp.timestamp(), tx.hash);
        batch.put(from_key.as_bytes(), b"");

        // ta:<address>:<timestamp>:<hash> -> "" (to 주소)
        let to_key = format!("ta:{}:{}:{}", tx.to, tx.timestamp.timestamp(), tx.hash);
        batch.put(to_key.as_bytes(), b"");

        // tb:<height>:<hash> -> hash (블록별 인덱스)
        if let Some(height) = tx.block_height {
            let block_tx_key = format!("tb:{}:{}", height, tx.hash);
            batch.put(block_tx_key.as_bytes(), tx.hash.as_bytes());
        }

        if is_new {
            // meta:tx_count 증가
            let current_count = self.get_transaction_count().unwrap_or(0);
            batch.put(b"meta:tx_count", (current_count + 1).to_string().as_bytes());

            // meta:total_volume 누적
            let current_volume = self.get_total_volume().unwrap_or(U256::zero());
            let new_volume = current_volume + tx.amount;
            batch.put(b"meta:total_volume", new_volume.to_string().as_bytes());

            // meta:circulating_supply 누적 (coinbase TX만)
            if tx.from == "Block_Reward" {
                let current_supply = self.get_circulating_supply().unwrap_or(U256::zero());
                batch.put(b"meta:circulating_supply", (current_supply + tx.amount).to_string().as_bytes());
            }
        }

        self.db.write(batch)?;
        Ok(())
    }

    /// 트랜잭션 조회
    pub fn get_transaction(&self, hash: &str) -> Result<Option<TransactionInfo>> {
        let candidates = Self::hash_lookup_candidates(hash);
        log::debug!("🔍 Looking up transaction with {} candidate(s): {:?}", candidates.len(), candidates);

        for (i, candidate) in candidates.iter().enumerate() {
            let key = format!("t:{}", candidate);
            log::debug!("   Attempt {}/{}: trying key '{}'", i + 1, candidates.len(), key);

            if let Some(data) = self.db.get(key.as_bytes())? {
                log::debug!("✅ Transaction found using candidate: {}", candidate);
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
                return Ok(Some(tx));
            }
        }

        log::warn!("❌ Transaction not found in DB after trying all candidates");
        Ok(None)
    }

    /// 모든 트랜잭션 조회 (페이징) — O(page * limit) via time-sorted index
    pub fn get_transactions(&self, page: u32, limit: u32) -> Result<Vec<TransactionInfo>> {
        let prefix = b"ti:" as &[u8];
        let mut iter = self.db.raw_iterator();
        iter.seek(prefix);

        let skip = ((page - 1) * limit) as usize;
        let mut skipped = 0usize;
        let mut results = Vec::with_capacity(limit as usize);

        while iter.valid() && results.len() < limit as usize {
            if let Some(key) = iter.key() {
                if !key.starts_with(prefix) {
                    break;
                }

                if skipped < skip {
                    skipped += 1;
                    iter.next();
                    continue;
                }

                if let Some(hash_bytes) = iter.value() {
                    let hash = String::from_utf8_lossy(hash_bytes);
                    if let Some(tx) = self.get_transaction(&hash)? {
                        results.push(tx);
                    }
                }
            }
            iter.next();
        }

        Ok(results)
    }

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
        let is_new = self.db.get(key.as_bytes())?.is_none();
        let mut batch = WriteBatch::default();
        let json = serde_json::to_string(info)?;
        batch.put(key.as_bytes(), json.as_bytes());
        if is_new {
            let current_count = self.get_address_count().unwrap_or(0);
            batch.put(b"meta:address_count", (current_count + 1).to_string().as_bytes());
        }
        self.db.write(batch)?;
        Ok(())
    }

    /// 주소 정보 조회
    pub fn get_address_info(&self, address: &str) -> Result<Option<AddressInfo>> {
        log::debug!("DB: Getting address info for: {}", address);
        let key = format!("addr:{}", address);
        match self.db.get(key.as_bytes())? {
            Some(data) => {
                let info: AddressInfo = serde_json::from_slice(&data)?;
                log::debug!("DB: Found address info - balance: {}", info.balance);
                Ok(Some(info))
            }
            None => {
                log::debug!("DB: Address info not found for: {}", address);
                Ok(None)
            }
        }
    }

    /// 주소 정보 계산 및 저장 (UTXO 기반 잔액 + 트랜잭션 통계)
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

        // UTXO DB에서 정확한 잔액 계산 (트랜잭션 집계의 누락 문제를 보완)
        let balance = self.get_balance_from_utxos(address).unwrap_or_else(|_| {
            if received > sent { received - sent } else { U256::zero() }
        });

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

    /// 총 거래량 조회 (running counter)
    pub fn get_total_volume(&self) -> Result<U256> {
        match self.db.get(b"meta:total_volume")? {
            Some(data) => {
                let s = String::from_utf8(data.to_vec())?;
                Ok(U256::from_dec_str(&s).unwrap_or(U256::zero()))
            }
            None => Ok(U256::zero()),
        }
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

    /// 데이터베이스 통계 — O(1): reads metadata counters only
    pub fn get_stats(&self) -> Result<(u64, u64, U256)> {
        let block_count = self.get_block_count()?;
        let tx_count = self.get_transaction_count()?;
        let total_volume = self.get_total_volume()?;
        Ok((block_count, tx_count, total_volume))
    }

    /// 총 주소 수 조회
    pub fn get_address_count(&self) -> Result<u64> {
        match self.db.get(b"meta:address_count")? {
            Some(data) => Ok(String::from_utf8(data.to_vec())?.parse()?),
            None => Ok(0),
        }
    }

    /// 발행된 총 코인 공급량 조회
    pub fn get_circulating_supply(&self) -> Result<U256> {
        match self.db.get(b"meta:circulating_supply")? {
            Some(data) => {
                let s = String::from_utf8(data.to_vec())?;
                Ok(U256::from_dec_str(&s).unwrap_or(U256::zero()))
            }
            None => Ok(U256::zero()),
        }
    }

    // ----------------------------------------------------------------
    // UTXO 저장소: eu:{txid}:{vout} -> "{address}:{amount_hex}"
    // ----------------------------------------------------------------

    /// UTXO DB가 초기화되었는지 확인
    pub fn has_utxo_data(&self) -> bool {
        let mut iter = self.db.raw_iterator();
        iter.seek(b"eu:");
        iter.valid() && iter.key().map_or(false, |k| k.starts_with(b"eu:"))
    }

    /// (내부용) 단일 UTXO 저장 - apply_utxo_changes 사용 권장
    #[allow(dead_code)]
    pub fn save_utxo(&self, txid: &str, vout: u32, address: &str, amount: U256) -> Result<()> {
        let key = format!("eu:{}:{}:{}", address, txid, vout);
        let value = format!("{:x}", amount);
        self.db.put(key.as_bytes(), value.as_bytes())?;
        Ok(())
    }

    /// (내부용) 단일 UTXO 삭제 - apply_utxo_changes 사용 권장
    #[allow(dead_code)]
    pub fn remove_utxo(&self, txid: &str, vout: u32, address: &str) -> Result<()> {
        let key = format!("eu:{}:{}:{}", address, txid, vout);
        self.db.delete(key.as_bytes())?;
        Ok(())
    }

    /// UTXO 저장/삭제를 WriteBatch로 한 번에 처리
    /// - created: (txid, vout, address, amount)
    /// - spent:   (txid, vout, address)  ← address가 ""이면 역방향 인덱스로 조회
    pub fn apply_utxo_changes(
        &self,
        created: &[(String, u32, String, U256)],
        spent: &[(String, u32, String)],
    ) -> Result<()> {
        let mut batch = WriteBatch::default();

        // 생성: 정방향(eu:) + 역방향(eur:) 인덱스 함께 저장
        for (txid, vout, address, amount) in created {
            let key = format!("eu:{}:{}:{}", address, txid, vout);
            batch.put(key.as_bytes(), format!("{:x}", amount).as_bytes());
            // 역방향 인덱스: eur:{txid}:{vout} → address
            let rev_key = format!("eur:{}:{}", txid, vout);
            batch.put(rev_key.as_bytes(), address.as_bytes());
        }

        // 소비: 역방향 인덱스로 주소 확인 후 두 키 모두 삭제
        for (txid, vout, address) in spent {
            let rev_key = format!("eur:{}:{}", txid, vout);

            // 주소가 없으면(증분 sync) 역방향 인덱스에서 조회
            let real_address = if !address.is_empty() {
                address.clone()
            } else {
                self.db.get(rev_key.as_bytes())?
                    .and_then(|v| String::from_utf8(v.to_vec()).ok())
                    .unwrap_or_default()
            };

            if !real_address.is_empty() {
                let key = format!("eu:{}:{}:{}", real_address, txid, vout);
                batch.delete(key.as_bytes());
            }
            batch.delete(rev_key.as_bytes());
        }

        self.db.write(batch)?;
        Ok(())
    }

    /// 특정 주소의 UTXO 합계로 잔액 계산 (주소 prefix seek으로 빠름)
    pub fn get_balance_from_utxos(&self, address: &str) -> Result<U256> {
        let mut balance = U256::zero();
        let prefix = format!("eu:{}:", address);
        let mut iter = self.db.raw_iterator();
        iter.seek(prefix.as_bytes());
        while iter.valid() {
            if let Some(key) = iter.key() {
                if !key.starts_with(prefix.as_bytes()) { break; }
                if let Some(value) = iter.value() {
                    if let Ok(s) = std::str::from_utf8(value) {
                        if let Ok(amount) = U256::from_str_radix(s, 16) {
                            balance += amount;
                        }
                    }
                }
            }
            iter.next();
        }
        Ok(balance)
    }

    /// 전체 주소별 UTXO 잔액 집계 (richlist용)
    pub fn get_all_balances_from_utxos(&self) -> Result<Vec<(String, U256)>> {
        let mut map: std::collections::HashMap<String, U256> = std::collections::HashMap::new();
        let mut iter = self.db.raw_iterator();
        iter.seek(b"eu:");
        while iter.valid() {
            if let Some(key) = iter.key() {
                if !key.starts_with(b"eu:") { break; }
                // 키: eu:{address}:{txid}:{vout}
                if let Ok(key_str) = std::str::from_utf8(key) {
                    let parts: Vec<&str> = key_str.splitn(4, ':').collect();
                    // parts[0]="eu", parts[1]=address, parts[2]=txid, parts[3]=vout
                    if parts.len() == 4 {
                        let address = parts[1];
                        if let Some(value) = iter.value() {
                            if let Ok(s) = std::str::from_utf8(value) {
                                if let Ok(amount) = U256::from_str_radix(s, 16) {
                                    *map.entry(address.to_string()).or_insert_with(U256::zero) += amount;
                                }
                            }
                        }
                    }
                }
            }
            iter.next();
        }
        let mut result: Vec<_> = map.into_iter().filter(|(_, b)| !b.is_zero()).collect();
        result.sort_by(|a, b| b.1.cmp(&a.1));
        Ok(result)
    }

    // ----------------------------------------------------------------

    /// 부자 리스트 조회: 잔액 기준 상위 N개 주소 (UTXO 기반)
    pub fn get_richlist(&self, limit: usize) -> Result<Vec<RichListEntry>> {
        let total_supply = self.get_circulating_supply().unwrap_or(U256::zero());

        // UTXO DB에서 주소별 잔액 집계 (정확한 값)
        let mut entries = self.get_all_balances_from_utxos()?;
        entries.truncate(limit);

        let supply_f64 = if total_supply.is_zero() {
            1.0f64
        } else {
            // Use u128 for precision (supply fits within u128 for practical amounts)
            total_supply.low_u128() as f64
        };

        let result = entries
            .into_iter()
            .enumerate()
            .map(|(i, (address, balance))| {
                let pct = balance.low_u128() as f64 / supply_f64 * 100.0;
                RichListEntry {
                    rank: i + 1,
                    address,
                    balance,
                    percentage: (pct * 100.0).round() / 100.0,
                }
            })
            .collect();

        Ok(result)
    }

    /// 데이터베이스 초기화 (재동기화용)
    #[allow(dead_code)]
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
