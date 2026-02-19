use crate::p2p::messages::{HandshakeInfo, InventoryType, P2pMessage};
use crate::p2p::peer::{Peer, PeerId};
use Astram_core::block;
use Astram_core::transaction::Transaction;
use bincode::{Decode, Encode};
use bytes::Bytes;
use futures::SinkExt;
use futures::StreamExt;
use futures::future;
use hex;
use log::{info, warn};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::sync::Arc;
use std::sync::OnceLock;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio_util::codec::{FramedRead, FramedWrite, LengthDelimitedCodec};

#[derive(Encode, Decode, Debug, serde::Serialize, serde::Deserialize)]
pub struct SavedPeer {
    pub addr: String,
    pub last_seen: u64,
}

pub const MAX_OUTBOUND: usize = 8;
pub const PEERS_FILE: &str = "peers.json";
pub const PROTOCOL_VERSION: u32 = 1;
pub const MAINNET_NETWORK_ID: &str = "Astram-mainnet";
pub const TESTNET_NETWORK_ID: &str = "Astram-testnet";
pub const MAINNET_CHAIN_ID: u64 = 1;
pub const TESTNET_CHAIN_ID: u64 = 8888;

static NETWORK_ID: OnceLock<String> = OnceLock::new();
static CHAIN_ID: OnceLock<u64> = OnceLock::new();

fn resolve_network_id() -> &'static str {
    NETWORK_ID
        .get_or_init(|| {
            if let Ok(value) = std::env::var("ASTRAM_NETWORK_ID") {
                let trimmed = value.trim();
                if !trimmed.is_empty() {
                    return trimmed.to_string();
                }
            }

            let network = std::env::var("ASTRAM_NETWORK").unwrap_or_else(|_| "mainnet".to_string());
            if network.eq_ignore_ascii_case("testnet") {
                TESTNET_NETWORK_ID.to_string()
            } else {
                MAINNET_NETWORK_ID.to_string()
            }
        })
        .as_str()
}

fn resolve_chain_id() -> u64 {
    *CHAIN_ID.get_or_init(|| {
        if let Ok(value) = std::env::var("ASTRAM_CHAIN_ID") {
            if let Ok(parsed) = value.trim().parse::<u64>() {
                return parsed;
            }
        }

        let network = std::env::var("ASTRAM_NETWORK").unwrap_or_else(|_| "mainnet".to_string());
        if network.eq_ignore_ascii_case("testnet") {
            TESTNET_CHAIN_ID
        } else {
            MAINNET_CHAIN_ID
        }
    })
}

// Security: Network-level protection constants
pub const MAX_PEERS_PER_IP: usize = 3; // Maximum connections from same IP
pub const HANDSHAKE_TIMEOUT_SECS: u64 = 30; // Handshake must complete within 30s
pub const MAX_INV_PER_MESSAGE: usize = 50000; // Maximum inventory items per message
pub const BLOCK_ANNOUNCE_RATE_LIMIT: u64 = 10; // Max block announcements per minute per peer

// Security: Peer diversity for Eclipse attack protection
pub const MAX_PEERS_PER_SUBNET_24: usize = 2; // Max peers from same /24 subnet
pub const MAX_PEERS_PER_SUBNET_16: usize = 4; // Max peers from same /16 subnet
pub const MIN_OUTBOUND_SUBNET_DIVERSITY: usize = 3; // Require connections to at least 3 different /16 subnets

type Shared<T> = Arc<Mutex<T>>;
pub struct PeerManager {
    peers: Shared<HashMap<PeerId, UnboundedSender<P2pMessage>>>,
    peer_heights: Shared<HashMap<PeerId, u64>>,
    peer_handshakes: Shared<HashMap<PeerId, HandshakeInfo>>,
    peer_ips: Shared<HashMap<String, Vec<PeerId>>>, // IP -> list of peer IDs
    my_height: Arc<Mutex<u64>>,
    my_listening_port: Arc<Mutex<u16>>,
    /// callback when a new block is received
    on_block: Arc<Mutex<Option<Arc<dyn Fn(block::Block) + Send + Sync>>>>,
    /// callback when a new transaction is received
    on_tx: Arc<Mutex<Option<Arc<dyn Fn(Transaction) + Send + Sync>>>>,
    on_getheaders: Arc<
        Mutex<
            Option<
                Arc<dyn Fn(Vec<Vec<u8>>, Option<Vec<u8>>) -> Vec<block::BlockHeader> + Send + Sync>,
            >,
        >,
    >,
    on_getdata: Arc<Mutex<Option<Arc<dyn Fn(PeerId, InventoryType, Vec<Vec<u8>>) + Send + Sync>>>>,
}

impl PeerManager {
    pub fn new() -> Self {
        Self {
            peers: Arc::new(Mutex::new(HashMap::new())),
            peer_heights: Arc::new(Mutex::new(HashMap::new())),
            peer_handshakes: Arc::new(Mutex::new(HashMap::new())),
            peer_ips: Arc::new(Mutex::new(HashMap::new())),
            my_height: Arc::new(Mutex::new(0)),
            my_listening_port: Arc::new(Mutex::new(8335)), // Default port
            on_block: Arc::new(Mutex::new(None)),
            on_tx: Arc::new(Mutex::new(None)),
            on_getheaders: Arc::new(Mutex::new(None)),
            on_getdata: Arc::new(Mutex::new(None)),
        }
    }

    pub fn set_on_block<F>(&self, cb: F)
    where
        F: Fn(block::Block) + Send + Sync + 'static,
    {
        *self.on_block.lock() = Some(Arc::new(cb));
    }

    pub fn set_on_tx<F>(&self, cb: F)
    where
        F: Fn(Transaction) + Send + Sync + 'static,
    {
        *self.on_tx.lock() = Some(Arc::new(cb));
    }

    pub fn set_on_getheaders<F>(&self, cb: F)
    where
        F: Fn(Vec<Vec<u8>>, Option<Vec<u8>>) -> Vec<block::BlockHeader> + Send + Sync + 'static,
    {
        *self.on_getheaders.lock() = Some(Arc::new(cb));
    }

    pub fn set_on_getdata<F>(&self, cb: F)
    where
        F: Fn(PeerId, InventoryType, Vec<Vec<u8>>) + Send + Sync + 'static,
    {
        *self.on_getdata.lock() = Some(Arc::new(cb));
    }

    pub fn set_my_height(&self, height: u64) {
        *self.my_height.lock() = height;
    }

    pub fn get_my_height(&self) -> u64 {
        *self.my_height.lock()
    }

    pub fn set_my_listening_port(&self, port: u16) {
        *self.my_listening_port.lock() = port;
    }

    pub fn get_my_listening_port(&self) -> u16 {
        *self.my_listening_port.lock()
    }

    /// Get handshake info for a specific peer
    pub fn get_peer_handshake(&self, peer_id: &str) -> Option<HandshakeInfo> {
        self.peer_handshakes.lock().get(peer_id).cloned()
    }

    /// Get all peer handshake infos
    pub fn get_all_peer_handshakes(&self) -> HashMap<PeerId, HandshakeInfo> {
        self.peer_handshakes.lock().clone()
    }

    /// Security: Extract subnet prefixes from IP address for diversity checking
    fn get_subnet_prefixes(ip: &str) -> Option<(String, String)> {
        let parts: Vec<&str> = ip.split('.').collect();
        if parts.len() >= 3 {
            let subnet_24 = format!("{}.{}.{}", parts[0], parts[1], parts[2]);
            let subnet_16 = format!("{}.{}", parts[0], parts[1]);
            Some((subnet_24, subnet_16))
        } else {
            None
        }
    }

    /// Security: Check if adding a peer from this IP would violate subnet diversity rules
    /// Returns (allowed, reason) - protects against Eclipse attacks
    #[allow(dead_code)]
    fn check_subnet_diversity(&self, ip: &str) -> (bool, Option<String>) {
        let (subnet_24, subnet_16) = match Self::get_subnet_prefixes(ip) {
            Some(subnets) => subnets,
            None => return (true, None), // Can't parse, allow
        };

        // Count existing peers in same subnets
        info!("[P2P] 🔒 check_subnet_diversity: acquiring peer_ips lock...");
        let lock_start = std::time::Instant::now();
        let peer_ips = self.peer_ips.lock();
        let lock_duration = lock_start.elapsed();
        if lock_duration.as_micros() > 100 {
            info!(
                "[P2P] ✅ check_subnet_diversity: peer_ips lock acquired (took {:?})",
                lock_duration
            );
        }
        let mut subnet_24_count = 0;
        let mut subnet_16_count = 0;

        for existing_ip in peer_ips.keys() {
            if let Some((existing_24, existing_16)) = Self::get_subnet_prefixes(existing_ip) {
                if existing_24 == subnet_24 {
                    subnet_24_count += 1;
                }
                if existing_16 == subnet_16 {
                    subnet_16_count += 1;
                }
            }
        }

        // Check /24 subnet limit
        if subnet_24_count >= MAX_PEERS_PER_SUBNET_24 {
            return (
                false,
                Some(format!(
                    "Too many peers from subnet {}.0/24 ({} peers, max: {})",
                    subnet_24, subnet_24_count, MAX_PEERS_PER_SUBNET_24
                )),
            );
        }

        // Check /16 subnet limit
        if subnet_16_count >= MAX_PEERS_PER_SUBNET_16 {
            return (
                false,
                Some(format!(
                    "Too many peers from subnet {}.0.0/16 ({} peers, max: {})",
                    subnet_16, subnet_16_count, MAX_PEERS_PER_SUBNET_16
                )),
            );
        }

        (true, None)
    }

    /// Security: Get current subnet diversity metrics
    pub fn get_subnet_diversity_stats(&self) -> (usize, usize) {
        use std::collections::HashSet;

        info!("[P2P] 🔒 get_subnet_diversity_stats: acquiring peer_ips lock...");
        let lock_start = std::time::Instant::now();
        let peer_ips = self.peer_ips.lock();
        let lock_duration = lock_start.elapsed();
        if lock_duration.as_micros() > 100 {
            info!(
                "[P2P] ✅ get_subnet_diversity_stats: peer_ips lock acquired (took {:?})",
                lock_duration
            );
        }
        let mut subnet_24s = HashSet::new();
        let mut subnet_16s = HashSet::new();

        for ip in peer_ips.keys() {
            if let Some((subnet_24, subnet_16)) = Self::get_subnet_prefixes(ip) {
                subnet_24s.insert(subnet_24);
                subnet_16s.insert(subnet_16);
            }
        }

        (subnet_24s.len(), subnet_16s.len())
    }

    /// inbound connections accept loop (spawn)
    pub async fn start_listener(self: Arc<Self>, bind_addr: &str) -> anyhow::Result<()> {
        let listener = TcpListener::bind(bind_addr).await?;
        info!("P2P listener bound to {}", bind_addr);

        loop {
            let (socket, peer_addr) = listener.accept().await?;
            let peer_id = format!("{}", peer_addr);
            let manager_clone = self.clone();
            tokio::spawn(async move {
                if let Err(e) = manager_clone.handle_incoming(socket, peer_id).await {
                    warn!("Incoming peer handling error: {:?}", e);
                }
            });
        }
    }

    /// outbound connection to peer
    pub async fn connect_peer(self: Arc<Self>, addr: &str) -> anyhow::Result<()> {
        let stream = TcpStream::connect(addr).await?;
        let peer_id = addr.to_string();
        self.spawn_peer_loop(stream, peer_id).await?;
        Ok(())
    }

    async fn handle_incoming(
        self: Arc<Self>,
        stream: TcpStream,
        peer_id: PeerId,
    ) -> anyhow::Result<()> {
        // Security: Extract IP address and check connection limit
        let peer_ip = peer_id.split(':').next().unwrap_or("").to_string();

        info!(
            "[P2P] 🔒 handle_incoming {}: acquiring peer_ips lock for validation...",
            peer_id
        );
        let validation_start = std::time::Instant::now();

        // OPTIMIZATION: Lock peer_ips ONCE and perform all checks together
        let (peer_count, diversity_ok, diversity_reason, subnet_24_count, subnet_16_count) = {
            let peer_ips_guard = self.peer_ips.lock();
            let lock_duration = validation_start.elapsed();
            info!(
                "[P2P] ✅ handle_incoming {}: peer_ips lock acquired (took {:?})",
                peer_id, lock_duration
            );

            // 1. Check if this IP already has too many connections
            let peer_count = peer_ips_guard
                .get(&peer_ip)
                .map(|peers| peers.len())
                .unwrap_or(0);

            if peer_count >= MAX_PEERS_PER_IP {
                warn!(
                    "[WARN] Rejecting connection from {} - IP {} already has {} connections (max: {})",
                    peer_id, peer_ip, peer_count, MAX_PEERS_PER_IP
                );
                return Ok(()); // Silently drop connection
            }

            // 2. Check subnet diversity (inline to avoid second lock)
            use std::collections::HashSet;
            let (diversity_ok, diversity_reason) = match Self::get_subnet_prefixes(&peer_ip) {
                None => (true, None), // Can't parse, allow
                Some((subnet_24, subnet_16)) => {
                    let mut subnet_24_count = 0;
                    let mut subnet_16_count = 0;

                    for existing_ip in peer_ips_guard.keys() {
                        if let Some((existing_24, existing_16)) =
                            Self::get_subnet_prefixes(existing_ip)
                        {
                            if existing_24 == subnet_24 {
                                subnet_24_count += 1;
                            }
                            if existing_16 == subnet_16 {
                                subnet_16_count += 1;
                            }
                        }
                    }

                    // Check /24 subnet limit
                    if subnet_24_count >= MAX_PEERS_PER_SUBNET_24 {
                        (
                            false,
                            Some(format!(
                                "Too many peers from subnet {}.0/24 ({} peers, max: {})",
                                subnet_24, subnet_24_count, MAX_PEERS_PER_SUBNET_24
                            )),
                        )
                    }
                    // Check /16 subnet limit
                    else if subnet_16_count >= MAX_PEERS_PER_SUBNET_16 {
                        (
                            false,
                            Some(format!(
                                "Too many peers from subnet {}.0/16 ({} peers, max: {})",
                                subnet_16, subnet_16_count, MAX_PEERS_PER_SUBNET_16
                            )),
                        )
                    } else {
                        (true, None)
                    }
                }
            };

            // 3. Get overall subnet diversity stats (inline to avoid third lock)
            let mut subnet_24s = HashSet::new();
            let mut subnet_16s = HashSet::new();

            for ip in peer_ips_guard.keys() {
                if let Some((subnet_24, subnet_16)) = Self::get_subnet_prefixes(ip) {
                    subnet_24s.insert(subnet_24);
                    subnet_16s.insert(subnet_16);
                }
            }

            let total_validation = validation_start.elapsed();
            info!(
                "[P2P] ✅ handle_incoming {}: validation completed (total {:?})",
                peer_id, total_validation
            );

            (
                peer_count,
                diversity_ok,
                diversity_reason,
                subnet_24s.len(),
                subnet_16s.len(),
            )
        }; // peer_ips lock released here

        if !diversity_ok {
            warn!(
                "[WARN] Rejecting connection from {} - subnet diversity violation: {}",
                peer_id,
                diversity_reason.unwrap_or_else(|| "Unknown".to_string())
            );
            return Ok(()); // Silently drop connection
        }

        info!(
            "[INFO] Accepting connection from {} ({} existing from IP, diversity: {}/24 subnets, {}/16 subnets)",
            peer_id, peer_count, subnet_24_count, subnet_16_count
        );

        self.spawn_peer_loop(stream, peer_id).await?;
        Ok(())
    }

    /// spawn peer read/write loops
    pub async fn spawn_peer_loop(
        self: Arc<Self>,
        stream: TcpStream,
        peer_id: PeerId,
    ) -> anyhow::Result<()> {
        let (r, w) = tokio::io::split(stream);

        let reader = FramedRead::new(r, LengthDelimitedCodec::new());
        let writer = FramedWrite::new(w, LengthDelimitedCodec::new());

        let peer = Peer {
            id: peer_id.clone(),
            reader,
            writer,
            handshake_info: None,
        };

        let peer_id_clone = peer.id.clone();
        let peer_id_clone2 = peer.id.clone();
        let mut writer = peer.writer;
        let mut reader = peer.reader;

        // channel for sending outgoing messages to the write task
        let (tx, rx): (UnboundedSender<P2pMessage>, UnboundedReceiver<P2pMessage>) =
            mpsc::unbounded_channel();

        // register sender in the manager so other parts can send to this peer
        self.peers.lock().insert(peer_id_clone.clone(), tx.clone());

        // Security: Track IP address for connection limiting
        info!(
            "[P2P] 🔒 spawn_peer_loop {}: acquiring peer_ips lock to register...",
            peer_id_clone
        );
        let lock_start = std::time::Instant::now();
        let peer_ip = peer_id_clone.split(':').next().unwrap_or("").to_string();
        {
            let mut peer_ips_guard = self.peer_ips.lock();
            let lock_duration = lock_start.elapsed();
            if lock_duration.as_micros() > 100 {
                info!(
                    "[P2P] ✅ spawn_peer_loop {}: peer_ips lock acquired (took {:?})",
                    peer_id_clone, lock_duration
                );
            }
            peer_ips_guard
                .entry(peer_ip.clone())
                .or_insert_with(Vec::new)
                .push(peer_id_clone.clone());
        } // peer_ips lock released

        // drop local tx so the only remaining sender is the one in peers map
        drop(tx);

        info!("Registered peer {} from IP {}", peer_id_clone, peer_ip);

        // Send handshake immediately
        if let Some(tx) = self.peers.lock().get(&peer_id_clone) {
            let my_height = self.get_my_height();
            let my_port = self.get_my_listening_port();
            let handshake_info = HandshakeInfo {
                protocol_version: PROTOCOL_VERSION,
                software_version: env!("CARGO_PKG_VERSION").to_string(),
                supported_features: vec![
                    "blocks".to_string(),
                    "transactions".to_string(),
                    "headers".to_string(),
                ],
                network_id: resolve_network_id().to_string(),
                chain_id: resolve_chain_id(),
                height: my_height,
                listening_port: my_port,
            };
            let _ = tx.send(P2pMessage::Handshake {
                info: handshake_info,
            });
        }

        let config = bincode::config::standard();
        let config_read = bincode::config::standard();

        // writer task: consumes rx and writes framed bytes to the socket
        let write_handle = tokio::spawn(async move {
            let mut rx = rx;
            loop {
                match rx.recv().await {
                    Some(msg) => {
                        match bincode::encode_to_vec(&msg, config) {
                            Ok(vec) => {
                                // convert Vec<u8> -> Bytes (LengthDelimitedCodec accepts bytes)
                                let bytes: Bytes = Bytes::from(vec);
                                if let Err(e) = writer.send(bytes).await {
                                    log::warn!("write error to peer {}: {:?}", peer_id, e);
                                    break;
                                }
                            }
                            Err(e) => {
                                log::warn!("bincode encode error for {}: {:?}", peer_id, e);
                                break;
                            }
                        }
                    }
                    None => {
                        // All senders dropped -> normal shutdown of writer
                        log::info!("write rx closed for peer {}", peer_id);
                        break;
                    }
                }
            }

            // best-effort to close the sink
            let _ = writer.close().await;
        });

        // read task: read framed bytes, decode, and hand to manager
        let manager_clone = self.clone();
        let read_handle = tokio::spawn(async move {
            loop {
                match reader.next().await {
                    Some(Ok(bytes_mut)) => {
                        // bytes_mut is BytesMut; get slice for bincode
                        let slice = bytes_mut.as_ref();
                        match bincode::decode_from_slice::<P2pMessage, _>(slice, config_read) {
                            Ok((msg, _remaining)) => {
                                // delegate to manager
                                manager_clone
                                    .handle_message(peer_id_clone.clone(), msg)
                                    .await;
                            }
                            Err(e) => {
                                log::warn!("peer {} decode error: {:?}", peer_id_clone, e);
                                break;
                            }
                        }
                    }
                    Some(Err(e)) => {
                        log::warn!("peer {} read error: {:?}", peer_id_clone, e);
                        break;
                    }
                    None => {
                        // stream ended (peer disconnected)
                        log::info!("peer {} disconnected (reader ended)", peer_id_clone);
                        break;
                    }
                }
            }
        });

        let read_fut = read_handle;
        let write_fut = write_handle;

        tokio::pin!(read_fut);
        tokio::pin!(write_fut);

        match future::select(read_fut, write_fut).await {
            future::Either::Left((read_res, write_fut)) => {
                log::info!("read finished first for peer {}", peer_id_clone2);
                if let Err(e) = read_res {
                    log::warn!("read task error: {:?}", e);
                }
                self.peers.lock().remove(&peer_id_clone2);

                // Security: Remove from IP tracking (OPTIMIZED: single lock)
                info!(
                    "[P2P] 🔒 cleanup {}: acquiring peer_ips lock for removal...",
                    peer_id_clone2
                );
                let lock_start = std::time::Instant::now();
                {
                    let peer_ip = peer_id_clone2.split(':').next().unwrap_or("").to_string();
                    let mut peer_ips_guard = self.peer_ips.lock();
                    let lock_duration = lock_start.elapsed();
                    if lock_duration.as_micros() > 100 {
                        info!(
                            "[P2P] ✅ cleanup {}: peer_ips lock acquired (took {:?})",
                            peer_id_clone2, lock_duration
                        );
                    }

                    if let Some(peer_list) = peer_ips_guard.get_mut(&peer_ip) {
                        peer_list.retain(|id| id != &peer_id_clone2);
                        if peer_list.is_empty() {
                            peer_ips_guard.remove(&peer_ip);
                        }
                    }
                } // peer_ips lock released

                let _ = write_fut.await; // await the remaining writer
            }
            future::Either::Right((write_res, read_fut)) => {
                log::info!("write finished first for peer {}", peer_id_clone2);
                if let Err(e) = write_res {
                    log::warn!("write task error: {:?}", e);
                }
                self.peers.lock().remove(&peer_id_clone2);

                // Security: Remove from IP tracking (OPTIMIZED: single lock)
                info!(
                    "[P2P] 🔒 cleanup {}: acquiring peer_ips lock for removal...",
                    peer_id_clone2
                );
                let lock_start = std::time::Instant::now();
                {
                    let peer_ip = peer_id_clone2.split(':').next().unwrap_or("").to_string();
                    let mut peer_ips_guard = self.peer_ips.lock();
                    let lock_duration = lock_start.elapsed();
                    if lock_duration.as_micros() > 100 {
                        info!(
                            "[P2P] ✅ cleanup {}: peer_ips lock acquired (took {:?})",
                            peer_id_clone2, lock_duration
                        );
                    }

                    if let Some(peer_list) = peer_ips_guard.get_mut(&peer_ip) {
                        peer_list.retain(|id| id != &peer_id_clone2);
                        if peer_list.is_empty() {
                            peer_ips_guard.remove(&peer_ip);
                        }
                    }
                } // peer_ips lock released

                let _ = read_fut.await; // await the remaining reader
            }
        }

        Ok(())
    }

    async fn handle_message(&self, peer_id: PeerId, msg: P2pMessage) {
        use P2pMessage::*;
        match msg {
            Handshake { info } => {
                info!(
                    "Handshake from {}: protocol={}, version={}, network={}, chain={}, height={}, features={:?}",
                    peer_id,
                    info.protocol_version,
                    info.software_version,
                    info.network_id,
                    info.chain_id,
                    info.height,
                    info.supported_features
                );

                // Validate protocol compatibility
                if info.protocol_version != PROTOCOL_VERSION {
                    warn!(
                        "Peer {} has incompatible protocol version {}",
                        peer_id, info.protocol_version
                    );
                    // Could disconnect here
                }

                if info.network_id != resolve_network_id() {
                    warn!(
                        "Peer {} is on different network: {}",
                        peer_id, info.network_id
                    );
                    // Could disconnect here
                }

                if info.chain_id != resolve_chain_id() {
                    warn!("Peer {} has different chain_id: {}", peer_id, info.chain_id);
                    // Could disconnect here
                }

                // Check if this is ourselves (same listening port)
                let my_port = self.get_my_listening_port();
                if info.listening_port == my_port {
                    warn!(
                        "Detected self-connection to {} (same listening port: {}), disconnecting",
                        peer_id, my_port
                    );
                    // Remove from peers map to disconnect
                    self.peers.lock().remove(&peer_id);
                    return; // Exit handler
                }

                // Store peer info
                self.peer_heights
                    .lock()
                    .insert(peer_id.clone(), info.height);
                self.peer_handshakes
                    .lock()
                    .insert(peer_id.clone(), info.clone());

                // Send handshake ack with our info
                if let Some(tx) = self.peers.lock().get(&peer_id) {
                    let my_height = self.get_my_height();
                    let my_info = HandshakeInfo {
                        protocol_version: PROTOCOL_VERSION,
                        software_version: env!("CARGO_PKG_VERSION").to_string(),
                        supported_features: vec![
                            "blocks".to_string(),
                            "transactions".to_string(),
                            "headers".to_string(),
                        ],
                        network_id: resolve_network_id().to_string(),
                        chain_id: resolve_chain_id(),
                        height: my_height,
                        listening_port: my_port,
                    };
                    let _ = tx.send(HandshakeAck { info: my_info });
                }

                // Start syncing headers
                if let Some(tx) = self.peers.lock().get(&peer_id) {
                    let locator = vec![];
                    let _ = tx.send(GetHeaders {
                        locator_hashes: locator,
                        stop_hash: None,
                    });
                }
            }

            HandshakeAck { info } => {
                info!(
                    "HandshakeAck from {}: protocol={}, version={}, network={}, chain={}, height={}",
                    peer_id,
                    info.protocol_version,
                    info.software_version,
                    info.network_id,
                    info.chain_id,
                    info.height
                );

                // Check if this is ourselves (same listening port)
                let my_port = self.get_my_listening_port();
                if info.listening_port == my_port {
                    warn!(
                        "Detected self-connection in HandshakeAck from {} (same listening port: {}), disconnecting",
                        peer_id, my_port
                    );
                    // Remove from peers map to disconnect
                    self.peers.lock().remove(&peer_id);
                    return; // Exit handler
                }

                // Store peer info
                let lock_start = std::time::Instant::now();
                self.peer_heights
                    .lock()
                    .insert(peer_id.clone(), info.height);
                let heights_duration = lock_start.elapsed();

                let lock_start = std::time::Instant::now();
                self.peer_handshakes.lock().insert(peer_id.clone(), info);
                let handshakes_duration = lock_start.elapsed();

                if heights_duration.as_micros() > 100 || handshakes_duration.as_micros() > 100 {
                    info!(
                        "[P2P] 🔒 HandshakeAck: peer_heights lock {:?}, peer_handshakes lock {:?}",
                        heights_duration, handshakes_duration
                    );
                }
            }

            Version { version, height } => {
                info!("{} sent version v{} height {}", peer_id, version, height);
                let lock_start = std::time::Instant::now();
                self.peer_heights.lock().insert(peer_id.clone(), height);
                if lock_start.elapsed().as_micros() > 100 {
                    info!(
                        "[P2P] 🔒 Version: peer_heights lock took {:?}",
                        lock_start.elapsed()
                    );
                }

                if let Some(tx) = self.peers.lock().get(&peer_id) {
                    let _ = tx.send(VerAck);
                }

                if let Some(tx) = self.peers.lock().get(&peer_id) {
                    let locator = vec![];
                    let _ = tx.send(GetHeaders {
                        locator_hashes: locator,
                        stop_hash: None,
                    });
                }
            }

            VerAck => {
                info!("{} verack", peer_id);
            }

            GetHeaders {
                locator_hashes,
                stop_hash,
            } => {
                info!(
                    "{} requested headers ({} locator hashes)",
                    peer_id,
                    locator_hashes.len()
                );
                let headers = match &*self.on_getheaders.lock() {
                    Some(cb) => (cb)(locator_hashes, stop_hash),
                    None => Vec::new(),
                };
                if let Some(tx) = self.peers.lock().get(&peer_id) {
                    let _ = tx.send(P2pMessage::Headers { headers });
                }
            }

            Headers { headers } => {
                info!("{} sent {} headers", peer_id, headers.len());
                if !headers.is_empty() {
                    // request full blocks for these headers
                    let mut hashes: Vec<Vec<u8>> = Vec::new();
                    for hdr in headers.iter() {
                        if let Ok(hash_hex) = block::compute_header_hash(hdr) {
                            if let Ok(bytes) = hex::decode(hash_hex) {
                                hashes.push(bytes);
                            }
                        }
                    }
                    if let Some(tx) = self.peers.lock().get(&peer_id) {
                        let _ = tx.send(P2pMessage::GetData {
                            object_type: InventoryType::Block,
                            hashes,
                        });
                    }
                }
            }

            Inv {
                object_type,
                hashes,
            } => {
                // Security: Validate INV message size to prevent memory exhaustion
                if hashes.len() > MAX_INV_PER_MESSAGE {
                    warn!(
                        "Peer {} sent excessive INV message: {} items (max: {}), ignoring",
                        peer_id,
                        hashes.len(),
                        MAX_INV_PER_MESSAGE
                    );
                    return; // Drop the message
                }

                info!("{} inv {} items", peer_id, hashes.len());
                if let Some(tx) = self.peers.lock().get(&peer_id) {
                    let _ = tx.send(GetData {
                        object_type,
                        hashes,
                    });
                }
            }

            GetData {
                object_type,
                hashes,
            } => {
                // Security: Validate GetData message size
                if hashes.len() > MAX_INV_PER_MESSAGE {
                    warn!(
                        "Peer {} sent excessive GetData: {} items (max: {}), ignoring",
                        peer_id,
                        hashes.len(),
                        MAX_INV_PER_MESSAGE
                    );
                    return; // Drop the message
                }

                info!("{} requested {} items", peer_id, hashes.len());
                if let Some(cb) = &*self.on_getdata.lock() {
                    (cb)(peer_id.clone(), object_type, hashes);
                }
            }

            Block { block } => {
                info!(
                    "[P2P] 📦 {} sent block #{} {}",
                    peer_id, block.header.index, block.hash
                );
                let callback_start = std::time::Instant::now();
                let lock_start = std::time::Instant::now();
                let cb = self.on_block.lock().clone();
                let lock_duration = lock_start.elapsed();

                if let Some(cb) = cb {
                    if lock_duration.as_micros() > 100 {
                        info!(
                            "[P2P] 🔒 Block callback: on_block lock took {:?}",
                            lock_duration
                        );
                    }
                    (cb)(block.clone());
                    info!(
                        "[P2P] ✅ Block callback completed in {:?}",
                        callback_start.elapsed()
                    );
                }
            }

            Tx { tx } => {
                info!(
                    "[P2P] 💸 {} sent transaction {}",
                    peer_id,
                    hex::encode(&tx.txid[..8])
                );
                let callback_start = std::time::Instant::now();
                let lock_start = std::time::Instant::now();
                let cb = self.on_tx.lock().clone();
                let lock_duration = lock_start.elapsed();

                if let Some(cb) = cb {
                    if lock_duration.as_micros() > 100 {
                        info!("[P2P] 🔒 TX callback: on_tx lock took {:?}", lock_duration);
                    }
                    (cb)(tx.clone());
                    let total_duration = callback_start.elapsed();
                    if total_duration.as_millis() > 1 {
                        info!("[P2P] ✅ TX callback completed in {:?}", total_duration);
                    }
                }
            }

            _ => {
                info!("{} sent {:?}", peer_id, msg);
            }
        }
    }

    pub fn broadcast_inv(&self, object_type: InventoryType, hashes: Vec<Vec<u8>>) {
        info!("[P2P] 🔒 broadcast_inv: acquiring peers lock...");
        let lock_start = std::time::Instant::now();
        let peers = self.peers.lock().clone();
        let lock_duration = lock_start.elapsed();
        info!(
            "[P2P] ✅ broadcast_inv: peers lock acquired (took {:?}), {} peers",
            lock_duration,
            peers.len()
        );

        for (_id, tx) in peers {
            let _ = tx.send(P2pMessage::Inv {
                object_type: object_type.clone(),
                hashes: hashes.clone(),
            });
        }
        info!(
            "[P2P] ✅ broadcast_inv: completed (total {:?})",
            lock_start.elapsed()
        );
    }

    pub fn send_to_peer(&self, peer_id: &PeerId, msg: P2pMessage) {
        let lock_start = std::time::Instant::now();
        if let Some(tx) = self.peers.lock().get(peer_id) {
            let lock_duration = lock_start.elapsed();
            if lock_duration.as_micros() > 100 {
                info!("[P2P] 🔒 send_to_peer: lock took {:?}", lock_duration);
            }
            let _ = tx.send(msg);
        }
    }

    pub async fn send_block_to_peer(&self, peer_id: &PeerId, block: &block::Block) {
        self.send_to_peer(
            peer_id,
            P2pMessage::Block {
                block: block.clone(),
            },
        );
    }

    pub fn load_saved_peers(&self) -> Vec<SavedPeer> {
        if let Ok(data) = std::fs::read_to_string(PEERS_FILE) {
            if let Ok(peers) = serde_json::from_str::<Vec<SavedPeer>>(&data) {
                return peers;
            }
        }
        Vec::new()
    }

    pub fn save_saved_peers(&self, peers: &[SavedPeer]) {
        if let Ok(json) = serde_json::to_string_pretty(peers) {
            let _ = fs::write(PEERS_FILE, json);
        }
    }

    pub async fn dns_seed_lookup(&self) -> anyhow::Result<Vec<String>> {
        let _seeds = vec![
            "seed1.Astram.org:19533",
            "seed2.Astram.org:19533",
            "dnsseed.Astram.io:19533",
        ];

        let peers = Vec::new();
        /*
                /// TODO : we need domain lookup in parallel
                for seed in seeds {
                    match lookup_host(seed).await {
                        Ok(addrs) => {
                            for a in addrs {
                                peers.push(a.to_string());
                            }
                        }
                        Err(e) => warn!("DNS seed {} lookup failed: {:?}", seed, e),
                    }
                }
        */
        Ok(peers)
    }

    /// Broadcast a block to all connected peers (fire-and-forget)
    pub async fn broadcast_block(&self, block: &block::Block) {
        info!(
            "[P2P] 🔒 broadcast_block #{}: acquiring peers lock...",
            block.header.index
        );
        let lock_start = std::time::Instant::now();
        let peers = self.peers.lock().clone();
        let lock_duration = lock_start.elapsed();
        info!(
            "[P2P] ✅ broadcast_block #{}: peers lock acquired (took {:?}), {} peers",
            block.header.index,
            lock_duration,
            peers.len()
        );

        for (_id, tx) in peers {
            // clone the block for each peer
            let _ = tx.send(P2pMessage::Block {
                block: block.clone(),
            });
        }
        info!(
            "[P2P] ✅ broadcast_block #{}: completed (total {:?})",
            block.header.index,
            lock_start.elapsed()
        );
    }

    /// Broadcast a transaction to all connected peers (async so callers can `.await`)
    pub async fn broadcast_tx(&self, tx_obj: &Transaction) {
        info!(
            "[P2P] 🔒 broadcast_tx {}: acquiring peers lock...",
            hex::encode(&tx_obj.txid[..8])
        );
        let lock_start = std::time::Instant::now();
        let peers = self.peers.lock().clone();
        let lock_duration = lock_start.elapsed();
        info!(
            "[P2P] ✅ broadcast_tx: peers lock acquired (took {:?}), {} peers",
            lock_duration,
            peers.len()
        );

        for (_id, tx) in peers {
            // clone the transaction for each peer
            let _ = tx.send(P2pMessage::Tx { tx: tx_obj.clone() });
        }
        info!(
            "[P2P] ✅ broadcast_tx: completed (total {:?})",
            lock_start.elapsed()
        );
    }

    /// Request headers from all connected peers using a GetHeaders message.
    /// `locator_hashes` and `stop_hash` are sent as-is to peers (best-effort).
    pub fn request_headers_from_peers(
        &self,
        locator_hashes: Vec<Vec<u8>>,
        stop_hash: Option<Vec<u8>>,
    ) {
        info!("[P2P] 🔒 request_headers_from_peers: acquiring peers lock...");
        let lock_start = std::time::Instant::now();
        let peers = self.peers.lock().clone();
        let lock_duration = lock_start.elapsed();
        info!(
            "[P2P] ✅ request_headers_from_peers: peers lock acquired (took {:?}), {} peers",
            lock_duration,
            peers.len()
        );

        for (_id, tx) in peers {
            let _ = tx.send(P2pMessage::GetHeaders {
                locator_hashes: locator_hashes.clone(),
                stop_hash: stop_hash.clone(),
            });
        }
    }

    pub fn get_peer_heights(&self) -> HashMap<PeerId, u64> {
        self.peer_heights.lock().clone()
    }

    /// Non-blocking snapshot for status endpoints. Returns None if any lock is contended.
    pub fn try_get_status_snapshot(&self) -> Option<(HashMap<PeerId, u64>, u64, usize, usize)> {
        use std::collections::HashSet;

        let peer_heights = match self.peer_heights.try_lock() {
            Some(guard) => {
                let cloned = guard.clone();
                drop(guard);
                cloned
            }
            None => {
                warn!("[P2P] ⚠️ try_get_status_snapshot: peer_heights lock CONTENDED");
                return None;
            }
        };

        let my_height = match self.my_height.try_lock() {
            Some(guard) => *guard,
            None => {
                warn!("[P2P] ⚠️ try_get_status_snapshot: my_height lock CONTENDED");
                return None;
            }
        };

        let peer_ips = match self.peer_ips.try_lock() {
            Some(guard) => guard,
            None => {
                warn!("[P2P] ⚠️ try_get_status_snapshot: peer_ips lock CONTENDED");
                return None;
            }
        };

        let mut subnet_24s = HashSet::new();
        let mut subnet_16s = HashSet::new();

        for ip in peer_ips.keys() {
            if let Some((subnet_24, subnet_16)) = Self::get_subnet_prefixes(ip) {
                subnet_24s.insert(subnet_24);
                subnet_16s.insert(subnet_16);
            }
        }

        Some((peer_heights, my_height, subnet_24s.len(), subnet_16s.len()))
    }

    /// Register this node with a DNS server
    /// The DNS server will automatically detect the IP address from the connection
    pub async fn register_with_dns(&self, dns_server: &str, my_port: u16) -> anyhow::Result<()> {
        let client = reqwest::Client::new();
        let my_height = self.get_my_height();
        let version = env!("CARGO_PKG_VERSION").to_string();

        let request = DnsRegisterRequest {
            address: None, // DNS server will detect the IP from the connection
            port: my_port,
            version,
            height: my_height,
        };

        let response = client
            .post(format!("{}/register", dns_server))
            .json(&request)
            .send()
            .await?;

        if response.status().is_success() {
            let resp: DnsRegisterResponse = response.json().await?;
            info!(
                "Successfully registered with DNS server: {} (total nodes: {})",
                resp.message, resp._node_count
            );
        } else {
            warn!("Failed to register with DNS server: {}", response.status());
        }

        Ok(())
    }

    /// Fetch peer nodes from DNS server
    pub async fn fetch_peers_from_dns(
        &self,
        dns_server: &str,
        limit: Option<usize>,
        min_height: Option<u64>,
    ) -> anyhow::Result<Vec<String>> {
        let client = reqwest::Client::new();
        let mut url = format!("{}/nodes", dns_server);

        let mut params = Vec::new();
        if let Some(l) = limit {
            params.push(format!("limit={}", l));
        }
        if let Some(h) = min_height {
            params.push(format!("min_height={}", h));
        }

        if !params.is_empty() {
            url = format!("{}?{}", url, params.join("&"));
        }

        let response = client.get(&url).send().await?;

        if response.status().is_success() {
            let resp: DnsNodesResponse = response.json().await?;
            info!("Fetched {} peer nodes from DNS server", resp.count);

            let peer_addrs: Vec<String> = resp
                .nodes
                .iter()
                .map(|n| format!("{}:{}", n.address, n.port))
                .collect();

            Ok(peer_addrs)
        } else {
            warn!(
                "Failed to fetch peers from DNS server: {}",
                response.status()
            );
            Ok(Vec::new())
        }
    }

    /// Start periodic DNS registration (call this in a background task)
    /// The DNS server will automatically detect the node's IP address from the connection
    pub async fn start_dns_registration_loop(
        self: Arc<Self>,
        dns_server: String,
        my_port: u16,
        interval_secs: u64,
    ) {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(interval_secs));

        loop {
            interval.tick().await;

            if let Err(e) = self.register_with_dns(&dns_server, my_port).await {
                warn!("DNS registration failed: {:?}", e);
            }
        }
    }
}

#[derive(Serialize)]
struct DnsRegisterRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    address: Option<String>,
    port: u16,
    version: String,
    height: u64,
}

#[derive(Deserialize)]
struct DnsRegisterResponse {
    #[serde(rename = "success")]
    _success: bool,
    message: String,
    #[serde(rename = "node_count")]
    _node_count: usize,
}

#[derive(Deserialize)]
struct DnsNodeInfo {
    address: String,
    port: u16,
    #[serde(rename = "version")]
    _version: String,
    #[serde(rename = "height")]
    _height: u64,
    #[serde(rename = "last_seen")]
    _last_seen: i64,
}

#[derive(Deserialize)]
struct DnsNodesResponse {
    nodes: Vec<DnsNodeInfo>,
    count: usize,
}
