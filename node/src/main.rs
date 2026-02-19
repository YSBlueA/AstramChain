// Use library exports instead of declaring local modules to avoid duplicate crate types
use Astram_core::Blockchain;
use Astram_core::block::Block;
use Astram_core::config::initial_block_reward;
use Astram_core::consensus;
use Astram_core::transaction::BINCODE_CONFIG;
use Astram_core::utxo::Utxo;
use astram_config::config::Config;
use astram_node::ChainState;
use astram_node::MempoolState;
use astram_node::MiningState;
use astram_node::NodeHandle;
use astram_node::NodeHandles;
use astram_node::NodeMeta;
use astram_node::p2p::service::P2PService;
use astram_node::server::run_server;
use hex;
use log::{info, warn};
use primitive_types::U256;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering as OtherOrdering;
use std::sync::{Arc, Mutex};
use tokio::signal;
use tokio::time::{Duration, sleep};

#[derive(Debug, Clone, Deserialize)]
struct DnsNodeInfo {
    address: String,
    port: u16,
    #[serde(rename = "version")]
    _version: String,
    height: u64,
    #[serde(rename = "last_seen")]
    _last_seen: i64,
    #[serde(rename = "first_seen")]
    _first_seen: i64,
    uptime_hours: f64,
}

#[derive(Debug, Deserialize)]
struct DnsNodesResponse {
    nodes: Vec<DnsNodeInfo>,
    count: usize,
}

#[derive(Debug, Clone)]
struct NodeSettings {
    data_dir: String,
    p2p_bind_addr: String,
    p2p_port: u16,
    http_bind_addr: String,
    http_port: u16,
    eth_rpc_bind_addr: String,
    eth_rpc_port: u16,
    dns_server_url: String,
}

impl Default for NodeSettings {
    fn default() -> Self {
        Self {
            data_dir: default_data_dir(),
            p2p_bind_addr: "0.0.0.0".to_string(),
            p2p_port: 8335,
            http_bind_addr: "127.0.0.1".to_string(),
            http_port: 19533,
            eth_rpc_bind_addr: "127.0.0.1".to_string(),
            eth_rpc_port: 8545,
            dns_server_url: "http://161.33.19.183:8053".to_string(),
        }
    }
}

fn default_data_dir() -> String {
    let home = dirs::home_dir().expect("Cannot find home directory");

    if cfg!(target_os = "windows") {
        let base = dirs::data_dir().unwrap_or(home).join("Astram");
        return base.join("data").to_string_lossy().into_owned();
    }

    home.join(".Astram")
        .join("data")
        .to_string_lossy()
        .into_owned()
}

fn expand_path_value(value: &str) -> String {
    let expanded = shellexpand::tilde(value).into_owned();
    if expanded.contains("%USERPROFILE%") {
        if let Ok(profile) = std::env::var("USERPROFILE") {
            return expanded.replace("%USERPROFILE%", &profile);
        }
    }
    expanded
}

fn resolve_node_settings_path() -> PathBuf {
    let exe_path = std::env::current_exe().ok().and_then(|path| {
        path.parent()
            .map(|parent| parent.join("config/nodeSettings.conf"))
    });

    if let Some(ref path) = exe_path {
        if path.exists() {
            return path.clone();
        }
    }

    let cwd_path = PathBuf::from("config/nodeSettings.conf");
    if cwd_path.exists() {
        return cwd_path;
    }

    exe_path.unwrap_or(cwd_path)
}

fn load_node_settings() -> NodeSettings {
    let mut settings = NodeSettings::default();
    let path = resolve_node_settings_path();

    match fs::read_to_string(&path) {
        Ok(contents) => {
            for (line_no, raw_line) in contents.lines().enumerate() {
                let line = raw_line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }

                let (key, value) = match line.split_once('=') {
                    Some(pair) => pair,
                    None => {
                        println!(
                            "[WARN] Invalid node setting on line {}: {}",
                            line_no + 1,
                            raw_line
                        );
                        continue;
                    }
                };

                let key = key.trim();
                let value = value.trim();
                match key {
                    "DATA_DIR" => settings.data_dir = expand_path_value(value),
                    "P2P_BIND_ADDR" => settings.p2p_bind_addr = value.to_string(),
                    "P2P_PORT" => settings.p2p_port = value.parse().unwrap_or(settings.p2p_port),
                    "HTTP_BIND_ADDR" => settings.http_bind_addr = value.to_string(),
                    "HTTP_PORT" => settings.http_port = value.parse().unwrap_or(settings.http_port),
                    "ETH_RPC_BIND_ADDR" => settings.eth_rpc_bind_addr = value.to_string(),
                    "ETH_RPC_PORT" => {
                        settings.eth_rpc_port = value.parse().unwrap_or(settings.eth_rpc_port)
                    }
                    "DNS_SERVER_URL" => settings.dns_server_url = value.to_string(),
                    _ => println!("[WARN] Unknown node setting key: {}", key),
                }
            }
        }
        Err(err) => {
            println!("[WARN] Node settings file not found at {:?}: {}", path, err);
        }
    }

    settings.data_dir = expand_path_value(&settings.data_dir);
    settings
}

fn to_socket_addr(addr: &str, port: u16, fallback: SocketAddr) -> SocketAddr {
    format!("{}:{}", addr, port).parse().unwrap_or(fallback)
}

#[tokio::main]
async fn main() {
    println!("[INFO] Astram node starting...");

    env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Debug)
        .init();

    let cfg = Config::load();
    let node_settings = Arc::new(load_node_settings());

    // Read wallet address from file (expand paths configured via CLI)
    let wallet_path = cfg.wallet_path_resolved();
    let wallet_file =
        fs::read_to_string(wallet_path.as_path()).expect("Failed to read wallet file");
    let wallet: Value = serde_json::from_str(&wallet_file).expect("Failed to parse wallet JSON");
    let miner_address = wallet["address"]
        .as_str()
        .expect("Failed to get address from wallet")
        .to_string();

    // DB path for core blockchain
    let db_path = node_settings.data_dir.clone();
    if let Err(err) = fs::create_dir_all(&db_path) {
        eprintln!(
            "[ERROR] Failed to create data directory {}: {}",
            db_path, err
        );
        std::process::exit(1);
    }

    print!("Initialize Block chain...\n");

    // Check for stale LOCK file and remove it if necessary
    let lock_path = std::path::Path::new(&db_path).join("LOCK");
    if lock_path.exists() {
        println!("[WARN] Found existing LOCK file, attempting to clean up...");

        // Try to remove stale lock file
        match fs::remove_file(&lock_path) {
            Ok(_) => {
                println!("[INFO] Removed stale LOCK file");
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
            Err(e) => {
                eprintln!("[ERROR] Failed to remove LOCK file: {}", e);
                eprintln!("Another instance may be running. Please stop it first.");
                std::process::exit(1);
            }
        }
    }

    // Initialize core Blockchain (RocksDB-backed)
    let bc = match Blockchain::new(db_path.as_str()) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("Failed to open blockchain DB: {}", e);
            eprintln!("If another instance is running, please stop it first.");
            std::process::exit(1);
        }
    };
    let bc = Arc::new(Mutex::new(bc));

    // Initialize P2P networking
    let p2p_service = P2PService::new();

    let mining_state = Arc::new(MiningState::default());

    let p2p_handle = p2p_service.manager();

    let chain_state = Arc::new(Mutex::new(ChainState::default()));
    let node_meta = Arc::new(NodeMeta {
        miner_address: Arc::new(Mutex::new(miner_address.clone())),
        my_public_address: Arc::new(Mutex::new(None)),
        node_start_time: std::time::Instant::now(),
        eth_to_astram_tx: Arc::new(Mutex::new(HashMap::new())),
    });

    let node = NodeHandles {
        bc: bc.clone(),
        mempool: Arc::new(Mutex::new(MempoolState::default())),
        mining: mining_state.clone(),
    };

    let node_handle = Arc::new(node);

    // Set current blockchain height in P2P manager
    let my_height = {
        let bc = node_handle.bc.lock().unwrap();
        if let Some(tip_hash) = &bc.chain_tip {
            if let Ok(Some(header)) = bc.load_header(tip_hash) {
                header.index + 1
            } else {
                0
            }
        } else {
            0
        }
    };
    p2p_handle.set_my_height(my_height);
    info!("[INFO] Local blockchain height set to: {}", my_height);

    let bind_addr = format!("{}:{}", node_settings.p2p_bind_addr, node_settings.p2p_port);

    // Set listening port in P2P manager (for self-connection detection)
    p2p_handle.set_my_listening_port(node_settings.p2p_port);

    p2p_service
        .start(bind_addr, node_handle.clone(), chain_state.clone())
        .await
        .expect("p2p start failed");

    // Start Ethereum JSON-RPC server for MetaMask
    let eth_rpc_node = node_handle.clone();
    let eth_rpc_addr = to_socket_addr(
        &node_settings.eth_rpc_bind_addr,
        node_settings.eth_rpc_port,
        SocketAddr::from(([127, 0, 0, 1], 8545)),
    );
    let eth_rpc_p2p = p2p_handle.clone();
    let eth_rpc_meta = node_meta.clone();
    tokio::spawn(async move {
        astram_node::server::run_eth_rpc_server(
            eth_rpc_node,
            eth_rpc_p2p,
            eth_rpc_meta,
            eth_rpc_addr,
        )
        .await;
    });

    // Graceful shutdown flag
    let shutdown_flag = Arc::new(AtomicBool::new(false));
    let shutdown_flag_clone = shutdown_flag.clone();
    let node_for_shutdown = node_handle.clone();

    // Setup signal handler for graceful shutdown
    tokio::spawn(async move {
        match signal::ctrl_c().await {
            Ok(()) => {
                println!("\n[WARN] Shutdown signal received, cleaning up...");
                println!(
                    "[INFO] Note: Connection errors during shutdown are expected and can be ignored"
                );
                shutdown_flag_clone.store(true, OtherOrdering::SeqCst);

                // Cancel ongoing mining immediately
                node_for_shutdown
                    .mining
                    .cancel_flag
                    .store(true, OtherOrdering::SeqCst);
                println!("[WARN] Mining cancellation requested...");
            }
            Err(err) => {
                eprintln!("Error setting up signal handler: {}", err);
            }
        }
    });

    let (task_handles, server_handle) = start_services(
        node_handle.clone(),
        p2p_handle.clone(),
        chain_state.clone(),
        node_meta.clone(),
        miner_address,
        shutdown_flag.clone(),
        node_settings.clone(),
    )
    .await;

    // Wait for all background tasks to complete
    println!("[INFO] Waiting for all tasks to complete...");
    for handle in task_handles {
        let _ = handle.await;
    }

    // Give time for active connections to close before aborting server
    println!("[INFO] Closing active connections...");
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Abort HTTP server (it runs indefinitely)
    // Note: This will cause connection errors for any open browser tabs - this is expected
    server_handle.abort();
    println!("[INFO] HTTP server stopped");

    // Give more time for all resources to be released
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Cleanup: Close database properly
    {
        println!("[INFO] Closing database...");

        // First, try to flush the DB while we still have a reference
        {
            if let Ok(bc) = node_handle.bc.lock() {
                // Flush WAL and compact
                if let Err(e) = bc.db.flush() {
                    log::warn!("Failed to flush DB: {}", e);
                } else {
                    println!("[OK] Database flushed");
                }

                // Cancel IO operations
                bc.db.cancel_all_background_work(true);
                println!("[INFO] Background work cancelled");
            }
        }

        // Give DB time to complete all operations
        tokio::time::sleep(Duration::from_secs(1)).await;

        // Drop the node_handle to release our reference
        drop(node_handle);
    }

    // Final wait to ensure LOCK file is released by OS
    tokio::time::sleep(Duration::from_secs(1)).await;

    println!("\n╔═══════════════════════════════════════╗");
    println!("║  Astram node stopped gracefully ✓    ║");
    println!("╚═══════════════════════════════════════╝");

    // Force process exit to ensure all resources are released
    std::process::exit(0);
}

/// Measure network latency to a peer by attempting a quick TCP connection
async fn measure_latency(address: &str) -> Option<u64> {
    let start = std::time::Instant::now();

    match tokio::time::timeout(
        Duration::from_secs(3),
        tokio::net::TcpStream::connect(address),
    )
    .await
    {
        Ok(Ok(_stream)) => {
            let latency = start.elapsed().as_millis() as u64;
            Some(latency)
        }
        _ => None,
    }
}

#[derive(Debug, Clone)]
struct ScoredPeer {
    address: String,
    height: u64,
    uptime_hours: f64,
    latency_ms: u64,
    score: f64,
}

/// Fetch best nodes from DNS server, excluding self
async fn fetch_best_nodes_from_dns(
    node_meta: Arc<NodeMeta>,
    settings: &NodeSettings,
    my_port: u16,
    limit: usize,
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    // Get my public address from state
    let my_address = { node_meta.my_public_address.lock().unwrap().clone() };

    let dns_url = settings.dns_server_url.clone();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5)) // 5 second timeout
        .build()?;
    let nodes_url = format!("{}/nodes?limit={}", dns_url, limit * 3); // Fetch more to test latency

    info!("Fetching best nodes from DNS server at {}", dns_url);

    let response = client.get(&nodes_url).send().await?;

    if response.status().is_success() {
        let result: DnsNodesResponse = response.json().await?;
        info!("Retrieved {} nodes from DNS server", result.count);

        // Filter out self - use public address if available
        let candidates: Vec<DnsNodeInfo> = result
            .nodes
            .into_iter()
            .filter(|node| {
                let node_id = format!("{}:{}", node.address, node.port);

                // Filter out exact match with public address (if we have it)
                if let Some(ref my_public_ip) = my_address {
                    let my_id = format!("{}:{}", my_public_ip, my_port);
                    if node_id == my_id {
                        info!("  Skipping {} - matches my public address", node_id);
                        return false;
                    }
                }

                // Filter out localhost addresses
                if node.address == "127.0.0.1"
                    || node.address == "localhost"
                    || node.address == "::1"
                {
                    info!(
                        "  Skipping {}:{} - localhost address",
                        node.address, node.port
                    );
                    return false;
                }

                true
            })
            .collect();

        info!(
            "Testing latency for {} candidate nodes...",
            candidates.len()
        );

        // Measure latency for each candidate in parallel
        let mut scored_peers = Vec::new();

        for node in candidates {
            let addr = format!("{}:{}", node.address, node.port);
            let latency = measure_latency(&addr).await;

            if let Some(latency_ms) = latency {
                // Calculate composite score:
                // - 30% height (normalized)
                // - 20% uptime (capped at 168h)
                // - 50% network latency (lower is better)

                // For scoring, we need to normalize. We'll do final scoring after collecting all
                scored_peers.push(ScoredPeer {
                    address: addr,
                    height: node.height,
                    uptime_hours: node.uptime_hours,
                    latency_ms,
                    score: 0.0, // Will calculate after we have all data
                });

                info!(
                    "  {} - height: {}, uptime: {:.1}h, latency: {}ms",
                    scored_peers.last().unwrap().address,
                    node.height,
                    node.uptime_hours,
                    latency_ms
                );
            } else {
                info!("  {}:{} - unreachable", node.address, node.port);
            }
        }

        if scored_peers.is_empty() {
            return Ok(vec![]);
        }

        // Normalize and calculate final scores
        let max_height = scored_peers.iter().map(|p| p.height).max().unwrap_or(1) as f64;
        let min_latency = scored_peers.iter().map(|p| p.latency_ms).min().unwrap_or(1) as f64;
        let max_latency = scored_peers
            .iter()
            .map(|p| p.latency_ms)
            .max()
            .unwrap_or(1000) as f64;

        for peer in &mut scored_peers {
            let height_score = (peer.height as f64 / max_height.max(1.0)) * 0.3;
            let uptime_score = (peer.uptime_hours.min(168.0) / 168.0) * 0.2;

            // Latency score: lower latency = higher score
            let latency_normalized = if max_latency > min_latency {
                1.0 - ((peer.latency_ms as f64 - min_latency) / (max_latency - min_latency))
            } else {
                1.0
            };
            let latency_score = latency_normalized * 0.5;

            peer.score = height_score + uptime_score + latency_score;
        }

        // Sort by score (descending)
        scored_peers.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // Log top peers
        info!("\n[INFO] Best peers by composite score:");
        for (i, peer) in scored_peers.iter().take(limit).enumerate() {
            info!(
                "  {}. {} - score: {:.3} (height: {}, uptime: {:.1}h, latency: {}ms)",
                i + 1,
                peer.address,
                peer.score,
                peer.height,
                peer.uptime_hours,
                peer.latency_ms
            );
        }

        let best_peers: Vec<String> = scored_peers
            .into_iter()
            .take(limit)
            .map(|p| p.address)
            .collect();

        Ok(best_peers)
    } else {
        let error_text = response.text().await?;
        Err(format!("Failed to fetch nodes from DNS server: {}", error_text).into())
    }
}

/// Register this node with the DNS server (non-blocking version)
/// Height is optional and only used for informational purposes
async fn register_with_dns(
    _node_handle: NodeHandle, // Not used - we don't need to lock for DNS registration
    settings: &NodeSettings,
    height: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    let dns_url = settings.dns_server_url.clone();
    let node_port = settings.p2p_port;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()?;
    let register_url = format!("{}/register", dns_url);

    let payload = serde_json::json!({
        "port": node_port,
        "version": "0.1.0",
        "height": height
    });

    let response = client.post(&register_url).json(&payload).send().await?;

    if response.status().is_success() {
        #[derive(serde::Deserialize)]
        struct RegisterResponse {
            #[serde(rename = "success")]
            _success: bool,
            message: String,
            #[serde(rename = "node_count")]
            _node_count: usize,
            registered_address: String,
            registered_port: u16,
        }

        let result: RegisterResponse = response.json().await?;
        info!(
            "Successfully registered with DNS server: {} ({}:{})",
            result.message, result.registered_address, result.registered_port
        );
        Ok(())
    } else {
        let error_text = response.text().await?;
        Err(format!("Failed to register with DNS server: {}", error_text).into())
    }
}

/// Synchronize blockchain with peers
async fn sync_blockchain(
    node_handle: NodeHandle,
    p2p_handle: Arc<astram_node::p2p::manager::PeerManager>,
) -> Result<(), Box<dyn std::error::Error>> {
    info!("[INFO] Starting blockchain synchronization...");

    let my_height = {
        let bc = node_handle.bc.lock().unwrap();
        if let Some(tip_hash) = &bc.chain_tip {
            if let Ok(Some(header)) = bc.load_header(tip_hash) {
                header.index + 1
            } else {
                0
            }
        } else {
            0
        }
    };

    info!("[INFO] Local blockchain height: {}", my_height);

    // Get peer heights
    let peer_heights = p2p_handle.get_peer_heights();

    if peer_heights.is_empty() {
        info!("[WARN] No peers connected yet, skipping sync");
        return Ok(());
    }

    let max_peer_height = peer_heights.values().max().copied().unwrap_or(0);
    info!("[INFO] Maximum peer height: {}", max_peer_height);

    if my_height >= max_peer_height {
        info!(
            "[INFO] Blockchain is already up to date (height: {})",
            my_height
        );
        return Ok(());
    }

    let blocks_behind = max_peer_height - my_height;
    info!(
        "[INFO] Need to sync {} blocks (from {} to {})",
        blocks_behind, my_height, max_peer_height
    );

    // For initial sync (when we have no blocks), request genesis block first
    if my_height == 0 {
        info!("[INFO] Requesting genesis block from peers...");
        // Request with empty locator to get blocks from the beginning
        p2p_handle.request_headers_from_peers(vec![], None);
    } else {
        // Request headers from our current tip
        let mut locator_hashes = Vec::new();
        {
            let bc = node_handle.bc.lock().unwrap();
            if let Some(tip_hash) = &bc.chain_tip {
                if let Ok(bytes) = hex::decode(tip_hash) {
                    locator_hashes.push(bytes);
                }
            }
        }
        info!("[INFO] Requesting headers from peers...");
        p2p_handle.request_headers_from_peers(locator_hashes, None);
    }

    // Wait for blocks to arrive (give peers time to respond)
    // Increase timeout for larger syncs
    let sync_timeout = Duration::from_secs(60);
    let sync_start = std::time::Instant::now();
    let mut last_height = my_height;

    loop {
        sleep(Duration::from_secs(2)).await;

        let current_height = {
            let bc = node_handle.bc.lock().unwrap();
            if let Some(tip_hash) = &bc.chain_tip {
                if let Ok(Some(header)) = bc.load_header(tip_hash) {
                    header.index + 1
                } else {
                    0
                }
            } else {
                0
            }
        };

        // Check if we made progress
        if current_height > last_height {
            info!(
                "[INFO] Sync progress: {} / {} blocks",
                current_height, max_peer_height
            );
            last_height = current_height;

            // Request more headers if we're still behind
            if current_height < max_peer_height {
                let mut locator_hashes = Vec::new();
                {
                    let bc = node_handle.bc.lock().unwrap();
                    if let Some(tip_hash) = &bc.chain_tip {
                        if let Ok(bytes) = hex::decode(tip_hash) {
                            locator_hashes.push(bytes);
                        }
                    }
                }
                p2p_handle.request_headers_from_peers(locator_hashes, None);
            }
        }

        if current_height >= max_peer_height {
            info!("[OK] Blockchain synchronized to height {}", current_height);
            break;
        }

        if sync_start.elapsed() > sync_timeout {
            info!(
                "[WARN] Sync timeout reached. Current height: {} (target: {})",
                current_height, max_peer_height
            );
            info!("[INFO] Will continue syncing in background via periodic header requests");
            break;
        }
    }

    Ok(())
}

async fn start_services(
    node_handle: NodeHandle,
    p2p_handle: Arc<astram_node::p2p::manager::PeerManager>,
    chain_state: Arc<Mutex<ChainState>>,
    node_meta: Arc<NodeMeta>,
    miner_address: String,
    shutdown_flag: Arc<AtomicBool>,
    settings: Arc<NodeSettings>,
) -> (
    Vec<tokio::task::JoinHandle<()>>,
    tokio::task::JoinHandle<()>,
) {
    println!("[INFO] my address {}", miner_address);

    let mut task_handles = Vec::new();

    let my_node_port = settings.p2p_port;

    // Register with DNS server initially with height 0 (or try to read non-blocking)
    // Don't hold lock for DNS registration - it can fail/timeout without affecting mining
    let initial_height = if let Ok(bc) = node_handle.bc.try_lock() {
        if let Some(tip_hash) = &bc.chain_tip {
            if let Ok(Some(header)) = bc.load_header(tip_hash) {
                header.index + 1
            } else {
                0
            }
        } else {
            0
        }
    } else {
        0 // If can't read without blocking, use 0
    };

    // Register with DNS server (fail fast if registration fails)
    // Note: This is outside the main mining loop, so it happens only once at startup
    // Periodic re-registration is done without trying to acquire any locks
    if let Err(e) = register_with_dns(node_handle.clone(), &settings, initial_height).await {
        log::error!("DNS registration failed; shutting down node: {}", e);
        std::process::exit(1);
    }

    let dns_node_handle = node_handle.clone();
    let shutdown_flag_dns = shutdown_flag.clone();
    let settings_dns = settings.clone();
    let dns_task = tokio::spawn(async move {
        // Re-register every 5 minutes to keep the node alive in DNS
        let mut interval = tokio::time::interval(Duration::from_secs(300));
        interval.tick().await; // Skip first immediate tick

        info!("[DNS] Re-registration task started (interval: 300s)");

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    if shutdown_flag_dns.load(OtherOrdering::SeqCst) {
                        info!("DNS registration task shutting down...");
                        break;
                    }

                    let tick_start = std::time::Instant::now();
                    info!("[DNS] ⏰ Re-registration tick START");

                    // Get current height for DNS (non-blocking attempt)
                    let height = if let Ok(bc) = dns_node_handle.bc.try_lock() {
                        info!("[DNS] bc.try_lock() success");
                        if let Some(tip_hash) = &bc.chain_tip {
                            if let Ok(Some(header)) = bc.load_header(tip_hash) {
                                header.index + 1
                            } else {
                                0
                            }
                        } else {
                            0
                        }
                    } else {
                        warn!("[DNS] bc.try_lock() failed - lock contended");
                        0 // If mining is running and lock is held, just use 0
                    };

                    info!("[DNS] Height determined: {} (took {:?})", height, tick_start.elapsed());

                    // Spawn DNS registration asynchronously - never blocks mining
                    let dns_handle_clone = dns_node_handle.clone();
                    let settings_clone = settings_dns.clone();
                    let spawn_time = std::time::Instant::now();
                    tokio::spawn(async move {
                        let start = std::time::Instant::now();
                        info!("[DNS] Registration task spawned (spawn delay: {:?})", spawn_time.elapsed());
                        match tokio::time::timeout(
                            Duration::from_secs(2),
                            register_with_dns(dns_handle_clone.clone(), &settings_clone, height),
                        )
                        .await
                        {
                            Ok(Ok(())) => {
                                info!(
                                    "[DNS] ✅ Re-registration OK (height={}, took {:?})",
                                    height,
                                    start.elapsed()
                                );
                            }
                            Ok(Err(e)) => {
                                warn!(
                                    "[DNS] ❌ Re-registration FAILED (height={}, took {:?}): {:?}",
                                    height,
                                    start.elapsed(),
                                    e
                                );
                            }
                            Err(e) => {
                                warn!(
                                    "[DNS] ⏱️  Re-registration TIMEOUT (height={}, took {:?}): {:?}",
                                    height,
                                    start.elapsed(),
                                    e
                                );
                            }
                        }
                    });

                    info!("[DNS] ⏰ Re-registration tick DONE (tick handling took {:?})", tick_start.elapsed());
                }
                _ = tokio::time::sleep(Duration::from_secs(1)) => {
                    // Check shutdown flag every second for quick response
                    if shutdown_flag_dns.load(OtherOrdering::SeqCst) {
                        info!("DNS registration task shutting down...");
                        break;
                    }
                }
            }
        }
    });
    task_handles.push(dns_task);

    // Connect to best nodes from DNS server
    let shutdown_flag_p2p = shutdown_flag.clone();
    let p2p_handle_for_task = p2p_handle.clone();
    let node_meta_for_p2p = node_meta.clone();
    let settings_p2p = settings.clone();
    let p2p_task = tokio::spawn(async move {
        // Wait a bit for DNS registration to complete
        sleep(Duration::from_secs(2)).await;

        // Initial connection to best nodes
        match fetch_best_nodes_from_dns(node_meta_for_p2p.clone(), &settings_p2p, my_node_port, 10)
            .await
        {
            Ok(peer_addrs) => {
                info!(
                    "[INFO] Connecting to {} best nodes from DNS",
                    peer_addrs.len()
                );
                for addr in peer_addrs {
                    let p2p_clone = p2p_handle_for_task.clone();
                    let addr_clone = addr.clone();
                    tokio::spawn(async move {
                        if let Err(e) = p2p_clone.connect_peer(&addr_clone).await {
                            log::warn!("Failed to connect to peer {}: {:?}", addr_clone, e);
                        } else {
                            info!("[OK] Connected to peer: {}", addr_clone);
                        }
                    });
                }
            }
            Err(e) => {
                log::warn!("Failed to fetch best nodes from DNS: {}", e);
            }
        }

        // Periodically refresh connections to best nodes (every 10 minutes)
        let mut interval = tokio::time::interval(Duration::from_secs(600));
        interval.tick().await; // Skip first immediate tick

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    if shutdown_flag_p2p.load(OtherOrdering::SeqCst) {
                        info!("P2P connection refresh task shutting down...");
                        break;
                    }
                    match fetch_best_nodes_from_dns(
                        node_meta_for_p2p.clone(),
                        &settings_p2p,
                        my_node_port,
                        10,
                    )
                    .await
                    {
                Ok(peer_addrs) => {
                    info!(
                        "[INFO] Refreshing connections to {} best nodes",
                        peer_addrs.len()
                    );
                    for addr in peer_addrs {
                        let p2p_clone = p2p_handle_for_task.clone();
                        let addr_clone = addr.clone();
                        tokio::spawn(async move {
                            let _ = p2p_clone.connect_peer(&addr_clone).await;
                        });
                    }
                }
                Err(e) => {
                    log::warn!("Failed to refresh nodes from DNS: {}", e);
                }
            }
                }
                _ = tokio::time::sleep(Duration::from_secs(1)) => {
                    // Check shutdown flag every second for quick response
                    if shutdown_flag_p2p.load(OtherOrdering::SeqCst) {
                        info!("P2P connection refresh task shutting down...");
                        break;
                    }
                }
            }
        }
    });
    task_handles.push(p2p_task);

    // Wait for initial P2P connections to establish
    info!("[INFO] Waiting for P2P connections to establish...");
    sleep(Duration::from_secs(5)).await;

    // Step 5: Synchronize blockchain with peers
    info!("[INFO] Step 5: Synchronizing blockchain with peers...");
    if let Err(e) = sync_blockchain(node_handle.clone(), p2p_handle.clone()).await {
        log::warn!("Blockchain sync encountered error: {}", e);
    }

    let nh: NodeHandle = node_handle.clone();
    let http_addr = to_socket_addr(
        &settings.http_bind_addr,
        settings.http_port,
        SocketAddr::from(([127, 0, 0, 1], 19533)),
    );
    // start HTTP server in background thread (warp is async so run in tokio)
    let server_p2p = p2p_handle.clone();
    let server_chain = chain_state.clone();
    let server_meta = node_meta.clone();
    let server_handle = tokio::spawn(async move {
        run_server(nh, server_p2p, server_chain, server_meta, http_addr).await;
    });

    // Step 6: Start mining
    println!("[INFO] Step 6: Starting mining...");

    // Mining loop - run in main task, not spawned
    mining_loop(
        node_handle.clone(),
        p2p_handle.clone(),
        chain_state.clone(),
        miner_address,
        shutdown_flag.clone(),
    )
    .await;

    // Return background tasks, but not server (we'll abort it)
    (task_handles, server_handle)
}

async fn mining_loop(
    node_handle: NodeHandle,
    p2p_handle: Arc<astram_node::p2p::manager::PeerManager>,
    chain_state: Arc<Mutex<ChainState>>,
    miner_address: String,
    shutdown_flag: Arc<AtomicBool>,
) {
    let requested_backend = std::env::var("MINER_BACKEND")
        .unwrap_or_else(|_| "cpu".to_string())
        .to_lowercase();
    let mut miner_backend = requested_backend.clone();

    if miner_backend == "cuda" && !cfg!(feature = "cuda-miner") {
        println!("[WARN] CUDA miner requested but not enabled; falling back to CPU");
        miner_backend = "cpu".to_string();
    }

    if miner_backend == "cuda" {
        println!("[INFO] Using CUDA miner backend");
    } else {
        println!("[INFO] Using CPU miner backend");
    }

    loop {
        // Check shutdown flag
        if shutdown_flag.load(OtherOrdering::SeqCst) {
            info!("[WARN] Shutdown flag detected, stopping mining loop...");
            // Ensure cancel flag is set
            node_handle
                .mining
                .cancel_flag
                .store(true, OtherOrdering::SeqCst);
            break;
        }

        // Snapshot pending txs + mining params while holding the lock briefly
        println!("[DEBUG] Mining: Attempting to acquire WRITE lock...");
        let (snapshot_txs, difficulty, prev_hash, index_snapshot, cancel_flag, hashrate_shared) = {
            println!("[DEBUG] Mining: WRITE lock acquired");

            // Mark mining as active
            node_handle.mining.active.store(true, OtherOrdering::SeqCst);

            // Reset cancel flag at the start of each mining round
            node_handle
                .mining
                .cancel_flag
                .store(false, OtherOrdering::SeqCst);

            // Take pending transactions to work on them outside the lock
            let txs_copy = {
                let mut mempool = node_handle.mempool.lock().unwrap();
                let txs = mempool.pending.clone();
                mempool.pending.clear();
                txs
            };

            let (prev_hash, next_index, diff) = {
                let mut bc = node_handle.bc.lock().unwrap();

                // previous tip hash
                let prev_hash = bc.chain_tip.clone().unwrap_or_else(|| "0".repeat(64));

                // determine next index from tip header (so header.index is known before mining)
                println!("[DEBUG] Mining: Loading tip header to determine next index");
                let next_index: u64 = if let Some(tip_hash) = bc.chain_tip.clone() {
                    println!(
                        "[DEBUG] Mining: Loading header for hash: {}",
                        &tip_hash[..16]
                    );
                    if let Ok(Some(prev_header)) = bc.load_header(&tip_hash) {
                        let next_index = prev_header.index + 1;
                        println!("[DEBUG] Mining: Got next_index = {}", next_index);
                        next_index
                    } else {
                        println!("[DEBUG] Mining: Failed to load header, using 0");
                        0
                    }
                } else {
                    println!("[DEBUG] Mining: No chain_tip, using next_index = 0");
                    0
                };

                // Calculate difficulty for the next block (dynamic adjustment every 30 blocks)
                let diff = bc
                    .calculate_adjusted_difficulty(next_index)
                    .unwrap_or(bc.difficulty);

                if diff != bc.difficulty {
                    println!(
                        "[INFO] Difficulty adjusted: {} -> {} (block #{})",
                        bc.difficulty, diff, next_index
                    );
                    // Update blockchain difficulty before mining
                    bc.difficulty = diff;
                }

                (prev_hash, next_index, diff)
            };

            // Update current difficulty in state
            *node_handle.mining.current_difficulty.lock().unwrap() = diff;

            (
                txs_copy,
                diff,
                prev_hash,
                next_index,
                node_handle.mining.cancel_flag.clone(),
                node_handle.mining.current_hashrate.clone(),
            )
        };
        println!("[DEBUG] Mining: WRITE lock released");
        // Write lock released - calculate fees OUTSIDE the lock

        // Calculate total fees from pending transactions (with separate read lock for DB)
        println!("[DEBUG] Mining: Attempting to acquire READ lock for fees...");
        let total_fees = {
            let state = node_handle.clone();
            println!("[DEBUG] Mining: READ lock acquired for fees");
            let mut fee_sum = U256::zero();
            let bc = state.bc.lock().unwrap();

            for tx in &snapshot_txs {
                // Calculate fee: input_sum - output_sum
                let mut input_sum = U256::zero();
                let mut output_sum = U256::zero();

                // Sum inputs (from UTXO)
                for inp in &tx.inputs {
                    let ukey = format!("u:{}:{}", inp.txid, inp.vout);
                    if let Ok(Some(blob)) = bc.db.get(ukey.as_bytes()) {
                        if let Ok((utxo, _)) =
                            bincode::decode_from_slice::<Utxo, _>(&blob, *BINCODE_CONFIG)
                        {
                            input_sum += utxo.amount();
                        }
                    }
                }

                // Sum outputs
                for out in &tx.outputs {
                    output_sum += out.amount();
                }

                // Fee is the difference
                if input_sum >= output_sum {
                    let fee = input_sum - output_sum;
                    fee_sum += fee;
                }
            }

            fee_sum
        };
        println!("[DEBUG] Mining: READ lock released after fees");
        // Read lock released

        // prepare block transactions: coinbase + pending
        // NOTE: we pass pending txs to consensus::mine_block_with_coinbase which will prepend coinbase
        let block_txs_for_logging = snapshot_txs.len();
        println!("[INFO] Mining {} pending tx(s)...", block_txs_for_logging);

        // Coinbase reward = block reward + total fees
        let base_reward = current_block_reward_snapshot();
        let coinbase_reward = base_reward + total_fees;

        if total_fees > U256::zero() {
            let fees_asrm = total_fees / U256::from(1_000_000_000_000_000_000u64);
            println!(
                "[INFO] Total fees in block: {} wei ({} ASRM)",
                total_fees, fees_asrm
            );
        }
        println!(
            "[INFO] Coinbase reward: {} (base: {} + fees: {})",
            coinbase_reward, base_reward, total_fees
        );

        // Record mining start time for hashrate calculation
        let mining_start = std::time::Instant::now();

        log::info!(
            "[INFO] Starting mining task for block {} with difficulty {}...",
            index_snapshot,
            difficulty
        );

        // prepare parameters for blocking mining call
        let prev_hash = prev_hash.clone();
        let difficulty_local = difficulty;
        let index_local = index_snapshot;
        let miner_addr_cloned = miner_address.clone();
        let txs_cloned = snapshot_txs.clone();
        let cancel_for_thread = cancel_flag.clone();
        let hashrate_for_thread = hashrate_shared.clone();

        // Run mining in a blocking task so we don't block the tokio runtime
        let backend = miner_backend.clone();
        println!("[DEBUG] ⏳ Spawning mining task on blocking thread pool...");
        let mined_block_res: anyhow::Result<Block> = tokio::task::spawn_blocking(move || {
            if backend == "cuda" {
                #[cfg(feature = "cuda-miner")]
                {
                    return consensus::mine_block_with_coinbase_cuda(
                        index_local,
                        prev_hash,
                        difficulty_local,
                        txs_cloned,
                        &miner_addr_cloned,
                        coinbase_reward,
                        cancel_for_thread,
                        Some(hashrate_for_thread),
                    );
                }
                #[cfg(not(feature = "cuda-miner"))]
                {
                    return Err(anyhow::anyhow!(
                        "CUDA miner not enabled. Build with --features cuda-miner"
                    ));
                }
            }

            println!("[DEBUG] 🔨 Mining thread: Using CPU backend, starting consensus mining...");
            let block = consensus::mine_block_with_coinbase(
                index_local,
                prev_hash,
                difficulty_local,
                txs_cloned,
                &miner_addr_cloned,
                coinbase_reward,
                cancel_for_thread,
                Some(hashrate_for_thread),
            );
            println!("[DEBUG] 🔨 Mining thread: consensus::mine_block_with_coinbase returned!");
            block
        })
        .await
        .expect("mining task panicked");

        println!("[DEBUG] ✅ Mining task COMPLETED and returned to main thread!");

        match mined_block_res {
            Ok(block) => {
                // Note: We do NOT modify the mined block's timestamp or hash
                // because that would invalidate the PoW nonce that was just found.
                // The block is already valid as-is from mining.

                println!("[DEBUG] Validating and inserting block into blockchain DB...");
                match node_handle
                    .bc
                    .lock()
                    .unwrap()
                    .validate_and_insert_block(&block)
                {
                    Ok(_) => {
                        println!(
                            "[OK]✅ Block saved to DB - index={} hash={}",
                            block.header.index, block.hash
                        );

                        // Update mining statistics
                        node_handle
                            .mining
                            .blocks_mined
                            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

                        // Calculate hashrate (rough estimate)
                        let mining_duration = mining_start.elapsed().as_secs_f64();
                        if mining_duration > 0.0 {
                            // Estimate: 2^difficulty hashes attempted in mining_duration seconds
                            let estimated_hashes = 2_u64.pow(difficulty_local) as f64;
                            let hashrate = estimated_hashes / mining_duration;
                            *node_handle.mining.current_hashrate.lock().unwrap() = hashrate;
                        }

                        let block_to_broadcast = block.clone();

                        {
                            let mut chain = chain_state.lock().unwrap();
                            chain.blockchain.push(block.clone());
                            chain.enforce_memory_limit(); // Security: Enforce memory limit
                        }
                        // pending already cleared earlier

                        // Update P2P manager height
                        p2p_handle.set_my_height(block.header.index + 1);

                        // Track this block as recently mined (to ignore when received from peers)
                        let now = chrono::Utc::now().timestamp();
                        {
                            let mut chain = chain_state.lock().unwrap();
                            chain.recently_mined_blocks.insert(block.hash.clone(), now);

                            // Clean up old entries (older than 5 minutes)
                            chain
                                .recently_mined_blocks
                                .retain(|_, &mut timestamp| now - timestamp < 300);
                        }

                        println!("[OK] Block mined! Broadcasting...");

                        // -------------------------
                        // Broadcast mined block
                        // -------------------------
                        // broadcast_block returns () (fire-and-forget), so just await it
                        p2p_handle.broadcast_block(&block_to_broadcast).await;
                    }
                    Err(e) => {
                        eprintln!("Block insertion failed: {}", e);
                        // requeue non-coinbase txs back to pending
                        {
                            let mut mempool = node_handle.mempool.lock().unwrap();
                            for tx in block.transactions.into_iter().skip(1) {
                                mempool.pending.push(tx);
                            }
                            // Security: Enforce mempool limits
                            mempool.enforce_mempool_limit();
                        }
                    }
                }
            }
            Err(e) => {
                let error_msg = format!("{}", e);

                // Check if mining was cancelled (not an actual error)
                if error_msg.contains("cancelled") || error_msg.contains("Mining cancelled") {
                    info!("[INFO] Mining cancelled (normal)");
                } else {
                    eprintln!("[ERROR] Mining error: {}", e);
                }

                // Mark mining as inactive and reset hashrate
                node_handle
                    .mining
                    .active
                    .store(false, OtherOrdering::SeqCst);
                *node_handle.mining.current_hashrate.lock().unwrap() = 0.0;

                // Only requeue txs if it wasn't a cancellation
                if !error_msg.contains("cancelled") && !error_msg.contains("Mining cancelled") {
                    let mut mempool = node_handle.mempool.lock().unwrap();
                    for tx in snapshot_txs.into_iter() {
                        mempool.pending.push(tx);
                    }
                    // Security: Enforce mempool limits
                    mempool.enforce_mempool_limit();
                }
            }
        }

        // Wait before next cycle, but check shutdown flag frequently for quick response
        println!("[INFO] ⏱️  Waiting 3 seconds before next mining cycle...");
        for _ in 0..3 {
            if shutdown_flag.load(OtherOrdering::SeqCst) {
                info!("[WARN] Shutdown detected during sleep, exiting mining loop");
                return;
            }
            sleep(Duration::from_secs(1)).await;
        }
        println!("[DEBUG] Mining cycle: Sleep completed, starting next iteration...");
    }
}

fn current_block_reward_snapshot() -> U256 {
    // For now, always return initial reward (genesis/early blocks)
    // In production, this would take current blockchain height as parameter
    initial_block_reward()
}
