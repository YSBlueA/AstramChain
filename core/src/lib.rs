pub mod block;
pub mod blockchain;
pub mod config;
pub mod consensus;
pub mod crypto;
pub mod db;
pub mod network;
pub mod security;
pub mod transaction;
pub mod utxo;
pub mod wallet;

// Explicit re-exports to avoid ambiguous glob re-exports
pub use block::{Block, BlockHeader, compute_header_hash, compute_merkle_root};
pub use blockchain::Blockchain;
pub use crypto::WalletKeypair;
pub use transaction::{Transaction, TransactionInput, TransactionOutput};
pub use wallet::Wallet;
