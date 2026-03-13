pub mod p2p;
pub mod server;

pub use crate::p2p::manager::PeerManager;
pub use server::*;

use Astram_core::Blockchain;
use Astram_core::block::Block;
use Astram_core::transaction::Transaction;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub struct NodeHandles {
    pub bc: Arc<Mutex<Blockchain>>,
    pub mempool: Arc<Mutex<MempoolState>>,
    pub mining: Arc<MiningState>,
}

// Lock order (when nested): bc -> chain -> mempool -> mining -> meta.

pub struct ChainState {
    pub blockchain: Vec<Block>,
    /// Orphan blocks pool: blocks waiting for their parent
    /// Key: block hash, Value: (block, received_timestamp)
    /// Security: Limited to MAX_ORPHAN_BLOCKS to prevent memory exhaustion attacks
    pub orphan_blocks: HashMap<String, (Block, i64)>,
    /// Recently mined block hashes (to ignore when received from peers)
    /// Key: block hash, Value: timestamp when mined
    pub recently_mined_blocks: HashMap<String, i64>,
}

impl Default for ChainState {
    fn default() -> Self {
        Self {
            blockchain: Vec::new(),
            orphan_blocks: HashMap::new(),
            recently_mined_blocks: HashMap::new(),
        }
    }
}

pub struct NodeMeta {
    /// Miner wallet address for this node
    pub miner_address: Arc<Mutex<String>>,
    /// My public IP address as registered with DNS server
    pub my_public_address: Arc<Mutex<Option<String>>>,
    pub node_start_time: std::time::Instant,
}

pub struct MiningState {
    /// Flag to cancel ongoing mining when a new block is received from network
    pub cancel_flag: Arc<std::sync::atomic::AtomicBool>,
    /// Mining status information
    pub active: Arc<std::sync::atomic::AtomicBool>,
    pub current_difficulty: Arc<Mutex<u32>>,
    pub current_hashrate: Arc<Mutex<f64>>,
    pub blocks_mined: Arc<std::sync::atomic::AtomicU64>,
}

impl Default for MiningState {
    fn default() -> Self {
        Self {
            cancel_flag: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            active: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            current_difficulty: Arc::new(Mutex::new(1)),
            current_hashrate: Arc::new(Mutex::new(0.0)),
            blocks_mined: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        }
    }
}

pub struct MempoolState {
    pub pending: Vec<Transaction>,
    /// Seen transactions with timestamp (to prevent relay loops and track when seen)
    /// Key: txid, Value: timestamp when first seen
    pub seen_tx: HashMap<String, i64>,
}

impl Default for MempoolState {
    fn default() -> Self {
        Self {
            pending: Vec::new(),
            seen_tx: HashMap::new(),
        }
    }
}

/// Security constants for node limits
pub const MAX_ORPHAN_BLOCKS: usize = 100; // Maximum orphan blocks to cache
pub const MAX_MEMORY_BLOCKS: usize = 500; // Maximum blocks to keep in memory
pub const ORPHAN_TIMEOUT: i64 = 1800; // 30 minutes - orphans older than this are dropped

/// Mempool DoS protection constants
pub const MAX_MEMPOOL_SIZE: usize = 10000; // Maximum transactions in mempool
pub const MAX_MEMPOOL_BYTES: usize = 300_000_000; // 300MB max mempool size
pub const MEMPOOL_EXPIRY_TIME: i64 = 86400; // 24 hours - old transactions expire
pub const MIN_RELAY_FEE_PER_BYTE: u64 = 1_000_000; // 1 Gwei per byte minimum

pub type NodeHandle = Arc<NodeHandles>;

impl ChainState {
    /// Security: Enforce memory block limit by removing oldest blocks
    /// Keeps only the most recent MAX_MEMORY_BLOCKS in memory
    pub fn enforce_memory_limit(&mut self) {
        if self.blockchain.len() > MAX_MEMORY_BLOCKS {
            let excess = self.blockchain.len() - MAX_MEMORY_BLOCKS;
            log::debug!(
                "[DEBUG] Memory block limit reached: {} blocks (max: {}), removing {} oldest blocks",
                self.blockchain.len(),
                MAX_MEMORY_BLOCKS,
                excess
            );

            // Remove oldest blocks (from the front)
            self.blockchain.drain(0..excess);

            log::debug!(
                "[DEBUG] Memory optimized: {} blocks remaining in memory",
                self.blockchain.len()
            );
        }
    }
}

impl MempoolState {
    /// Security: Enforce mempool limits to prevent DoS attacks
    /// Evicts low-fee or old transactions when limits are exceeded
    pub fn enforce_mempool_limit(&mut self) {
        use primitive_types::U256;

        let now = chrono::Utc::now().timestamp();

        // 1. Remove expired transactions (older than 24 hours)
        // Collect expired txids first, then retain, to update seen_tx correctly.
        let before = self.pending.len();
        self.pending.retain(|tx| now - tx.timestamp <= MEMPOOL_EXPIRY_TIME);
        let expired_count = before - self.pending.len();
        if expired_count > 0 {
            log::info!(
                "[INFO] Removed {} expired transactions from mempool",
                expired_count
            );
        }

        // Early exit: skip expensive sort + serialize when clearly under limits.
        // Assume max ~10 KB per tx for the byte estimate.
        let needs_count_eviction = self.pending.len() > MAX_MEMPOOL_SIZE;
        let possibly_over_bytes = self.pending.len() > MAX_MEMPOOL_BYTES / 10_240;
        if !needs_count_eviction && !possibly_over_bytes {
            return;
        }

        // 2. Sort by fee-per-byte ascending (lowest = evict first).
        //    sort_by_cached_key serializes each tx once for the key.
        self.pending.sort_by_cached_key(|tx| {
            let tx_bytes =
                bincode::encode_to_vec(tx, Astram_core::blockchain::BINCODE_CONFIG.clone())
                    .unwrap_or_default();
            let tx_size = tx_bytes.len().max(1) as u64;

            let input_sum: U256 = tx
                .inputs
                .iter()
                .filter_map(|_| Some(U256::from(1_000_000_000_000_000_000u64))) // Estimate
                .fold(U256::zero(), |acc, amt| acc + amt);

            let output_sum: U256 = tx
                .outputs
                .iter()
                .map(|out| out.amount())
                .fold(U256::zero(), |acc, amt| acc + amt);

            let fee = if input_sum > output_sum {
                (input_sum - output_sum).as_u64()
            } else {
                0
            };

            fee / tx_size
        });

        // 3. Compute per-tx sizes once — O(N) — for both count and byte-limit evictions.
        let sizes: Vec<usize> = self
            .pending
            .iter()
            .map(|tx| {
                bincode::encode_to_vec(tx, Astram_core::blockchain::BINCODE_CONFIG.clone())
                    .map(|b| b.len())
                    .unwrap_or(0)
            })
            .collect();

        let total_bytes: usize = sizes.iter().sum();

        // How many to drop for count limit (front = lowest fee = evict first).
        let count_excess = self.pending.len().saturating_sub(MAX_MEMPOOL_SIZE);

        // How many to drop for byte limit (continuing from count_excess offset).
        let mut remove_count = count_excess;
        if total_bytes > MAX_MEMPOOL_BYTES {
            let mut remaining_bytes = total_bytes;
            // Subtract the bytes already accounted for by count eviction.
            for i in 0..count_excess {
                remaining_bytes = remaining_bytes.saturating_sub(sizes[i]);
            }
            while remaining_bytes > MAX_MEMPOOL_BYTES && remove_count < sizes.len() {
                remaining_bytes = remaining_bytes.saturating_sub(sizes[remove_count]);
                remove_count += 1;
            }
            log::warn!(
                "[WARN] Mempool size limit exceeded: {} bytes (max: {} MB)",
                total_bytes,
                MAX_MEMPOOL_BYTES / 1_000_000
            );
        } else if count_excess > 0 {
            log::warn!(
                "[WARN] Mempool transaction limit reached: {} txs (max: {})",
                self.pending.len(),
                MAX_MEMPOOL_SIZE
            );
        }

        // Single O(remove_count) drain — no repeated O(N) remove(0) calls.
        if remove_count > 0 {
            let removed_txids: Vec<String> = self
                .pending
                .drain(..remove_count)
                .map(|tx| tx.txid)
                .collect();
            for txid in removed_txids {
                self.seen_tx.remove(&txid);
            }
            log::info!(
                "[INFO] Evicted {} transactions from mempool (count_excess={}, total_bytes={})",
                remove_count, count_excess, total_bytes
            );
        }
    }
}
