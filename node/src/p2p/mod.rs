pub mod manager;
pub mod messages;
pub mod peer;

pub use manager::PeerManager;
pub use messages::P2pMessage;

pub struct SavedPeer {
    addr: String,
    last_seen: u64,
}
