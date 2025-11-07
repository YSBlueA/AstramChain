pub mod server;

pub use server::*;

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