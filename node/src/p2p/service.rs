// node/src/p2p/service.rs
use crate::NodeHandle;
use crate::p2p::manager::{MAX_OUTBOUND, PeerManager};
use hex;
use log::{info, warn};
use netcoin_core::block;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::time::{Duration, sleep};

pub struct P2PService {
    pub manager: Arc<PeerManager>,
}

impl P2PService {
    pub fn new() -> Self {
        Self {
            manager: Arc::new(PeerManager::new()),
        }
    }

    pub fn manager(&self) -> Arc<PeerManager> {
        self.manager.clone()
    }

    pub async fn start(&self, bind_addr: String, node_handle: NodeHandle) -> anyhow::Result<()> {
        self.start_listener(bind_addr).await;
        self.connect_initial_peers().await;
        self.register_handlers(node_handle.clone());
        self.start_header_sync(node_handle.clone());

        Ok(())
    }

    async fn start_listener(&self, addr: String) {
        let p2p = self.manager.clone();

        tokio::spawn(async move {
            if let Err(e) = p2p.start_listener(&addr).await {
                log::error!("P2P listener failed: {:?}", e);
            }
        });
    }

    async fn connect_initial_peers(&self) {
        let p2p = self.manager.clone();

        let dns_list = p2p.dns_seed_lookup().await.unwrap_or_default();
        let saved_list = p2p.load_saved_peers();

        let mut peers = HashSet::new();
        for addr in dns_list {
            peers.insert(addr);
        }
        for sp in saved_list {
            peers.insert(sp.addr);
        }

        for addr in peers.into_iter().take(MAX_OUTBOUND) {
            let p2p_clone = p2p.clone();
            tokio::spawn(async move {
                if let Err(e) = p2p_clone.connect_peer(&addr).await {
                    warn!("Failed connect {}: {:?}", addr, e);
                }
            });
        }
    }

    fn register_handlers(&self, node_handle: NodeHandle) {
        let p2p = self.manager.clone();

        // getheaders handler - load headers from DB
        let nh = node_handle.clone();
        p2p.set_on_getheaders(move |locator_hashes, _stop_hash| {
            let state = nh.lock().unwrap();
            let mut headers = Vec::new();

            // Get chain tip
            let tip_hash = match &state.bc.chain_tip {
                Some(h) => h.clone(),
                None => return headers,
            };

            // Build full chain from tip backwards
            let mut chain = Vec::new();
            let mut current_hash = Some(tip_hash);
            
            while let Some(hash) = current_hash {
                if let Ok(Some(header)) = state.bc.load_header(&hash) {
                    chain.push(header.clone());
                    if header.index == 0 {
                        break;
                    }
                    current_hash = Some(header.previous_hash.clone());
                } else {
                    break;
                }
            }
            
            // Reverse to get genesis-first order
            chain.reverse();

            // Determine starting point
            let start_index = if locator_hashes.is_empty() {
                // No locator - start from genesis
                0
            } else {
                // Find first matching locator
                let mut found_index = 0;
                for loc_hash in &locator_hashes {
                    if let Ok(hash_hex) = hex::encode(loc_hash).parse::<String>() {
                        if let Some(pos) = chain.iter().position(|h| {
                            if let Ok(computed) = netcoin_core::block::compute_header_hash(h) {
                                computed == hash_hex
                            } else {
                                false
                            }
                        }) {
                            found_index = pos + 1; // Start from next block
                            break;
                        }
                    }
                }
                found_index
            };

            // Return up to 200 headers starting from start_index
            headers = chain.into_iter()
                .skip(start_index)
                .take(200)
                .collect();

            headers
        });

        // block handler
        let nh2 = node_handle.clone();
        p2p.set_on_block(move |block: block::Block| {
            let nh_async = nh2.clone();
            tokio::spawn(async move {
                let mut state = nh_async.lock().unwrap();

                // Check if this is a block we recently mined ourselves
                if state.recently_mined_blocks.contains_key(&block.hash) {
                    info!(
                        "🔁 Ignoring block we mined ourselves: index={} hash={}",
                        block.header.index, block.hash
                    );
                    return;
                }

                // Cancel ongoing mining when receiving a new block
                state
                    .mining_cancel_flag
                    .store(true, std::sync::atomic::Ordering::SeqCst);

                // Try to insert the block
                match state.bc.validate_and_insert_block(&block) {
                    Ok(_) => {
                        info!(
                            "✅ Block added via p2p: index={} hash={}",
                            block.header.index, block.hash
                        );
                        state.blockchain.push(block.clone());
                        state.enforce_memory_limit(); // 🔒 Security: Enforce memory limit

                        // Update P2P manager height
                        state.p2p.set_my_height(block.header.index + 1);

                        // Remove transactions from pending pool that are in the new block
                        let block_txids: std::collections::HashSet<String> = block
                            .transactions
                            .iter()
                            .map(|tx| tx.txid.clone())
                            .collect();

                        let removed_count = block_txids.len().saturating_sub(1); // -1 for coinbase
                        state.pending.retain(|tx| !block_txids.contains(&tx.txid));

                        if removed_count > 0 {
                            info!(
                                "🗑️  Removed {} transactions from mempool (included in peer block)",
                                removed_count
                            );
                        }

                        // Check if this block triggers a chain reorganization
                        match state.bc.reorganize_if_needed(&block.hash) {
                            Ok(true) => {
                                info!("🔄 Chain reorganization completed");
                            }
                            Ok(false) => {
                                // No reorg needed, current chain is best
                            }
                            Err(e) => {
                                warn!("⚠️  Reorganization check failed: {:?}", e);
                            }
                        }

                        // Try to process orphan blocks that may now be valid
                        Self::process_orphan_blocks(&mut state);

                        info!("⛏️  Mining cancelled, restarting with updated chain...");
                    }
                    Err(e) => {
                        // Block validation failed - check if it's an orphan
                        let error_msg = format!("{:?}", e);
                        
                        if error_msg.contains("previous header not found") {
                            // 🔒 Security: Check orphan pool size limit before adding
                            let now = chrono::Utc::now().timestamp();
                            
                            if state.orphan_blocks.len() >= crate::MAX_ORPHAN_BLOCKS {
                                warn!(
                                    "⚠️  Orphan pool full ({} blocks), dropping oldest orphan to accept new one",
                                    state.orphan_blocks.len()
                                );
                                
                                // Find and remove oldest orphan
                                let oldest_hash = state.orphan_blocks
                                    .iter()
                                    .min_by_key(|(_, (_, timestamp))| *timestamp)
                                    .map(|(h, _)| h.clone());
                                
                                if let Some(hash) = oldest_hash {
                                    state.orphan_blocks.remove(&hash);
                                }
                            }
                            
                            // Clean up expired orphans (older than 30 minutes)
                            state.orphan_blocks.retain(|_, (_, timestamp)| {
                                now - *timestamp < crate::ORPHAN_TIMEOUT
                            });
                            
                            state.orphan_blocks.insert(block.hash.clone(), (block.clone(), now));
                            
                            info!(
                                "📦 Orphan block received (index={}, hash={}), storing for later (orphan pool size: {})",
                                block.header.index,
                                &block.hash[..16],
                                state.orphan_blocks.len()
                            );
                            
                            // Request the parent block
                            // TODO: implement getdata request for parent block
                        } else {
                            warn!("❌ Invalid block from p2p: {:?}", e);
                        }
                    }
                }
            });
        });

        // transaction handler
        let nh3 = node_handle.clone();
        let p2p_for_tx = p2p.clone();
        p2p.set_on_tx(move |tx: netcoin_core::transaction::Transaction| {
            let nh_async = nh3.clone();
            let p2p_tx_relay = p2p_for_tx.clone();
            tokio::spawn(async move {
                // Check and update state in a separate scope to ensure lock is released
                let should_relay = {
                    let mut state = nh_async.lock().unwrap();

                    // Check if we've already seen this transaction (prevents loops)
                    if state.seen_tx.contains_key(&tx.txid) {
                        info!("🔁 Transaction {} already seen, skipping", tx.txid);
                        return;
                    }

                    // Check if transaction already exists in pending pool
                    if state.pending.iter().any(|t| t.txid == tx.txid) {
                        info!("Transaction {} already in mempool, skipping", tx.txid);
                        // Mark as seen even if already in mempool
                        let now = chrono::Utc::now().timestamp();
                        state.seen_tx.insert(tx.txid.clone(), now);
                        return;
                    }

                    // Validate transaction signatures
                    match tx.verify_signatures() {
                        Ok(true) => {
                            info!("✅ Transaction {} received and validated from p2p", tx.txid);
                            
                            // 🔒 Security: Check for double-spending in mempool
                            let mut tx_utxos = std::collections::HashSet::new();
                            for inp in &tx.inputs {
                                tx_utxos.insert(format!("{}:{}", inp.txid, inp.vout));
                            }
                            
                            // Check for conflicts with pending transactions
                            let mut has_conflict = false;
                            for pending_tx in &state.pending {
                                for pending_inp in &pending_tx.inputs {
                                    let pending_utxo = format!("{}:{}", pending_inp.txid, pending_inp.vout);
                                    if tx_utxos.contains(&pending_utxo) {
                                        warn!(
                                            "🚫 Double-spend detected in P2P TX {}: UTXO {} already used by pending TX {}",
                                            tx.txid, pending_utxo, pending_tx.txid
                                        );
                                        has_conflict = true;
                                        break;
                                    }
                                }
                                if has_conflict {
                                    break;
                                }
                            }
                            
                            if has_conflict {
                                false // Reject this transaction
                            } else {
                                // Mark transaction as seen with timestamp
                                let now = chrono::Utc::now().timestamp();
                                state.seen_tx.insert(tx.txid.clone(), now);
                                
                                // Clean up old seen_tx entries (older than 1 hour)
                                state.seen_tx.retain(|_, &mut timestamp| {
                                    now - timestamp < 3600
                                });
                                
                                // Add to mempool
                                state.pending.push(tx.clone());
                                
                                // 🔒 Security: Enforce mempool limits after adding transaction
                                state.enforce_mempool_limit();
                                
                                info!("📝 Mempool size: {} transactions", state.pending.len());
                                
                                true // Should relay to other peers
                            }
                        }
                        Ok(false) => {
                            warn!("❌ Transaction {} has invalid signatures", tx.txid);
                            false
                        }
                        Err(e) => {
                            warn!("❌ Transaction {} validation error: {:?}", tx.txid, e);
                            false
                        }
                    }
                }; // Lock is released here
                
                // Relay transaction to other peers if validated
                if should_relay {
                    p2p_tx_relay.broadcast_tx(&tx).await;
                    info!("📡 Relayed transaction {} to other peers", tx.txid);
                }
            });
        });

        // getdata handler - send requested blocks/transactions
        let nh4 = node_handle.clone();
        let p2p_clone = p2p.clone();
        p2p.set_on_getdata(move |peer_id, object_type, hashes| {
            use crate::p2p::messages::InventoryType;
            
            let state = nh4.lock().unwrap();
            let p2p_inner = p2p_clone.clone();
            
            match object_type {
                InventoryType::Block => {
                    // Load and send requested blocks
                    for hash_bytes in hashes {
                        if let Ok(hash_hex) = hex::encode(&hash_bytes).parse::<String>() {
                            // Try to load block from DB
                            if let Ok(Some(block)) = state.bc.load_block(&hash_hex) {
                                // Send block to peer
                                let peer_id_clone = peer_id.clone();
                                let p2p_for_send = p2p_inner.clone();
                                tokio::spawn(async move {
                                    p2p_for_send.send_block_to_peer(&peer_id_clone, &block).await;
                                });
                            }
                        }
                    }
                }
                InventoryType::Transaction => {
                    // TODO: Send transactions from mempool
                }
                InventoryType::Error => {
                    // Ignore error type
                }
            }
        });
    }

    /// Process orphan blocks that may now be valid
    fn process_orphan_blocks(state: &mut crate::NodeState) {
        let mut processed_any = true;
        let max_iterations = 100; // Prevent infinite loops
        let mut iterations = 0;

        while processed_any && iterations < max_iterations {
            processed_any = false;
            iterations += 1;

            // Find orphan blocks whose parent now exists
            let orphans_to_try: Vec<_> = state
                .orphan_blocks
                .iter()
                .map(|(hash, (block, _))| (hash.clone(), block.clone()))
                .collect();

            for (hash, block) in orphans_to_try {
                // Check if parent exists now
                if let Ok(Some(_)) = state.bc.load_block(&block.header.previous_hash) {
                    // Parent exists! Try to validate and insert
                    match state.bc.validate_and_insert_block(&block) {
                        Ok(_) => {
                            info!(
                                "✅ Orphan block now valid: index={} hash={}",
                                block.header.index, &hash[..16]
                            );
                            state.blockchain.push(block.clone());
                            state.enforce_memory_limit(); // 🔒 Security: Enforce memory limit
                            state.orphan_blocks.remove(&hash);
                            processed_any = true;

                            // Update P2P manager height
                            state.p2p.set_my_height(block.header.index + 1);

                            // Remove transactions from mempool
                            let block_txids: std::collections::HashSet<String> = block
                                .transactions
                                .iter()
                                .map(|tx| tx.txid.clone())
                                .collect();
                            state.pending.retain(|tx| !block_txids.contains(&tx.txid));

                            // Check for reorganization
                            let _ = state.bc.reorganize_if_needed(&hash);
                        }
                        Err(e) => {
                            warn!(
                                "⚠️  Orphan block still invalid: index={} hash={}, error: {:?}",
                                block.header.index, &hash[..16], e
                            );
                            // Keep in orphan pool for now
                        }
                    }
                }
            }
        }

        // Clean up old orphan blocks (older than 1 hour)
        let now = chrono::Utc::now().timestamp();
        let one_hour = 3600;
        state.orphan_blocks.retain(|hash, (block, timestamp)| {
            let age = now - *timestamp;
            if age > one_hour {
                info!(
                    "🗑️  Removing old orphan block: index={} hash={} (age: {}s)",
                    block.header.index,
                    &hash[..16],
                    age
                );
                false
            } else {
                true
            }
        });

        if !state.orphan_blocks.is_empty() {
            info!("📦 Orphan pool size: {}", state.orphan_blocks.len());
        }
    }

    fn start_header_sync(&self, node_handle: NodeHandle) {
        let p2p = self.manager.clone();
        tokio::spawn(async move {
            loop {
                let mut locator = Vec::new();
                {
                    let state = node_handle.lock().unwrap();
                    for b in state.blockchain.iter().rev().take(10) {
                        if let Ok(bytes) = hex::decode(&b.hash) {
                            locator.push(bytes);
                        }
                    }
                }
                p2p.request_headers_from_peers(locator, None);
                sleep(Duration::from_secs(15)).await;
            }
        });
    }
}
