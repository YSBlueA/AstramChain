pub mod p2p;
pub mod server;

pub use crate::p2p::manager::PeerManager;
pub use server::*;

use netcoin_core::Blockchain;
use netcoin_core::block::Block;
use netcoin_core::transaction::Transaction;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

pub struct NodeState {
    pub bc: Blockchain,
    pub blockchain: Vec<Block>,
    pub pending: Vec<Transaction>,
    pub seen_tx: HashSet<String>,
    pub p2p: Arc<PeerManager>,
    /// Maps Ethereum transaction hash to NetCoin UTXO txid (for MetaMask compatibility)
    pub eth_to_netcoin_tx: HashMap<String, String>,
    /// Flag to cancel ongoing mining when a new block is received from network
    pub mining_cancel_flag: Arc<std::sync::atomic::AtomicBool>,
    /// Orphan blocks pool: blocks waiting for their parent
    /// Key: block hash, Value: (block, received_timestamp)
    pub orphan_blocks: HashMap<String, (Block, i64)>,
}

pub type NodeHandle = Arc<Mutex<NodeState>>;
