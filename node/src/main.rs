#[cfg(not(feature = "cuda-miner"))]
compile_error!("Astram-node is GPU-only. Build with CUDA support (`cuda-miner` feature enabled).");

// Use library exports instead of declaring local modules to avoid duplicate crate types
use Astram_core::Blockchain;
use Astram_core::block::Block;
use Astram_core::config::calculate_block_reward;
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

use std::collections::HashSet;
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
    bootstrap_peers: Vec<String>,
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
            bootstrap_peers: Vec::new(),
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
                    "ASTRAM_NETWORK" | "ASTRAM_NETWORK_ID" | "ASTRAM_CHAIN_ID" | "ASTRAM_NETWORK_MAGIC" => {
                        #[cfg(debug_assertions)]
                        {
                            // Debug builds: allow environment variable overrides for testing
                            unsafe {
                                std::env::set_var(key, value);
                            }
                        }
                        #[cfg(not(debug_assertions))]
                        {
                            // Release builds: network parameters are hardcoded, ignore config overrides
                            warn!("[RELEASE] Network parameter override '{}' ignored - using hardcoded mainnet values", key);
                        }
                    }
                    "BOOTSTRAP_PEERS" => {
                        settings.bootstrap_peers = value
                            .split(',')
                            .map(|entry| entry.trim().to_string())
                            .filter(|entry| !entry.is_empty())
                            .collect();
                    }
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
        .filter_level(log::LevelFilter::Info)
        .filter_module("warp", log::LevelFilter::Warn)
        .filter_module("hyper", log::LevelFilter::Warn)
        .filter_module("reqwest", log::LevelFilter::Warn)
        .filter_module("Astram::http", log::LevelFilter::Warn)
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

    // Check and recover tip if needed
    {
        let mut bc_guard = bc.lock().unwrap();
        
        // Always count blocks for diagnostic purposes
        let block_count = bc_guard.count_blocks();
        log::info!("📊 Database contains {} blocks", block_count);
        
        if let Some(tip_hash) = &bc_guard.chain_tip {
            // Tip exists - verify it's the highest block
            if let Ok(Some(header)) = bc_guard.load_header(tip_hash) {
                log::info!("✅ Chain tip verified at height {}", header.index);
                
                // Check for significant mismatch and auto-recover
                if block_count > 100 && header.index + 1 < block_count as u64 / 2 {
                    log::warn!(
                        "⚠️  Potential issue: tip at height {} but {} blocks in DB",
                        header.index, block_count
                    );
                    log::warn!("   Automatically triggering tip recovery...");
                    if let Err(e) = bc_guard.recover_tip() {
                        log::error!("Failed to recover tip: {}", e);
                    } else {
                        // Log the new tip after recovery
                        if let Some(new_tip_hash) = &bc_guard.chain_tip {
                            if let Ok(Some(new_header)) = bc_guard.load_header(new_tip_hash) {
                                log::info!("🎉 Tip successfully recovered to height {}", new_header.index);
                            }
                        }
                    }
                }
            } else {
                log::warn!("⚠️  Chain tip points to missing block, attempting recovery...");
                if let Err(e) = bc_guard.recover_tip() {
                    log::error!("Failed to recover tip: {}", e);
                }
            }
        } else {
            // No tip - check if there are blocks in DB that need recovery
            log::info!("No tip found, checking if recovery needed...");
            if let Err(e) = bc_guard.recover_tip() {
                log::debug!("No blocks to recover (fresh database): {}", e);
            }
        }
    }

    // Initialize P2P networking
    let p2p_service = P2PService::new();

    let mining_state = Arc::new(MiningState::default());

    let p2p_handle = p2p_service.manager();

    let chain_state = Arc::new(Mutex::new(ChainState::default()));
    let node_meta = Arc::new(NodeMeta {
        miner_address: Arc::new(Mutex::new(miner_address.clone())),
        my_public_address: Arc::new(Mutex::new(None)),
        node_start_time: std::time::Instant::now(),
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
            log::info!("[INIT] Chain tip hash: {}", tip_hash);
            match bc.load_header(tip_hash) {
                Ok(Some(header)) => {
                    let height = header.index;
                    log::info!("[INIT] Successfully loaded tip header at height {}", height);
                    height
                }
                Ok(None) => {
                    log::error!("[INIT] Chain tip '{}' exists but header not found in DB!", tip_hash);
                    0
                }
                Err(e) => {
                    log::error!("[INIT] Failed to load chain tip header: {}", e);
                    0
                }
            }
        } else {
            log::info!("[INIT] No chain tip found (empty blockchain)");
            0
        }
    };
    p2p_handle.set_my_height(my_height);
    info!("[INFO] Local blockchain height set to: {}", my_height);

    let bind_addr = format!("{}:{}", node_settings.p2p_bind_addr, node_settings.p2p_port);

    // Set listening port in P2P manager (for self-connection detection)
    p2p_handle.set_my_listening_port(node_settings.p2p_port);
    p2p_handle.set_my_bind_addr(node_settings.p2p_bind_addr.clone());

    p2p_service
        .start(bind_addr, node_handle.clone(), chain_state.clone())
        .await
        .expect("p2p start failed");

    // Start Ethereum JSON-RPC server for MetaMask
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
        Ok(Err(e)) => {
            log::debug!("Connection to {} failed: {}", address, e);
            None
        }
        Err(_) => {
            log::debug!("Connection to {} timed out after 3s", address);
            None
        }
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

/// Get the appropriate localhost address for connecting to a local peer
/// - In WSL: Returns the Windows host IP (from default gateway)
/// - Otherwise: Returns 127.0.0.1
fn get_localhost_address() -> String {
    // Check if we're in WSL environment
    if std::env::var("WSL_DISTRO_NAME").is_ok() || 
       std::env::var("WSL_INTEROP").is_ok() {
        // Method 1: Try to get default gateway from 'ip route' command
        if let Ok(output) = std::process::Command::new("ip")
            .args(&["route", "show", "default"])
            .output() 
        {
            if let Ok(route_output) = String::from_utf8(output.stdout) {
                // Parse: "default via 172.25.128.1 dev eth0"
                for part in route_output.split_whitespace() {
                    // Look for IP address pattern after "via"
                    if part.contains('.') && !part.starts_with("127.") {
                        // Basic validation: check if it looks like an IP
                        let segments: Vec<&str> = part.split('.').collect();
                        if segments.len() == 4 && segments.iter().all(|s| s.parse::<u8>().is_ok()) {
                            info!("🔧 WSL detected: using Windows host IP {} (from default gateway)", part);
                            return part.to_string();
                        }
                    }
                }
            }
        }
        
        // Method 2: Fallback to /etc/resolv.conf nameserver
        if let Ok(contents) = std::fs::read_to_string("/etc/resolv.conf") {
            for line in contents.lines() {
                if line.trim().starts_with("nameserver") {
                    if let Some(ip) = line.split_whitespace().nth(1) {
                        // Validate it's not a loopback address
                        if !ip.starts_with("127.") {
                            info!("🔧 WSL detected: using Windows host IP {} (from /etc/resolv.conf)", ip);
                            return ip.to_string();
                        }
                    }
                }
            }
        }
        
        warn!("⚠️  WSL detected but could not determine Windows host IP, falling back to 127.0.0.1");
    }
    
    // Default to standard localhost
    "127.0.0.1".to_string()
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

        // Log all discovered nodes for visibility
        for (idx, node) in result.nodes.iter().enumerate() {
            info!(
                "  DNS Node {}: {}:{} (height: {}, uptime: {:.1}h)",
                idx + 1,
                node.address,
                node.port,
                node.height,
                node.uptime_hours
            );
        }

        // Filter out self - use public address if available
        let candidates: Vec<DnsNodeInfo> = result
            .nodes
            .into_iter()
            .filter(|node| {
                let node_id = format!("{}:{}", node.address, node.port);

                // Filter out exact match with public address+port (if we have it)
                if let Some(ref my_public_ip) = my_address {
                    let my_id = format!("{}:{}", my_public_ip, my_port);
                    if node_id == my_id {
                        info!("  ⏭️  Skipping {} - exact match with my public IP:port ({})", node_id, my_id);
                        return false;
                    }
                }

                // Filter out localhost addresses
                if node.address == "127.0.0.1"
                    || node.address == "localhost"
                    || node.address == "::1"
                {
                    info!(
                        "  ⏭️  Skipping {}:{} - localhost address",
                        node.address, node.port
                    );
                    return false;
                }

                info!("  ✅ Candidate: {}:{} (height: {})", node.address, node.port, node.height);
                true
            })
            .collect();

        info!(
            "Selected {} candidates for latency testing",
            candidates.len()
        );

        let fallback_candidates: Vec<String> = candidates
            .iter()
            .map(|node| format!("{}:{}", node.address, node.port))
            .collect();

        // Measure latency for each candidate in parallel
        let mut scored_peers = Vec::new();

        info!("🔍 Starting latency measurements for {} candidates...", candidates.len());
        for node in candidates.into_iter() {
            let addr = format!("{}:{}", node.address, node.port);
            
            // Convert to localhost if it's the same public IP (for local node discovery)
            let test_addr = if let Some(ref my_public_ip) = my_address {
                if node.address == *my_public_ip {
                    let localhost_ip = get_localhost_address();
                    let localhost_addr = format!("{}:{}", localhost_ip, node.port);
                    info!("  Testing latency to {} (using {} for local connection)...", addr, localhost_addr);
                    localhost_addr
                } else {
                    info!("  Testing latency to {}...", addr);
                    addr.clone()
                }
            } else {
                info!("  Testing latency to {}...", addr);
                addr.clone()
            };
            
            let latency = measure_latency(&test_addr).await;

            if let Some(latency_ms) = latency {
                // Calculate composite score:
                // - 30% height (normalized)
                // - 20% uptime (capped at 168h)
                // - 50% network latency (lower is better)

                // For scoring, we need to normalize. We'll do final scoring after collecting all
                scored_peers.push(ScoredPeer {
                    address: addr.clone(),
                    height: node.height,
                    uptime_hours: node.uptime_hours,
                    latency_ms,
                    score: 0.0, // Will calculate after we have all data
                });

                info!(
                    "  ✅ {} - height: {}, uptime: {:.1}h, latency: {}ms",
                    addr,
                    node.height,
                    node.uptime_hours,
                    latency_ms
                );
            } else {
                warn!("  ❌ {} - unreachable (latency probe failed)", addr);
            }
        }

        info!("📊 Latency testing complete: {}/{} peers reachable", scored_peers.len(), fallback_candidates.len());

        if scored_peers.is_empty() {
            let fallback_count = fallback_candidates.len().min(limit);
            if fallback_count > 0 {
                warn!(
                    "⚠️  No peers passed latency probe; falling back to {} raw DNS candidates",
                    fallback_count
                );
                for (idx, addr) in fallback_candidates.iter().take(fallback_count).enumerate() {
                    info!("  Fallback {}: {}", idx + 1, addr);
                }
            } else {
                warn!("❌ No peers available (DNS candidates and latency probes both failed)");
            }
            return Ok(fallback_candidates.into_iter().take(limit).collect());
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

fn build_fallback_peer_targets(
    p2p_handle: &Arc<astram_node::p2p::manager::PeerManager>,
    settings: &NodeSettings,
    my_public_ip: Option<String>,
    my_port: u16,
    limit: usize,
) -> Vec<String> {
    let mut unique = HashSet::new();
    let mut targets = Vec::new();
    let my_public_id = my_public_ip.map(|ip| format!("{}:{}", ip, my_port));

    for raw in &settings.bootstrap_peers {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }

        let normalized = if trimmed.contains(':') {
            trimmed.to_string()
        } else {
            format!("{}:{}", trimmed, my_port)
        };

        if my_public_id.as_deref() == Some(normalized.as_str()) {
            continue;
        }

        if unique.insert(normalized.clone()) {
            targets.push(normalized);
        }

        if targets.len() >= limit {
            return targets;
        }
    }

    for saved in p2p_handle.load_saved_peers() {
        let addr = saved.addr.trim().to_string();
        if addr.is_empty() {
            continue;
        }
        if my_public_id.as_deref() == Some(addr.as_str()) {
            continue;
        }

        if unique.insert(addr.clone()) {
            targets.push(addr);
        }

        if targets.len() >= limit {
            return targets;
        }
    }

    targets
}

/// Register this node with the DNS server (non-blocking version)
/// Height is optional and only used for informational purposes
/// Returns the public IP address as seen by the DNS server.
async fn register_with_dns(
    _node_handle: NodeHandle, // Not used - we don't need to lock for DNS registration
    settings: &NodeSettings,
    height: u64,
) -> Result<String, Box<dyn std::error::Error>> {
    let dns_url = settings.dns_server_url.clone();
    let node_port = settings.p2p_port;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()?;
    let register_url = format!("{}/register", dns_url);

    let payload = serde_json::json!({
        "port": node_port,
        "version": env!("CARGO_PKG_VERSION"),
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
        Ok(result.registered_address)
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
                header.index
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

    // Request next block we need (my_height + 1)
    info!("[SYNC] Requesting block #{} from peers...", my_height + 1);
    
    // 방법: 블록 높이를 직접 요청하도록 피어들에게 알림
    // P2P 매니저를 통해 다음 블록 요청
    // (request_next_block 기능이 구현되어야 함)

    // Wait for blocks to arrive (give peers time to respond)
    // Increase timeout for larger syncs
    let max_sync_duration = Duration::from_secs(600); // 10 minutes max
    let idle_timeout = Duration::from_secs(30); // Stop if no progress for 30 seconds
    let sync_start = std::time::Instant::now();
    let mut last_height = my_height;
    let mut last_progress_time = sync_start;

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
            last_progress_time = std::time::Instant::now(); // Reset idle timer

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

        // Check for idle (no progress for 30 seconds)
        if last_progress_time.elapsed() > idle_timeout {
            info!(
                "[WARN] No sync progress for {} seconds. Current height: {} / {}",
                idle_timeout.as_secs(),
                current_height, max_peer_height
            );
            info!("[INFO] Will continue syncing in background via periodic header requests");
            break;
        }

        // Check for overall timeout (10 minutes)
        if sync_start.elapsed() > max_sync_duration {
            info!(
                "[WARN] Sync max duration reached ({} minutes). Current height: {} / {}",
                max_sync_duration.as_secs() / 60,
                current_height, max_peer_height
            );
            info!("[INFO] Proceeding with partial sync; background sync will continue");
            break;
        }
    }

    Ok(())
}

/// Wait until blockchain is completely synchronized with peers
/// This ensures mining only starts when we have all blocks
async fn wait_for_complete_sync(
    node_handle: NodeHandle,
    p2p_handle: Arc<astram_node::p2p::manager::PeerManager>,
) {
    const MAX_WAIT_DURATION: Duration = Duration::from_secs(1800); // 30 minutes max
    const CHECK_INTERVAL: Duration = Duration::from_secs(5);
    const NO_PEER_GRACE_DURATION: Duration = Duration::from_secs(30);
    
    let wait_start = std::time::Instant::now();
    let mut last_checked_height = 0u64;
    let mut last_progress_time = wait_start;
    let mut no_peer_since: Option<std::time::Instant> = None;
    
    loop {
        sleep(CHECK_INTERVAL).await;
        
        // Get current blockchain height
        let my_height = {
            let bc = node_handle.bc.lock().unwrap();
            if let Some(tip_hash) = &bc.chain_tip {
                if let Ok(Some(header)) = bc.load_header(tip_hash) {
                    header.index
                } else {
                    0
                }
            } else {
                0
            }
        };
        
        // Get max peer height
        let peer_heights = p2p_handle.get_peer_heights();
        let max_peer_height = peer_heights.values().max().copied().unwrap_or(0);

        // If there are no peers, don't block forever at startup.
        if peer_heights.is_empty() {
            if no_peer_since.is_none() {
                no_peer_since = Some(std::time::Instant::now());
                warn!(
                    "[WARN] No peers connected yet (local height: {}). Waiting up to {}s before mining.",
                    my_height,
                    NO_PEER_GRACE_DURATION.as_secs()
                );
            }

            if my_height > 0 {
                if let Some(since) = no_peer_since {
                    if since.elapsed() >= NO_PEER_GRACE_DURATION {
                        warn!(
                            "[WARN] Still no peers after {}s. Proceeding with local chain at height {}.",
                            NO_PEER_GRACE_DURATION.as_secs(),
                            my_height
                        );
                        return;
                    }
                }
            }

            if wait_start.elapsed() > MAX_WAIT_DURATION {
                warn!(
                    "[CRITICAL] Sync wait timeout after {} minutes with no peers. Local height: {}",
                    MAX_WAIT_DURATION.as_secs() / 60,
                    my_height
                );
                warn!("[WARN] Proceeding without peer sync confirmation.");
                return;
            }

            continue;
        }

        no_peer_since = None;
        
        // Check if we've caught up
        if my_height >= max_peer_height && max_peer_height > 0 {
            info!(
                "[OK] Blockchain fully synchronized! Height: {} (peers: {})",
                my_height, max_peer_height
            );
            return;
        }
        
        // Track progress
        if my_height > last_checked_height {
            info!(
                "[SYNC] Progress: {} / {} blocks",
                my_height, max_peer_height
            );
            last_checked_height = my_height;
            last_progress_time = std::time::Instant::now();
        }
        
        // Check if we've been idle for too long
        if last_progress_time.elapsed() > Duration::from_secs(120) {
            info!(
                "[WARN] No progress for 2 minutes. Current: {} / {} blocks",
                my_height, max_peer_height
            );
            if my_height > 0 {
                info!("[INFO] Proceeding with current sync state ({}% complete)",
                    if max_peer_height > 0 { my_height * 100 / max_peer_height } else { 0 }
                );
                return;
            }
        }
        
        // Check overall timeout
        if wait_start.elapsed() > MAX_WAIT_DURATION {
            warn!(
                "[CRITICAL] Sync timeout after {} minutes. Current: {} / {} blocks",
                MAX_WAIT_DURATION.as_secs() / 60,
                my_height, max_peer_height
            );
            warn!("[WARN] Proceeding with incomplete sync - mining may produce orphan blocks!");
            return;
        }
    }
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
                header.index
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
    match register_with_dns(node_handle.clone(), &settings, initial_height).await {
        Ok(registered_address) => {
            p2p_handle.set_my_public_ip(Some(registered_address.clone()));
            *node_meta.my_public_address.lock().unwrap() = Some(registered_address);
        }
        Err(e) => {
            log::error!("DNS registration failed; shutting down node: {}", e);
            std::process::exit(1);
        }
    }

    let dns_node_handle = node_handle.clone();
    let p2p_handle_dns = p2p_handle.clone();
    let node_meta_dns = node_meta.clone();
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
                    let height_opt = if let Ok(bc) = dns_node_handle.bc.try_lock() {
                        info!("[DNS] bc.try_lock() success");
                        if let Some(tip_hash) = &bc.chain_tip {
                            if let Ok(Some(header)) = bc.load_header(tip_hash) {
                                Some(header.index)
                            } else {
                                None
                            }
                        } else {
                            Some(0)  // Empty blockchain
                        }
                    } else {
                        warn!("[DNS] bc.try_lock() failed - lock contended, skipping re-registration");
                        None  // Skip re-registration instead of using 0
                    };

                    // Skip re-registration if we couldn't determine height
                    let height = match height_opt {
                        Some(h) => h,
                        None => {
                            warn!("[DNS] Skipping re-registration tick (cannot determine height)");
                            continue;  // Skip this tick
                        }
                    };

                    info!("[DNS] Height determined: {} (took {:?})", height, tick_start.elapsed());

                    // Spawn DNS registration asynchronously - never blocks mining
                    let dns_handle_clone = dns_node_handle.clone();
                    let settings_clone = settings_dns.clone();
                    let p2p_handle_clone = p2p_handle_dns.clone();
                    let node_meta_clone = node_meta_dns.clone();
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
                            Ok(Ok(registered_address)) => {
                                p2p_handle_clone.set_my_public_ip(Some(registered_address.clone()));
                                *node_meta_clone.my_public_address.lock().unwrap() = Some(registered_address);
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
        let mut initial_targets = match fetch_best_nodes_from_dns(
            node_meta_for_p2p.clone(),
            &settings_p2p,
            my_node_port,
            10,
        )
        .await
        {
            Ok(peer_addrs) => peer_addrs,
            Err(e) => {
                log::warn!("Failed to fetch best nodes from DNS: {}", e);
                Vec::new()
            }
        };

        if initial_targets.is_empty() {
            let my_public_ip = { node_meta_for_p2p.my_public_address.lock().unwrap().clone() };
            let fallback = build_fallback_peer_targets(
                &p2p_handle_for_task,
                &settings_p2p,
                my_public_ip,
                my_node_port,
                10,
            );
            if !fallback.is_empty() {
                warn!(
                    "[P2P] DNS returned no peer targets, falling back to {} bootstrap/saved peers",
                    fallback.len()
                );
                initial_targets = fallback;
            }
        }

        info!(
            "[INFO] Connecting to {} best nodes from discovery",
            initial_targets.len()
        );
        
        let my_public_ip_for_connect = { node_meta_for_p2p.my_public_address.lock().unwrap().clone() };
        
        for addr in initial_targets {
            let p2p_clone = p2p_handle_for_task.clone();
            let my_public_ip_clone = my_public_ip_for_connect.clone();
            
            // Convert to localhost if it's the same public IP (for local node discovery)
            let connection_addr = if let Some(ref my_ip) = my_public_ip_clone {
                if let Some((peer_ip, peer_port)) = addr.split_once(':') {
                    if peer_ip == my_ip {
                        let localhost_ip = get_localhost_address();
                        let localhost_addr = format!("{}:{}", localhost_ip, peer_port);
                        info!("[P2P] Converting {} to {} for local connection", addr, localhost_addr);
                        localhost_addr
                    } else {
                        addr.clone()
                    }
                } else {
                    addr.clone()
                }
            } else {
                addr.clone()
            };
            
            let original_addr = addr.clone();
            tokio::spawn(async move {
                if let Err(e) = p2p_clone.connect_peer(&connection_addr).await {
                    log::warn!("Failed to connect to peer {} ({}): {:?}", original_addr, connection_addr, e);
                } else {
                    info!("[OK] Connected to peer: {} (via {})", original_addr, connection_addr);
                }
            });
        }

        // Periodically refresh connections to best nodes (every 60 seconds)
        let mut interval = tokio::time::interval(Duration::from_secs(60));
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
                        Ok(mut peer_addrs) => {
                            if peer_addrs.is_empty() {
                                let my_public_ip = { node_meta_for_p2p.my_public_address.lock().unwrap().clone() };
                                let fallback = build_fallback_peer_targets(
                                    &p2p_handle_for_task,
                                    &settings_p2p,
                                    my_public_ip,
                                    my_node_port,
                                    10,
                                );
                                if !fallback.is_empty() {
                                    warn!(
                                        "[P2P] Refresh found no DNS targets, using {} fallback peers",
                                        fallback.len()
                                    );
                                    peer_addrs = fallback;
                                }
                            }

                            info!(
                                "[INFO] Refreshing connections to {} discovered peers",
                                peer_addrs.len()
                            );
                            
                            let my_public_ip_refresh = { node_meta_for_p2p.my_public_address.lock().unwrap().clone() };
                            
                            for addr in peer_addrs {
                                let p2p_clone = p2p_handle_for_task.clone();
                                let my_public_ip_clone = my_public_ip_refresh.clone();
                                
                                // Convert to localhost if it's the same public IP
                                let connection_addr = if let Some(ref my_ip) = my_public_ip_clone {
                                    if let Some((peer_ip, peer_port)) = addr.split_once(':') {
                                        if peer_ip == my_ip {
                                            let localhost_ip = get_localhost_address();
                                            format!("{}:{}", localhost_ip, peer_port)
                                        } else {
                                            addr.clone()
                                        }
                                    } else {
                                        addr.clone()
                                    }
                                } else {
                                    addr.clone()
                                };
                                
                                tokio::spawn(async move {
                                    let _ = p2p_clone.connect_peer(&connection_addr).await;
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
    
    // Wait longer for latency tests and connections to complete
    // (latency test: 3s × N peers + connection time)
    let mut wait_count = 0;
    while wait_count < 15 {  // Max 15 seconds
        sleep(Duration::from_secs(1)).await;
        wait_count += 1;
        
        // Check if we have any peers connected
        let peer_count = p2p_handle.get_peer_heights().len();
        if peer_count > 0 {
            info!("[INFO] {} peer(s) connected, proceeding...", peer_count);
            break;
        }
        
        if wait_count % 5 == 0 {
            info!("[INFO] Still waiting for peer connections... ({}/15s elapsed)", wait_count);
        }
    }

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

    // Step 5.5: Wait for complete blockchain synchronization before mining
    println!("[INFO] Step 5.5: Waiting for complete blockchain synchronization...");
    wait_for_complete_sync(node_handle.clone(), p2p_handle.clone()).await;

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
        .unwrap_or_else(|_| "cuda".to_string())
        .to_lowercase();

    if requested_backend != "cuda" {
        println!("[ERROR] Only CUDA miner backend is supported");
        std::process::exit(1);
    }

    if !cfg!(feature = "cuda-miner") {
        println!("[ERROR] CUDA miner feature not enabled. Build with --features cuda-miner");
        std::process::exit(1);
    }

    println!("[INFO] Using CUDA miner backend");

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

        // During synchronization, check if we're catching up with peers
        // If yes, skip mining to allow blocks to be processed without lock contention
        let peer_heights = p2p_handle.get_peer_heights();
        let my_height = {
            if let Ok(bc) = node_handle.bc.try_lock() {
                if let Some(tip_hash) = &bc.chain_tip {
                    if let Ok(Some(header)) = bc.load_header(tip_hash) {
                        Some(header.index)
                    } else {
                        None
                    }
                } else {
                    Some(0)
                }
            } else {
                None // Can't acquire lock, skip mining this round
            }
        };
        
        // Skip mining during sync if we're more than 5 blocks behind
        if let (Some(my_h), Some(&peer_h)) = (my_height, peer_heights.values().max()) {
            if peer_h > my_h + 5 {
                info!("[MINING] Skipping mining during sync (local: {}, peer max: {})", my_h, peer_h);
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            }
        }

        // Snapshot pending txs + mining params while holding the lock briefly
        println!("[DEBUG] Mining: Attempting to acquire WRITE lock...");
        let (snapshot_txs, difficulty, prev_hash, index_snapshot, cancel_flag, hashrate_shared) = {
            println!("[DEBUG] Mining: WRITE lock acquired");
            let lock_acq_time = std::time::Instant::now();
            info!("[LOCK-DEBUG] 🔒 Mining: attempting bc.lock()...");

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
                let lock_start = std::time::Instant::now();
                info!("[LOCK-DEBUG] ⏳ Mining: attempting bc.lock() for header read...");
                let mut bc = node_handle.bc.lock().unwrap();
                info!("[LOCK-DEBUG] ✅ Mining: acquired bc.lock() after {:?}", lock_start.elapsed());

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

                let lock_release_time = std::time::Instant::now();
                info!("[LOCK-DEBUG] ⏳ Mining: releasing bc.lock()...");
                // bc goes out of scope here - lock released
                
                info!("[LOCK-DEBUG] ✅ Mining: released bc.lock() after {:?}", lock_release_time.elapsed());

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
        let base_reward = current_block_reward_snapshot(index_snapshot);
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
        println!("[DEBUG] ⏳ Spawning mining task on blocking thread pool...");
        let mined_block_res: anyhow::Result<Block> = tokio::task::spawn_blocking(move || {
            consensus::mine_block_with_coinbase_cuda(
                index_local,
                prev_hash,
                difficulty_local,
                txs_cloned,
                &miner_addr_cloned,
                coinbase_reward,
                cancel_for_thread,
                Some(hashrate_for_thread),
            )
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
                let lock_insert_time = std::time::Instant::now();
                info!("[LOCK-DEBUG] 🔒 Mining: attempting bc.lock() for block insert...");
                let insert_result = {
                    let lock_acq_insert = std::time::Instant::now();
                    let mut bc_insert = node_handle
                        .bc
                        .lock()
                        .unwrap();
                    info!("[LOCK-DEBUG] ✅ Mining: acquired bc.lock() for insert after {:?}", lock_acq_insert.elapsed());
                    bc_insert.validate_and_insert_block(&block)
                    // bc_insert lock released here
                };
                info!("[LOCK-DEBUG] ✅ Mining: released bc.lock() after insert");
                
                match insert_result
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
                            // difficulty_local is compact bits, not an exponent.
                            // Approximate attempts by converted leading-zero hardness: 16^z = 2^(4z).
                            let leading_zeros =
                                consensus::compact_to_leading_zeros(difficulty_local);
                            let estimated_hashes = 16_f64.powi(leading_zeros as i32);
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

fn current_block_reward_snapshot(block_height:u64) -> U256 {
    // For now, always return initial reward (genesis/early blocks)
    // In production, this would take current blockchain height as parameter
    //initial_block_reward()
    calculate_block_reward(block_height)
}
