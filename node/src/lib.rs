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
            log::warn!(
                "[WARN] Memory block limit reached: {} blocks (max: {}), removing {} oldest blocks",
                self.blockchain.len(),
                MAX_MEMORY_BLOCKS,
                excess
            );

            // Remove oldest blocks (from the front)
            self.blockchain.drain(0..excess);

            log::info!(
                "[INFO] Memory optimized: {} blocks remaining in memory",
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
        let initial_count = self.pending.len();
        self.pending.retain(|tx| {
            let age = now - tx.timestamp;
            if age > MEMPOOL_EXPIRY_TIME {
                self.seen_tx.remove(&tx.txid);
                false
            } else {
                true
            }
        });

        let expired_count = initial_count - self.pending.len();
        if expired_count > 0 {
            log::info!(
                "[INFO] Removed {} expired transactions from mempool",
                expired_count
            );
        }

        // 2. Check transaction count limit
        if self.pending.len() > MAX_MEMPOOL_SIZE {
            let excess = self.pending.len() - MAX_MEMPOOL_SIZE;
            log::warn!(
                "[WARN] Mempool transaction limit reached: {} txs (max: {})",
                self.pending.len(),
                MAX_MEMPOOL_SIZE
            );

            // Sort by fee rate (fee per byte) - lowest first for eviction
            self.pending.sort_by_cached_key(|tx| {
                let tx_bytes =
                    bincode::encode_to_vec(tx, Astram_core::blockchain::BINCODE_CONFIG.clone())
                        .unwrap_or_default();
                let tx_size = tx_bytes.len().max(1) as u64;

                // Calculate total fee
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

                // Fee per byte (lower = evict first)
                fee / tx_size
            });

            // Remove lowest fee transactions
            for _ in 0..excess {
                if let Some(tx) = self.pending.first() {
                    let txid = tx.txid.clone();
                    self.pending.remove(0);
                    self.seen_tx.remove(&txid);
                }
            }

            log::info!(
                "[INFO] Evicted {} low-fee transactions from mempool",
                excess
            );
        }

        // 3. Check total mempool byte size
        let total_bytes: usize = self
            .pending
            .iter()
            .filter_map(|tx| {
                bincode::encode_to_vec(tx, Astram_core::blockchain::BINCODE_CONFIG.clone()).ok()
            })
            .map(|bytes| bytes.len())
            .sum();

        if total_bytes > MAX_MEMPOOL_BYTES {
            log::warn!(
                "[WARN] Mempool size limit exceeded: {} bytes (max: {} MB)",
                total_bytes,
                MAX_MEMPOOL_BYTES / 1_000_000
            );

            // Already sorted by fee rate, remove more low-fee txs
            while !self.pending.is_empty() {
                let current_size: usize = self
                    .pending
                    .iter()
                    .filter_map(|tx| {
                        bincode::encode_to_vec(tx, Astram_core::blockchain::BINCODE_CONFIG.clone())
                            .ok()
                    })
                    .map(|bytes| bytes.len())
                    .sum();

                if current_size <= MAX_MEMPOOL_BYTES {
                    break;
                }

                if let Some(tx) = self.pending.first() {
                    let txid = tx.txid.clone();
                    self.pending.remove(0);
                    self.seen_tx.remove(&txid);
                }
            }
        }
    }
}
