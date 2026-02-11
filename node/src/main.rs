// Use library exports instead of declaring local modules to avoid duplicate crate types
use Astram_config::config::Config;
use Astram_core::Blockchain;
use Astram_core::block;
use Astram_core::block::{Block, BlockHeader, compute_header_hash, compute_merkle_root};
use Astram_core::config::{calculate_block_reward, initial_block_reward};
use Astram_core::consensus;
use Astram_core::transaction::{BINCODE_CONFIG, Transaction};
use Astram_core::utxo::Utxo;
use Astram_node::NodeHandle;
use Astram_node::NodeState;
use Astram_node::p2p::service::P2PService;
use Astram_node::server::run_server;
use chrono::Utc;
use hex;
use log::info;
use primitive_types::U256;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering as OtherOrdering;
use std::sync::{Arc, Mutex};
use tokio::signal;
use tokio::time::{Duration, sleep};

#[derive(Debug, Clone, Deserialize)]
struct DnsNodeInfo {
    address: String,
    port: u16,
    version: String,
    height: u64,
    last_seen: i64,
    first_seen: i64,
    uptime_hours: f64,
}

#[derive(Debug, Deserialize)]
struct DnsNodesResponse {
    nodes: Vec<DnsNodeInfo>,
    count: usize,
}

#[tokio::main]
async fn main() {
    println!("[INFO] Astram node starting...");

    env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Debug)
        .init();

    let cfg = Config::load();

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
    let db_path = cfg.data_dir.clone();

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

    // Initialize P2P networking
    let p2p_service = P2PService::new();

    let mining_cancel_flag = Arc::new(AtomicBool::new(false));

    let node = NodeState {
        bc,
        blockchain: vec![],
        pending: vec![],
        seen_tx: HashMap::new(),
        p2p: p2p_service.manager(),
        eth_to_Astram_tx: HashMap::new(),
        mining_cancel_flag: mining_cancel_flag.clone(),
        orphan_blocks: HashMap::new(),
        mining_active: Arc::new(AtomicBool::new(false)),
        current_difficulty: Arc::new(Mutex::new(1)),
        current_hashrate: Arc::new(Mutex::new(0.0)),
        blocks_mined: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        node_start_time: std::time::Instant::now(),
        miner_address: Arc::new(Mutex::new(miner_address.clone())),
        recently_mined_blocks: HashMap::new(),
        my_public_address: Arc::new(Mutex::new(None)),
    };

    let node_handle = Arc::new(Mutex::new(node));

    // Set current blockchain height in P2P manager
    let my_height = {
        let state = node_handle.lock().unwrap();
        if let Some(tip_hash) = &state.bc.chain_tip {
            if let Ok(Some(header)) = state.bc.load_header(tip_hash) {
                header.index + 1
            } else {
                0
            }
        } else {
            0
        }
    };
    p2p_service.manager().set_my_height(my_height);
    info!("[INFO] Local blockchain height set to: {}", my_height);

    // Get listening port from environment or use default
    let node_port_str = std::env::var("NODE_PORT").unwrap_or_else(|_| "8335".to_string());
    let node_port: u16 = node_port_str.parse().unwrap_or(8335);
    let bind_addr = format!("0.0.0.0:{}", node_port);

    // Set listening port in P2P manager (for self-connection detection)
    p2p_service.manager().set_my_listening_port(node_port);

    p2p_service
        .start(bind_addr, node_handle.clone())
        .await
        .expect("p2p start failed");

    // Start Ethereum JSON-RPC server for MetaMask
    let eth_rpc_node = node_handle.clone();
    tokio::spawn(async move {
        Astram_node::server::run_eth_rpc_server(eth_rpc_node).await;
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
                shutdown_flag_clone.store(true, OtherOrdering::SeqCst);

                // Cancel ongoing mining immediately
                let state = node_for_shutdown.lock().unwrap();
                state.mining_cancel_flag.store(true, OtherOrdering::SeqCst);
                println!("[WARN] Mining cancellation requested...");
            }
            Err(err) => {
                eprintln!("Error setting up signal handler: {}", err);
            }
        }
    });

    let (task_handles, server_handle) =
        start_services(node_handle.clone(), miner_address, shutdown_flag.clone()).await;

    // Wait for all background tasks to complete
    println!("[INFO] Waiting for all tasks to complete...");
    for handle in task_handles {
        let _ = handle.await;
    }

    // Abort HTTP server (it runs indefinitely)
    server_handle.abort();
    println!("[INFO] HTTP server stopped");

    // Give more time for all resources to be released
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Cleanup: Close database properly
    {
        println!("[INFO] Closing database...");

        // Check Arc reference count
        let arc_count = Arc::strong_count(&node_handle);
        println!("[INFO] Arc strong references remaining: {}", arc_count);

        // First, try to flush the DB while we still have a reference
        {
            if let Ok(mut state) = node_handle.lock() {
                // Flush WAL and compact
                if let Err(e) = state.bc.db.flush() {
                    log::warn!("Failed to flush DB: {}", e);
                } else {
                    println!("[OK] Database flushed");
                }

                // Cancel IO operations
                state.bc.db.cancel_all_background_work(true);
                println!("[INFO] Background work cancelled");
            }
        }

        // Give DB time to complete all operations
        tokio::time::sleep(Duration::from_secs(1)).await;

        // Drop the node_handle to release our reference
        drop(node_handle);

        println!("[INFO] All references released");
    }

    // Final wait to ensure LOCK file is released by OS
    tokio::time::sleep(Duration::from_secs(1)).await;

    println!("\n[OK] Astram node stopped gracefully");

    // Force process exit to ensure all resources are released
    std::process::exit(0);
    /*
    // Get current blockchain height from DB and set it in P2P manager
    let my_height: u64 = if let Some(tip_hash) = &bc.chain_tip {
        if let Ok(Some(header)) = bc.load_header(tip_hash) {
            header.index + 1
        } else {
            0
        }
    } else {
        0
    };
    p2p.set_my_height(my_height);
    println!("Local blockchain height: {}", my_height);

    /*
    {
        let p2p_clone = p2p.clone();
        p2p_clone.set_on_block(|block: block::Block| {
            tokio::spawn(async move {
                match Astram_core::consensus::validate_and_add_block(block).await {
                    Ok(_) => info!("Block added via p2p"),
                    Err(e) => log::warn!("Received invalid block from p2p: {:?}", e),
                }
            });
        });
    }
    */

    let p2p_clone = p2p.clone();
    tokio::spawn(async move {
        if let Err(e) = p2p_clone.start_listener("0.0.0.0:8335").await {
            log::error!("P2P listener failed: {:?}", e);
        }
    });

    let p2p_clone = p2p.clone();
    let dns_list = p2p_clone.dns_seed_lookup().await.unwrap_or_default();

    let saved_list = p2p_clone.load_saved_peers();
    let mut peers: HashSet<String> = HashSet::new();

    for addr in dns_list {
        peers.insert(addr);
    }

    for sp in saved_list {
        peers.insert(sp.addr);
    }

    let target_peers: Vec<String> = peers.into_iter().take(MAX_OUTBOUND).collect();

    for addr in target_peers {
        let p2p_clone = p2p.clone();
        tokio::spawn(async move {
            if let Err(e) = p2p_clone.connect_peer(&addr).await {
                log::warn!("Failed connect {}: {:?}", addr, e);
            }
        });
    }

    // TODO: Implement blockchain sync logic.
    // Compare local DB height with peers and request missing blocks.

    // If chain is empty (no tip), create genesis from wallet address
    // Reuse the already loaded miner address

    // If DB has no tip, create genesis block
    if bc.chain_tip.is_none() {
        println!("No chain tip found - creating genesis block...");
        let genesis_hash = bc
            .create_genesis(&miner_address)
            .expect("create_genesis failed");
        // load genesis header & tx to in-memory chain view
        if let Ok(Some(header)) = bc.load_header(&genesis_hash) {
            // need block transactions loaded too -> load txs by scanning index i:0
            // Simplify: construct block from header and coinbase tx from DB
            // Try to load coinbase tx via stored tx key (i:0 -> hash -> t:<txid>)
            // For simplicity, we will append a minimal block header-only view.
            let block = Block {
                header,
                transactions: vec![], // empty details (can be expanded)
                hash: genesis_hash.clone(),
            };
            // Build NodeState with this genesis header
            let node = NodeState {
                bc,
                blockchain: vec![block],
                pending: vec![],
                seen_tx: HashMap::new(),
                p2p: p2p.clone(),
                eth_to_Astram_tx: HashMap::new(),
                mining_cancel_flag: mining_cancel_flag.clone(),
                orphan_blocks: HashMap::new(),
                mining_active: Arc::new(AtomicBool::new(false)),
                current_difficulty: Arc::new(Mutex::new(1)),
                current_hashrate: Arc::new(Mutex::new(0.0)),
                blocks_mined: Arc::new(std::sync::atomic::AtomicU64::new(0)),
                node_start_time: std::time::Instant::now(),
                miner_address: Arc::new(Mutex::new(miner_address.clone())),
                recently_mined_blocks: HashMap::new(),
                my_public_address: Arc::new(Mutex::new(None)),
            };
            let node_handle = Arc::new(Mutex::new(node));

            // set p2p handlers (headers provider + block handler) and periodic sync
            {
                let p2p_clone = p2p.clone();
                let nh = node_handle.clone();
                p2p_clone.set_on_getheaders(
                    move |locator_hashes: Vec<Vec<u8>>, _stop_hash: Option<Vec<u8>>| {
                        let state = nh.lock().unwrap();
                        let mut headers: Vec<BlockHeader> = state
                            .blockchain
                            .iter()
                            .rev()
                            .take(200)
                            .map(|b| b.header.clone())
                            .collect();
                        headers.reverse();
                        headers
                    },
                );

                let p2p_clone2 = p2p.clone();
                let nh2 = node_handle.clone();
                p2p_clone2.set_on_block(move |block: block::Block| {
                    let nh_async = nh2.clone();
                    tokio::spawn(async move {
                        let mut state = nh_async.lock().unwrap();
                        match state.bc.validate_and_insert_block(&block) {
                            Ok(_) => {
                                info!("Block added via p2p");
                                state.blockchain.push(block);
                                state.enforce_memory_limit(); // Security: Enforce memory limit
                            }
                            Err(e) => log::warn!("Received invalid block from p2p: {:?}", e),
                        }
                    });
                });

                // spawn periodic header sync
                let p2p_sync = p2p.clone();
                let nh_sync = node_handle.clone();
                tokio::spawn(async move {
                    loop {
                        // build locator from last N in-memory headers
                        let mut locator: Vec<Vec<u8>> = Vec::new();
                        {
                            let state = nh_sync.lock().unwrap();
                            for b in state.blockchain.iter().rev().take(10) {
                                if let Ok(bytes) = hex::decode(&b.hash) {
                                    locator.push(bytes);
                                }
                            }
                        }
                        p2p_sync.request_headers_from_peers(locator, None);
                        sleep(Duration::from_secs(15)).await;
                    }
                });
            }
            start_services(node_handle, miner_address).await;
            return;
        }
    }

    // Otherwise, we have an existing chain tip. For simplicity, we won't reconstruct full chain here.
    // We'll create NodeState with empty in-memory chain but with bc loaded.
    let node = NodeState {
        bc,
        blockchain: vec![],
        pending: vec![],
        seen_tx: HashMap::new(),
        p2p: p2p.clone(),
        eth_to_Astram_tx: HashMap::new(),
        mining_cancel_flag: mining_cancel_flag.clone(),
        orphan_blocks: HashMap::new(),
        mining_active: Arc::new(AtomicBool::new(false)),
        current_difficulty: Arc::new(Mutex::new(1)),
        current_hashrate: Arc::new(Mutex::new(0.0)),
        blocks_mined: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        node_start_time: std::time::Instant::now(),
        miner_address: Arc::new(Mutex::new(miner_address.clone())),
        recently_mined_blocks: HashMap::new(),
        my_public_address: Arc::new(Mutex::new(None)),
    };
    let node_handle = Arc::new(Mutex::new(node));

    // set p2p handlers and periodic sync (for non-genesis startup)
    {
        let p2p_clone = p2p.clone();
        let nh = node_handle.clone();
        p2p_clone.set_on_getheaders(
            move |locator_hashes: Vec<Vec<u8>>, _stop_hash: Option<Vec<u8>>| {
                let state = nh.lock().unwrap();
                let mut headers: Vec<BlockHeader> = state
                    .blockchain
                    .iter()
                    .rev()
                    .take(200)
                    .map(|b| b.header.clone())
                    .collect();
                headers.reverse();
                headers
            },
        );

        let p2p_clone2 = p2p.clone();
        let nh2 = node_handle.clone();
        p2p_clone2.set_on_block(move |block: block::Block| {
            let nh_async = nh2.clone();
            tokio::spawn(async move {
                let mut state = nh_async.lock().unwrap();
                match state.bc.validate_and_insert_block(&block) {
                    Ok(_) => {
                        info!("Block added via p2p");
                        state.blockchain.push(block);
                        state.enforce_memory_limit(); // Security: Enforce memory limit
                    }
                    Err(e) => log::warn!("Received invalid block from p2p: {:?}", e),
                }
            });
        });

        let p2p_sync = p2p.clone();
        let nh_sync = node_handle.clone();
        tokio::spawn(async move {
            loop {
                let mut locator: Vec<Vec<u8>> = Vec::new();
                {
                    let state = nh_sync.lock().unwrap();
                    for b in state.blockchain.iter().rev().take(10) {
                        if let Ok(bytes) = hex::decode(&b.hash) {
                            locator.push(bytes);
                        }
                    }
                }
                p2p_sync.request_headers_from_peers(locator, None);
                sleep(Duration::from_secs(15)).await;
            }
        });
    }

    start_services(node_handle.clone(), miner_address).await;
    */
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
    node_handle: NodeHandle,
    my_port: u16,
    limit: usize,
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    // Get my public address from state
    let my_address = {
        let state = node_handle.lock().unwrap();
        state.my_public_address.lock().unwrap().clone()
    };

    let dns_url =
        std::env::var("DNS_SERVER_URL").unwrap_or_else(|_| "http://161.33.19.183:8053".to_string());
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

/// Register this node with the DNS server
async fn register_with_dns(node_handle: NodeHandle) -> Result<(), Box<dyn std::error::Error>> {
    let dns_url =
        std::env::var("DNS_SERVER_URL").unwrap_or_else(|_| "http://161.33.19.183:8053".to_string());
    // DNS server will automatically detect the IP address from the connection
    let node_port = std::env::var("NODE_PORT").unwrap_or_else(|_| "8335".to_string());

    let height = {
        let state = node_handle.lock().unwrap();
        if let Some(tip_hash) = &state.bc.chain_tip {
            if let Ok(Some(header)) = state.bc.load_header(tip_hash) {
                header.index + 1
            } else {
                0
            }
        } else {
            0
        }
    };

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5)) // 5 second timeout
        .build()?;
    let register_url = format!("{}/register", dns_url);

    // address field is now optional - DNS server will detect it from the connection
    let payload = serde_json::json!({
        "port": node_port.parse::<u16>().unwrap_or(8335),
        "version": "0.1.0",
        "height": height
    });

    info!("Registering node with DNS server at {}", dns_url);

    let response = client.post(&register_url).json(&payload).send().await?;

    if response.status().is_success() {
        #[derive(serde::Deserialize)]
        struct RegisterResponse {
            success: bool,
            message: String,
            node_count: usize,
            registered_address: String,
            registered_port: u16,
        }

        let result: RegisterResponse = response.json().await?;
        info!(
            "Successfully registered with DNS server: {} ({}:{})",
            result.message, result.registered_address, result.registered_port
        );

        // Store the public IP address we were registered with
        {
            let state = node_handle.lock().unwrap();
            *state.my_public_address.lock().unwrap() = Some(result.registered_address);
        }

        Ok(())
    } else {
        let error_text = response.text().await?;
        Err(format!("Failed to register with DNS server: {}", error_text).into())
    }
}

/// Synchronize blockchain with peers
async fn sync_blockchain(
    node_handle: NodeHandle,
    p2p_handle: Arc<Astram_node::p2p::manager::PeerManager>,
) -> Result<(), Box<dyn std::error::Error>> {
    info!("[INFO] Starting blockchain synchronization...");

    let my_height = {
        let state = node_handle.lock().unwrap();
        if let Some(tip_hash) = &state.bc.chain_tip {
            if let Ok(Some(header)) = state.bc.load_header(tip_hash) {
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
            let state = node_handle.lock().unwrap();
            if let Some(tip_hash) = &state.bc.chain_tip {
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
    let sync_timeout = Duration::from_secs(60);
    let sync_start = std::time::Instant::now();
    let mut last_height = my_height;

    loop {
        sleep(Duration::from_secs(2)).await;

        let current_height = {
            let state = node_handle.lock().unwrap();
            if let Some(tip_hash) = &state.bc.chain_tip {
                if let Ok(Some(header)) = state.bc.load_header(tip_hash) {
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
                    let state = node_handle.lock().unwrap();
                    if let Some(tip_hash) = &state.bc.chain_tip {
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
    miner_address: String,
    shutdown_flag: Arc<AtomicBool>,
) -> (
    Vec<tokio::task::JoinHandle<()>>,
    tokio::task::JoinHandle<()>,
) {
    println!("[INFO] my address {}", miner_address);

    let mut task_handles = Vec::new();

    let my_node_address = std::env::var("NODE_ADDRESS").unwrap_or_else(|_| "127.0.0.1".to_string());
    let my_node_port = std::env::var("NODE_PORT")
        .unwrap_or_else(|_| "8335".to_string())
        .parse::<u16>()
        .unwrap_or(8335);

    // Register with DNS server (fail fast if registration fails)
    if let Err(e) = register_with_dns(node_handle.clone()).await {
        log::error!("DNS registration failed; shutting down node: {}", e);
        std::process::exit(1);
    }

    let dns_node_handle = node_handle.clone();
    let shutdown_flag_dns = shutdown_flag.clone();
    let dns_task = tokio::spawn(async move {
        // Re-register every 5 minutes to keep the node alive in DNS
        let mut interval = tokio::time::interval(Duration::from_secs(300));
        interval.tick().await; // Skip first immediate tick

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    if shutdown_flag_dns.load(OtherOrdering::SeqCst) {
                        info!("DNS registration task shutting down...");
                        break;
                    }
                    if let Err(e) = register_with_dns(dns_node_handle.clone()).await {
                        log::warn!("Failed to re-register with DNS server: {}", e);
                    }
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
    let p2p_handle = {
        let state = node_handle.lock().unwrap();
        state.p2p.clone()
    };

    let my_addr_clone = my_node_address.clone();
    let shutdown_flag_p2p = shutdown_flag.clone();
    let p2p_handle_for_task = p2p_handle.clone();
    let node_handle_for_p2p = node_handle.clone();
    let p2p_task = tokio::spawn(async move {
        // Wait a bit for DNS registration to complete
        sleep(Duration::from_secs(2)).await;

        // Initial connection to best nodes
        match fetch_best_nodes_from_dns(node_handle_for_p2p.clone(), my_node_port, 10).await {
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
                    match fetch_best_nodes_from_dns(node_handle_for_p2p.clone(), my_node_port, 10).await {
                Ok(peer_addrs) => {
                    info!(
                        "[INFO] Refreshing connections to {} best nodes",
                        peer_addrs.len()
                    );
                    for addr in peer_addrs {
                        let p2p_clone = p2p_handle_for_task.clone();
                        let addr_clone = addr.clone();
                        tokio::spawn(async move {
                            if let Err(e) = p2p_clone.connect_peer(&addr_clone).await {
                                log::debug!(
                                    "Peer connection refresh failed for {}: {:?}",
                                    addr_clone,
                                    e
                                );
                            }
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

    let nh: Arc<Mutex<NodeState>> = node_handle.clone();
    // start HTTP server in background thread (warp is async so run in tokio)
    let server_handle = tokio::spawn(async move {
        run_server(nh).await;
    });

    // Step 6: Start mining
    println!("[INFO] Step 6: Starting mining...");

    // Mining loop - run in main task, not spawned
    mining_loop(node_handle.clone(), miner_address, shutdown_flag.clone()).await;

    // Return background tasks, but not server (we'll abort it)
    (task_handles, server_handle)
}

async fn mining_loop(
    node_handle: NodeHandle,
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
            let state = node_handle.lock().unwrap();
            state.mining_cancel_flag.store(true, OtherOrdering::SeqCst);
            break;
        }

        // Snapshot pending txs + mining params while holding the lock briefly
        let (
            snapshot_txs,
            difficulty,
            prev_hash,
            index_snapshot,
            p2p_handle,
            total_fees,
            cancel_flag,
            hashrate_shared,
        ) = {
            let mut state = node_handle.lock().unwrap();

            // Mark mining as active
            state.mining_active.store(true, OtherOrdering::SeqCst);

            // Reset cancel flag at the start of each mining round
            state.mining_cancel_flag.store(false, OtherOrdering::SeqCst);

            // clone pending transactions to work on them outside the lock
            let txs_copy = state.pending.clone();

            // previous tip hash
            let prev_hash = state.bc.chain_tip.clone().unwrap_or_else(|| "0".repeat(64));

            // determine next index from tip header (so header.index is known before mining)
            let mut next_index: u64 = 0;
            if let Some(tip_hash) = state.bc.chain_tip.clone() {
                if let Ok(Some(prev_header)) = state.bc.load_header(&tip_hash) {
                    next_index = prev_header.index + 1;
                } else {
                    next_index = 0;
                }
            } else {
                next_index = 0;
            }

            // Calculate difficulty for the next block (dynamic adjustment every 30 blocks)
            let diff = state
                .bc
                .calculate_adjusted_difficulty(next_index)
                .unwrap_or(state.bc.difficulty);

            if diff != state.bc.difficulty {
                println!(
                    "[INFO] Difficulty adjusted: {} -> {} (block #{})",
                    state.bc.difficulty, diff, next_index
                );
                // Update blockchain difficulty before mining
                state.bc.difficulty = diff;
            }

            // Update current difficulty in state
            *state.current_difficulty.lock().unwrap() = diff;

            // Calculate total fees from pending transactions
            let mut fee_sum = U256::zero();
            for tx in &txs_copy {
                // Calculate fee: input_sum - output_sum
                let mut input_sum = U256::zero();
                let mut output_sum = U256::zero();

                // Sum inputs (from UTXO)
                for inp in &tx.inputs {
                    let ukey = format!("u:{}:{}", inp.txid, inp.vout);
                    if let Ok(Some(blob)) = state.bc.db.get(ukey.as_bytes()) {
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

            // clear pending locally ??we'll requeue on failure
            state.pending.clear();

            (
                txs_copy,
                diff,
                prev_hash,
                next_index,
                state.p2p.clone(),
                fee_sum,
                state.mining_cancel_flag.clone(),
                state.current_hashrate.clone(),
            )
        };

        // prepare block transactions: coinbase + pending
        // NOTE: we pass pending txs to consensus::mine_block_with_coinbase which will prepend coinbase
        let block_txs_for_logging = snapshot_txs.len();
        println!("[INFO] Mining {} pending tx(s)...", block_txs_for_logging);

        // Coinbase reward = block reward + total fees
        let base_reward = current_block_reward_snapshot();
        let coinbase_reward = base_reward + total_fees;

        if total_fees > U256::zero() {
            let fees_ASRM = total_fees / U256::from(1_000_000_000_000_000_000u64);
            println!(
                "[INFO] Total fees in block: {} wei ({} ASRM)",
                total_fees, fees_ASRM
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

            consensus::mine_block_with_coinbase(
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

        match mined_block_res {
            Ok(block) => {
                // Re-acquire lock to insert block atomically
                let mut state = node_handle.lock().unwrap();

                // Note: We do NOT modify the mined block's timestamp or hash
                // because that would invalidate the PoW nonce that was just found.
                // The block is already valid as-is from mining.

                match state.bc.validate_and_insert_block(&block) {
                    Ok(_) => {
                        println!(
                            "[OK] Mined new block index={} hash={}",
                            block.header.index, block.hash
                        );

                        // Update mining statistics
                        state
                            .blocks_mined
                            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

                        // Calculate hashrate (rough estimate)
                        let mining_duration = mining_start.elapsed().as_secs_f64();
                        if mining_duration > 0.0 {
                            // Estimate: 2^difficulty hashes attempted in mining_duration seconds
                            let estimated_hashes = 2_u64.pow(difficulty_local) as f64;
                            let hashrate = estimated_hashes / mining_duration;
                            *state.current_hashrate.lock().unwrap() = hashrate;
                        }

                        let block_to_broadcast = block.clone();

                        state.blockchain.push(block.clone());
                        state.enforce_memory_limit(); // Security: Enforce memory limit
                        // pending already cleared earlier

                        // Update P2P manager height
                        state.p2p.set_my_height(block.header.index + 1);

                        // Track this block as recently mined (to ignore when received from peers)
                        let now = chrono::Utc::now().timestamp();
                        state.recently_mined_blocks.insert(block.hash.clone(), now);

                        // Clean up old entries (older than 5 minutes)
                        state
                            .recently_mined_blocks
                            .retain(|_, &mut timestamp| now - timestamp < 300);

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
                        for tx in block.transactions.into_iter().skip(1) {
                            state.pending.push(tx);
                        }
                        // Security: Enforce mempool limits
                        state.enforce_mempool_limit();
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
                let mut state = node_handle.lock().unwrap();
                state.mining_active.store(false, OtherOrdering::SeqCst);
                *state.current_hashrate.lock().unwrap() = 0.0;

                // Only requeue txs if it wasn't a cancellation
                if !error_msg.contains("cancelled") && !error_msg.contains("Mining cancelled") {
                    for tx in snapshot_txs.into_iter() {
                        state.pending.push(tx);
                    }
                    // Security: Enforce mempool limits
                    state.enforce_mempool_limit();
                }
            }
        }

        // Wait before next cycle, but check shutdown flag frequently for quick response
        for _ in 0..10 {
            if shutdown_flag.load(OtherOrdering::SeqCst) {
                info!("[WARN] Shutdown detected during sleep, exiting mining loop");
                return;
            }
            sleep(Duration::from_secs(1)).await;
        }
    }

    // server_handle.await.unwrap(); // unreachable because loop is infinite
}

// Constants for ASRM token economics
const HALVING_INTERVAL: u64 = 210_000; // blocks (approx 4 years at ~10 min/block)

fn current_block_reward_snapshot() -> U256 {
    // For now, always return initial reward (genesis/early blocks)
    // In production, this would take current blockchain height as parameter
    initial_block_reward()
}
