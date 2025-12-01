pub mod p2p;
pub mod server;

pub use p2p::*;
pub use server::*;

use netcoin_core::Blockchain;
use netcoin_core::block::Block;
use netcoin_core::transaction::Transaction;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

pub struct NodeState {
    pub bc: Blockchain,
    pub blockchain: Vec<Block>,
    pub pending: Vec<Transaction>,
    pub seen_tx: HashSet<String>,
    pub p2p: Arc<PeerManager>,
}

pub type NodeHandle = Arc<Mutex<NodeState>>;
