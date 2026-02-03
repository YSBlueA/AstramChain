use crate::p2p::messages::{HandshakeInfo, InventoryType, P2pMessage};
use crate::p2p::peer::{Peer, PeerId};
use bincode::{Decode, Encode};
use bytes::Bytes;
use futures::SinkExt;
use futures::StreamExt;
use futures::future;
use hex;
use log::{info, warn};
use netcoin_core::block;
use netcoin_core::transaction::Transaction;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::sync::Arc;
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
pub const NETWORK_ID: &str = "netcoin-mainnet";
pub const CHAIN_ID: u64 = 1;

type Shared<T> = Arc<Mutex<T>>;
pub struct PeerManager {
    peers: Shared<HashMap<PeerId, UnboundedSender<P2pMessage>>>,
    peer_heights: Shared<HashMap<PeerId, u64>>,
    peer_handshakes: Shared<HashMap<PeerId, HandshakeInfo>>,
    my_height: Arc<Mutex<u64>>,
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
    on_getdata: Arc<
        Mutex<
            Option<
                Arc<dyn Fn(PeerId, InventoryType, Vec<Vec<u8>>) + Send + Sync>,
            >,
        >,
    >,
}

impl PeerManager {
    pub fn new() -> Self {
        Self {
            peers: Arc::new(Mutex::new(HashMap::new())),
            peer_heights: Arc::new(Mutex::new(HashMap::new())),
            peer_handshakes: Arc::new(Mutex::new(HashMap::new())),
            my_height: Arc::new(Mutex::new(0)),
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

    /// Get handshake info for a specific peer
    pub fn get_peer_handshake(&self, peer_id: &str) -> Option<HandshakeInfo> {
        self.peer_handshakes.lock().get(peer_id).cloned()
    }

    /// Get all peer handshake infos
    pub fn get_all_peer_handshakes(&self) -> HashMap<PeerId, HandshakeInfo> {
        self.peer_handshakes.lock().clone()
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

        // drop local tx so the only remaining sender is the one in peers map
        drop(tx);

        info!("Registered peer {}", peer_id_clone);

        // Send handshake immediately
        if let Some(tx) = self.peers.lock().get(&peer_id_clone) {
            let my_height = self.get_my_height();
            let handshake_info = HandshakeInfo {
                protocol_version: PROTOCOL_VERSION,
                software_version: env!("CARGO_PKG_VERSION").to_string(),
                supported_features: vec![
                    "blocks".to_string(),
                    "transactions".to_string(),
                    "headers".to_string(),
                ],
                network_id: NETWORK_ID.to_string(),
                chain_id: CHAIN_ID,
                height: my_height,
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
                let _ = write_fut.await; // await the remaining writer
            }
            future::Either::Right((write_res, read_fut)) => {
                log::info!("write finished first for peer {}", peer_id_clone2);
                if let Err(e) = write_res {
                    log::warn!("write task error: {:?}", e);
                }
                self.peers.lock().remove(&peer_id_clone2);
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

                if info.network_id != NETWORK_ID {
                    warn!(
                        "Peer {} is on different network: {}",
                        peer_id, info.network_id
                    );
                    // Could disconnect here
                }

                if info.chain_id != CHAIN_ID {
                    warn!("Peer {} has different chain_id: {}", peer_id, info.chain_id);
                    // Could disconnect here
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
                        network_id: NETWORK_ID.to_string(),
                        chain_id: CHAIN_ID,
                        height: my_height,
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

                // Store peer info
                self.peer_heights
                    .lock()
                    .insert(peer_id.clone(), info.height);
                self.peer_handshakes.lock().insert(peer_id.clone(), info);
            }

            Version { version, height } => {
                info!("{} sent version v{} height {}", peer_id, version, height);
                self.peer_heights.lock().insert(peer_id.clone(), height);

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
                info!("{} requested {} items", peer_id, hashes.len());
                if let Some(cb) = &*self.on_getdata.lock() {
                    (cb)(peer_id.clone(), object_type, hashes);
                }
            }

            Block { block } => {
                info!("{} sent block {}", peer_id, block.hash);
                if let Some(cb) = &*self.on_block.lock() {
                    (cb)(block);
                }
            }

            Tx { tx } => {
                info!("{} sent transaction {}", peer_id, tx.txid);
                if let Some(cb) = &*self.on_tx.lock() {
                    (cb)(tx);
                }
            }

            _ => {
                info!("{} sent {:?}", peer_id, msg);
            }
        }
    }

    pub fn broadcast_inv(&self, object_type: InventoryType, hashes: Vec<Vec<u8>>) {
        let peers = self.peers.lock().clone();
        for (_id, tx) in peers {
            let _ = tx.send(P2pMessage::Inv {
                object_type: object_type.clone(),
                hashes: hashes.clone(),
            });
        }
    }

    pub fn send_to_peer(&self, peer_id: &PeerId, msg: P2pMessage) {
        if let Some(tx) = self.peers.lock().get(peer_id) {
            let _ = tx.send(msg);
        }
    }

    pub async fn send_block_to_peer(&self, peer_id: &PeerId, block: &block::Block) {
        self.send_to_peer(peer_id, P2pMessage::Block {
            block: block.clone(),
        });
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
        use tokio::net::lookup_host;
        let seeds = vec![
            "seed1.netcoin.org:8333",
            "seed2.netcoin.org:8333",
            "dnsseed.netcoin.io:8333",
        ];

        let mut peers = Vec::new();
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
        let peers = self.peers.lock().clone();
        for (_id, tx) in peers {
            // clone the block for each peer
            let _ = tx.send(P2pMessage::Block {
                block: block.clone(),
            });
        }
    }

    /// Broadcast a transaction to all connected peers (async so callers can `.await`)
    pub async fn broadcast_tx(&self, tx_obj: &Transaction) {
        let peers = self.peers.lock().clone();
        for (_id, tx) in peers {
            // clone the transaction for each peer
            let _ = tx.send(P2pMessage::Tx { tx: tx_obj.clone() });
        }
    }

    /// Request headers from all connected peers using a GetHeaders message.
    /// `locator_hashes` and `stop_hash` are sent as-is to peers (best-effort).
    pub fn request_headers_from_peers(
        &self,
        locator_hashes: Vec<Vec<u8>>,
        stop_hash: Option<Vec<u8>>,
    ) {
        let peers = self.peers.lock().clone();
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

    /// Register this node with a DNS server
    pub async fn register_with_dns(
        &self,
        dns_server: &str,
        my_address: &str,
        my_port: u16,
    ) -> anyhow::Result<()> {
        let client = reqwest::Client::new();
        let my_height = self.get_my_height();
        let version = env!("CARGO_PKG_VERSION").to_string();

        let request = DnsRegisterRequest {
            address: my_address.to_string(),
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
                resp.message, resp.node_count
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
    pub async fn start_dns_registration_loop(
        self: Arc<Self>,
        dns_server: String,
        my_address: String,
        my_port: u16,
        interval_secs: u64,
    ) {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(interval_secs));

        loop {
            interval.tick().await;

            if let Err(e) = self
                .register_with_dns(&dns_server, &my_address, my_port)
                .await
            {
                warn!("DNS registration failed: {:?}", e);
            }
        }
    }
}

#[derive(Serialize)]
struct DnsRegisterRequest {
    address: String,
    port: u16,
    version: String,
    height: u64,
}

#[derive(Deserialize)]
struct DnsRegisterResponse {
    success: bool,
    message: String,
    node_count: usize,
}

#[derive(Deserialize)]
struct DnsNodeInfo {
    address: String,
    port: u16,
    version: String,
    height: u64,
    last_seen: i64,
}

#[derive(Deserialize)]
struct DnsNodesResponse {
    nodes: Vec<DnsNodeInfo>,
    count: usize,
}
