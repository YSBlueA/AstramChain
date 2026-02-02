// node/src/p2p/messages.rs

use bincode::{Decode, Encode};
use netcoin_core::block::Block;
use netcoin_core::block::BlockHeader;
use netcoin_core::transaction::Transaction;

/// Peer handshake information
#[derive(Debug, Clone, Encode, Decode)]
pub struct HandshakeInfo {
    pub protocol_version: u32,
    pub software_version: String,
    pub supported_features: Vec<String>,
    pub network_id: String,
    pub chain_id: u64,
    pub height: u64,
}

/// (inv/getdata)
#[derive(Debug, Clone, Encode, Decode)]
pub enum InventoryType {
    Error = 0,
    Transaction = 1,
    Block = 2,
}

/// message type
#[derive(Debug, Clone, Encode, Decode)]
pub enum P2pMessage {
    Handshake {
        info: HandshakeInfo,
    },
    HandshakeAck {
        info: HandshakeInfo,
    },
    Version {
        version: String,
        height: u64,
    },
    VerAck,
    GetHeaders {
        locator_hashes: Vec<Vec<u8>>,
        stop_hash: Option<Vec<u8>>,
    },
    Headers {
        headers: Vec<BlockHeader>,
    },
    Inv {
        object_type: InventoryType,
        hashes: Vec<Vec<u8>>,
    },
    GetData {
        object_type: InventoryType,
        hashes: Vec<Vec<u8>>,
    },
    Block {
        block: Block,
    },
    Tx {
        tx: Transaction,
    },
    Ping(u64),
    Pong(u64),
}
