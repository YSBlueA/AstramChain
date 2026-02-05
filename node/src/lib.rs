pub mod p2p;
pub mod server;

pub use crate::p2p::manager::PeerManager;
pub use server::*;

use netcoin_core::Blockchain;
use netcoin_core::block::Block;
use netcoin_core::transaction::Transaction;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub struct NodeState {
    pub bc: Blockchain,
    pub blockchain: Vec<Block>,
    pub pending: Vec<Transaction>,
    /// Seen transactions with timestamp (to prevent relay loops and track when seen)
    /// Key: txid, Value: timestamp when first seen
    pub seen_tx: HashMap<String, i64>,
    pub p2p: Arc<PeerManager>,
    /// Maps Ethereum transaction hash to NetCoin UTXO txid (for MetaMask compatibility)
    pub eth_to_netcoin_tx: HashMap<String, String>,
    /// Flag to cancel ongoing mining when a new block is received from network
    pub mining_cancel_flag: Arc<std::sync::atomic::AtomicBool>,
    /// Orphan blocks pool: blocks waiting for their parent
    /// Key: block hash, Value: (block, received_timestamp)
    /// 🔒 Security: Limited to MAX_ORPHAN_BLOCKS to prevent memory exhaustion attacks
    pub orphan_blocks: HashMap<String, (Block, i64)>,
    /// Mining status information
    pub mining_active: Arc<std::sync::atomic::AtomicBool>,
    pub current_difficulty: Arc<Mutex<u32>>,
    pub current_hashrate: Arc<Mutex<f64>>,
    pub blocks_mined: Arc<std::sync::atomic::AtomicU64>,
    pub node_start_time: std::time::Instant,
    /// Miner wallet address for this node
    pub miner_address: Arc<Mutex<String>>,
    /// Recently mined block hashes (to ignore when received from peers)
    /// Key: block hash, Value: timestamp when mined
    pub recently_mined_blocks: HashMap<String, i64>,
    /// My public IP address as registered with DNS server
    pub my_public_address: Arc<Mutex<Option<String>>>,
}

/// Security constants for node limits
pub const MAX_ORPHAN_BLOCKS: usize = 100; // Maximum orphan blocks to cache
pub const MAX_MEMORY_BLOCKS: usize = 500; // Maximum blocks to keep in memory
pub const ORPHAN_TIMEOUT: i64 = 1800; // 30 minutes - orphans older than this are dropped

pub type NodeHandle = Arc<Mutex<NodeState>>;

impl NodeState {
    /// 🔒 Security: Enforce memory block limit by removing oldest blocks
    /// Keeps only the most recent MAX_MEMORY_BLOCKS in memory
    pub fn enforce_memory_limit(&mut self) {
        if self.blockchain.len() > MAX_MEMORY_BLOCKS {
            let excess = self.blockchain.len() - MAX_MEMORY_BLOCKS;
            log::warn!(
                "⚠️ Memory block limit reached: {} blocks (max: {}), removing {} oldest blocks",
                self.blockchain.len(),
                MAX_MEMORY_BLOCKS,
                excess
            );

            // Remove oldest blocks (from the front)
            self.blockchain.drain(0..excess);

            log::info!(
                "✅ Memory optimized: {} blocks remaining in memory",
                self.blockchain.len()
            );
        }
    }
}
