use crate::p2p::messages::{InventoryType, P2pMessage};
use crate::p2p::peer::{Peer, PeerId};
use bincode::{Decode, Encode};
use bytes::Bytes;
use futures::SinkExt;
use futures::StreamExt;
use futures::future;
use log::{info, warn};
use netcoin_core::block;
use parking_lot::Mutex;
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

type Shared<T> = Arc<Mutex<T>>;
pub struct PeerManager {
    peers: Shared<HashMap<PeerId, UnboundedSender<P2pMessage>>>,
    /// callback when a new block is received
    on_block: Option<Arc<dyn Fn(block::Block) + Send + Sync>>,
}

impl PeerManager {
    pub fn new() -> Self {
        Self {
            peers: Arc::new(Mutex::new(HashMap::new())),
            on_block: None,
        }
    }

    pub fn set_on_block<F>(&mut self, cb: F)
    where
        F: Fn(block::Block) + Send + Sync + 'static,
    {
        self.on_block = Some(Arc::new(cb));
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
            Version { version, height } => {
                info!("{} sent version v{} height {}", peer_id, version, height);

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

            Headers { headers } => {
                info!("{} sent {} headers", peer_id, headers.len());
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
            }

            Block { block } => {
                info!("{} sent block {}", peer_id, block.hash);
                if let Some(cb) = &self.on_block {
                    (cb)(block);
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
}
