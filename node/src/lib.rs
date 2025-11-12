pub mod server;
pub mod p2p;

pub use server::*;
pub use p2p::*;

use netcoin_core::Blockchain;
use netcoin_core::block::Block;
use netcoin_core::transaction::Transaction;
use std::sync::{Arc, Mutex};

pub struct NodeState {
    pub bc: Blockchain,
    pub blockchain: Vec<Block>,
    pub pending: Vec<Transaction>,
}

pub type NodeHandle = Arc<Mutex<NodeState>>;