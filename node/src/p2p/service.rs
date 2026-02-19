// node/src/p2p/service.rs
use crate::ChainState;
use crate::NodeHandle;
use crate::p2p::manager::{MAX_OUTBOUND, PeerManager};
use hex;
use log::{info, warn};
use Astram_core::block;
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

    pub async fn start(
        &self,
        bind_addr: String,
        node_handle: NodeHandle,
        chain_state: Arc<std::sync::Mutex<ChainState>>,
    ) -> anyhow::Result<()> {
        self.start_listener(bind_addr).await;
        self.connect_initial_peers().await;
        self.register_handlers(node_handle.clone(), chain_state.clone());
        self.start_header_sync(chain_state.clone());

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

    fn register_handlers(
        &self,
        node_handle: NodeHandle,
        chain_state: Arc<std::sync::Mutex<ChainState>>,
    ) {
        let p2p = self.manager.clone();

        // getheaders handler - load headers from DB
        let nh = node_handle.clone();
        p2p.set_on_getheaders(move |locator_hashes, _stop_hash| {
            let mut headers = Vec::new();

            let bc = nh.bc.lock().unwrap();

            // Get chain tip
            let tip_hash = match &bc.chain_tip {
                Some(h) => h.clone(),
                None => return headers,
            };

            // Build full chain from tip backwards
            let mut chain = Vec::new();
            let mut current_hash = Some(tip_hash);
            
            while let Some(hash) = current_hash {
                if let Ok(Some(header)) = bc.load_header(&hash) {
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
                    let hash_hex = hex::encode(loc_hash);
                    if let Some(pos) = chain.iter().position(|h| {
                        if let Ok(computed) = Astram_core::block::compute_header_hash(h) {
                            computed == hash_hex
                        } else {
                            false
                        }
                    }) {
                        found_index = pos + 1; // Start from next block
                        break;
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
        let chain_for_block = chain_state.clone();
        let p2p_for_block = p2p.clone();
        p2p.set_on_block(move |block: block::Block| {
            info!("[P2P] 📦 Block handler START for block #{} {}", block.header.index, &block.hash[..16]);
            let handler_start = std::time::Instant::now();
            
            let nh_async = nh2.clone();
            let chain_async = chain_for_block.clone();
            let p2p_block = p2p_for_block.clone();
            tokio::spawn(async move {
                let state = nh_async;

                // Check if this is a block we recently mined ourselves
                {
                    info!("[P2P] 🔒 Block handler: acquiring chain lock for recently_mined check...");
                    let lock_start = std::time::Instant::now();
                    let chain = chain_async.lock().unwrap();
                    info!("[P2P] ✅ Block handler: chain lock acquired (took {:?})", lock_start.elapsed());
                    
                    if chain.recently_mined_blocks.contains_key(&block.hash) {
                        info!(
                            "[INFO] Ignoring block we mined ourselves: index={} hash={}",
                            block.header.index, block.hash
                        );
                        return;
                    }
                }

                // Cancel ongoing mining when receiving a new block
                state
                    .mining
                    .cancel_flag
                    .store(true, std::sync::atomic::Ordering::SeqCst);

                // Try to insert the block
                info!("[P2P] 🔒 Block handler: acquiring bc lock for validation...");
                let lock_start = std::time::Instant::now();
                let mut bc = state.bc.lock().unwrap();
                info!("[P2P] ✅ Block handler: bc lock acquired (took {:?})", lock_start.elapsed());
                
                let validation_start = std::time::Instant::now();
                match bc.validate_and_insert_block(&block) {
                    Ok(_) => {
                        info!(
                            "[P2P] ✅ Block #{} validated and inserted (validation took {:?})",
                            block.header.index, validation_start.elapsed()
                        );
                        info!(
                            "[OK] Block added via p2p: index={} hash={}",
                            block.header.index, block.hash
                        );
                        
                        // Release bc lock before taking chain lock
                        drop(bc);
                        
                        {
                            info!("[P2P] 🔒 Block handler: acquiring chain lock for blockchain update...");
                            let lock_start = std::time::Instant::now();
                            let mut chain = chain_async.lock().unwrap();
                            info!("[P2P] ✅ Block handler: chain lock acquired (took {:?})", lock_start.elapsed());
                            chain.blockchain.push(block.clone());
                            chain.enforce_memory_limit(); // Security: Enforce memory limit
                        }

                        // Update P2P manager height
                        p2p_block.set_my_height(block.header.index + 1);

                        // Remove transactions from pending pool that are in the new block
                        let block_txids: std::collections::HashSet<String> = block
                            .transactions
                            .iter()
                            .map(|tx| tx.txid.clone())
                            .collect();

                        let removed_count = block_txids.len().saturating_sub(1); // -1 for coinbase
                        {
                            info!("[P2P] 🔒 Block handler: acquiring mempool lock to remove txs...");
                            let lock_start = std::time::Instant::now();
                            let mut mempool = state.mempool.lock().unwrap();
                            info!("[P2P] ✅ Block handler: mempool lock acquired (took {:?})", lock_start.elapsed());
                            mempool.pending.retain(|tx| !block_txids.contains(&tx.txid));
                        }

                        if removed_count > 0 {
                            info!(
                                "[INFO] Removed {} transactions from mempool (included in peer block)",
                                removed_count
                            );
                        }

                        // Reacquire bc lock for reorganization check
                        info!("[P2P] 🔒 Block handler: reacquiring bc lock for reorg check...");
                        let lock_start = std::time::Instant::now();
                        let mut bc = state.bc.lock().unwrap();
                        info!("[P2P] ✅ Block handler: bc lock reacquired (took {:?})", lock_start.elapsed());
                        
                        // Check if this block triggers a chain reorganization
                        match bc.reorganize_if_needed(&block.hash) {
                            Ok(true) => {
                                info!("[OK] Chain reorganization completed");
                            }
                            Ok(false) => {
                                // No reorg needed, current chain is best
                            }
                            Err(e) => {
                                warn!("[WARN] Reorganization check failed: {:?}", e);
                            }
                        }

                        // Try to process orphan blocks that may now be valid
                        {
                            let mut chain = chain_async.lock().unwrap();
                            Self::process_orphan_blocks(
                                &mut bc,
                                &mut chain,
                                &state.mempool,
                                p2p_block.clone(),
                            );
                        }

                        info!("[P2P] ✅ Block handler COMPLETED for block #{} (total time {:?})", block.header.index, handler_start.elapsed());
                        info!("[INFO] Mining cancelled, restarting with updated chain...");
                    }
                    Err(e) => {
                        // Block validation failed - check if it's an orphan
                        let error_msg = format!("{:?}", e);
                        
                        if error_msg.contains("previous header not found") {
                            // Security: Check orphan pool size limit before adding
                            let now = chrono::Utc::now().timestamp();
                            
                            let mut chain = chain_async.lock().unwrap();
                            if chain.orphan_blocks.len() >= crate::MAX_ORPHAN_BLOCKS {
                                warn!(
                                    "[WARN] Orphan pool full ({} blocks), dropping oldest orphan to accept new one",
                                    chain.orphan_blocks.len()
                                );
                                
                                // Find and remove oldest orphan
                                let oldest_hash = chain.orphan_blocks
                                    .iter()
                                    .min_by_key(|(_, (_, timestamp))| *timestamp)
                                    .map(|(h, _)| h.clone());
                                
                                if let Some(hash) = oldest_hash {
                                    chain.orphan_blocks.remove(&hash);
                                }
                            }
                            
                            // Clean up expired orphans (older than 30 minutes)
                            chain.orphan_blocks.retain(|_, (_, timestamp)| {
                                now - *timestamp < crate::ORPHAN_TIMEOUT
                            });
                            
                            chain.orphan_blocks.insert(block.hash.clone(), (block.clone(), now));
                            
                            info!(
                                "[INFO] Orphan block received (index={}, hash={}), storing for later (orphan pool size: {})",
                                block.header.index,
                                &block.hash[..16],
                                chain.orphan_blocks.len()
                            );
                            
                            // Request the parent block
                            // TODO: implement getdata request for parent block
                            info!("[P2P] ⏸️ Block handler: orphan block stored (total time {:?})", handler_start.elapsed());
                        } else {
                            warn!("[WARN] Invalid block from p2p: {:?}", e);
                            info!("[P2P] ❌ Block handler: invalid block rejected (total time {:?})", handler_start.elapsed());
                        }
                    }
                }
            });
        });

        // transaction handler
        let nh3 = node_handle.clone();
        let p2p_for_tx = p2p.clone();
        p2p.set_on_tx(move |tx: Astram_core::transaction::Transaction| {
            info!("[P2P] 💸 TX handler START for tx {}", hex::encode(&tx.txid[..8]));
            let handler_start = std::time::Instant::now();
            
            let nh_async = nh3.clone();
            let p2p_tx_relay = p2p_for_tx.clone();
            tokio::spawn(async move {
                // Check and update state in a separate scope to ensure lock is released
                let should_relay = {
                    let state = nh_async;

                    {
                        info!("[P2P] 🔒 TX handler: acquiring mempool lock for seen_tx check...");
                        let lock_start = std::time::Instant::now();
                        let mut mempool = state.mempool.lock().unwrap();
                        info!("[P2P] ✅ TX handler: mempool lock acquired (took {:?})", lock_start.elapsed());

                        // Check if we've already seen this transaction (prevents loops)
                        if mempool.seen_tx.contains_key(&tx.txid) {
                            info!("[INFO] Transaction {} already seen, skipping", tx.txid);
                            return;
                        }

                        // Check if transaction already exists in pending pool
                        if mempool.pending.iter().any(|t| t.txid == tx.txid) {
                            info!("Transaction {} already in mempool, skipping", tx.txid);
                            // Mark as seen even if already in mempool
                            let now = chrono::Utc::now().timestamp();
                            mempool.seen_tx.insert(tx.txid.clone(), now);
                            return;
                        }
                    }

                    // Validate transaction signatures
                    info!("[P2P] 🔐 TX handler: validating signatures...");
                    let validation_start = std::time::Instant::now();
                    match tx.verify_signatures() {
                        Ok(true) => {
                            info!("[P2P] ✅ TX handler: signatures validated (took {:?})", validation_start.elapsed());
                            info!("[OK] Transaction {} received and validated from p2p", tx.txid);
                            
                            // Security: Check for double-spending in mempool
                            let mut tx_utxos = std::collections::HashSet::new();
                            for inp in &tx.inputs {
                                tx_utxos.insert(format!("{}:{}", inp.txid, inp.vout));
                            }
                            
                            let now = chrono::Utc::now().timestamp();
                            
                            info!("[P2P] 🔒 TX handler: reacquiring mempool lock for conflict check...");
                            let lock_start = std::time::Instant::now();
                            let mut mempool = state.mempool.lock().unwrap();
                            info!("[P2P] ✅ TX handler: mempool lock reacquired (took {:?})", lock_start.elapsed());

                            if mempool.seen_tx.contains_key(&tx.txid)
                                || mempool.pending.iter().any(|t| t.txid == tx.txid)
                            {
                                info!("[INFO] Transaction {} already recorded, skipping", tx.txid);
                                return;
                            }

                            let mut has_conflict = false;
                            for pending_tx in &mempool.pending {
                                for pending_inp in &pending_tx.inputs {
                                    let pending_utxo =
                                        format!("{}:{}", pending_inp.txid, pending_inp.vout);
                                    if tx_utxos.contains(&pending_utxo) {
                                        warn!(
                                            "[WARN] Double-spend detected in P2P TX {}: UTXO {} already used by pending TX {}",
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
                                false
                            } else {
                                // Mark transaction as seen with timestamp
                                mempool.seen_tx.insert(tx.txid.clone(), now);

                                // Clean up old seen_tx entries (older than 1 hour)
                                mempool.seen_tx.retain(|_, &mut timestamp| now - timestamp < 3600);

                                // Add to mempool
                                mempool.pending.push(tx.clone());
                                // Security: Enforce mempool limits after adding transaction
                                mempool.enforce_mempool_limit();
                                info!("[INFO] Mempool size: {} transactions", mempool.pending.len());
                                info!("[P2P] ✅ TX handler: transaction added to mempool (total handler time {:?})", handler_start.elapsed());

                                true // Should relay to other peers
                            }
                        }
                        Ok(false) => {
                            warn!("[WARN] Transaction {} has invalid signatures", tx.txid);
                            info!("[P2P] ❌ TX handler: invalid signatures (total time {:?})", handler_start.elapsed());
                            false
                        }
                        Err(e) => {
                            warn!("[WARN] Transaction {} validation error: {:?}", tx.txid, e);
                            info!("[P2P] ❌ TX handler: validation error (total time {:?})", handler_start.elapsed());
                            false
                        }
                    }
                }; // Lock is released here
                
                // Relay transaction to other peers if validated
                if should_relay {
                    info!("[P2P] 📡 TX handler: relaying to peers...");
                    let relay_start = std::time::Instant::now();
                    p2p_tx_relay.broadcast_tx(&tx).await;
                    info!("[P2P] ✅ TX handler: relayed (took {:?}), total handler time {:?}", relay_start.elapsed(), handler_start.elapsed());
                    info!("[INFO] Relayed transaction {} to other peers", tx.txid);
                }
            });
        });

        // getdata handler - send requested blocks/transactions
        let nh4 = node_handle.clone();
        let p2p_clone = p2p.clone();
        p2p.set_on_getdata(move |peer_id, object_type, hashes| {
            use crate::p2p::messages::InventoryType;
            
            let state = nh4.clone();
            let p2p_inner = p2p_clone.clone();
            
            match object_type {
                InventoryType::Block => {
                    // Load and send requested blocks
                    for hash_bytes in hashes {
                        let hash_hex = hex::encode(&hash_bytes);
                        // Try to load block from DB
                        if let Ok(Some(block)) = state.bc.lock().unwrap().load_block(&hash_hex) {
                            // Send block to peer
                            let peer_id_clone = peer_id.clone();
                            let p2p_for_send = p2p_inner.clone();
                            tokio::spawn(async move {
                                p2p_for_send.send_block_to_peer(&peer_id_clone, &block).await;
                            });
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
    fn process_orphan_blocks(
        bc: &mut Astram_core::Blockchain,
        chain: &mut ChainState,
        mempool: &std::sync::Mutex<crate::MempoolState>,
        p2p_handle: Arc<PeerManager>,
    ) {
        let mut processed_any = true;
        let max_iterations = 100; // Prevent infinite loops
        let mut iterations = 0;

        while processed_any && iterations < max_iterations {
            processed_any = false;
            iterations += 1;

            // Find orphan blocks whose parent now exists
            let orphans_to_try: Vec<_> = chain
                .orphan_blocks
                .iter()
                .map(|(hash, (block, _))| (hash.clone(), block.clone()))
                .collect();

            for (hash, block) in orphans_to_try {
                // Check if parent exists now
                if let Ok(Some(_)) = bc.load_block(&block.header.previous_hash) {
                    // Parent exists! Try to validate and insert
                    match bc.validate_and_insert_block(&block) {
                        Ok(_) => {
                            info!(
                                "[OK] Orphan block now valid: index={} hash={}",
                                block.header.index, &hash[..16]
                            );
                            chain.blockchain.push(block.clone());
                            chain.enforce_memory_limit(); // Security: Enforce memory limit
                            chain.orphan_blocks.remove(&hash);
                            processed_any = true;

                            // Update P2P manager height
                            p2p_handle.set_my_height(block.header.index + 1);

                            // Remove transactions from mempool
                            let block_txids: std::collections::HashSet<String> = block
                                .transactions
                                .iter()
                                .map(|tx| tx.txid.clone())
                                .collect();
                            {
                                let mut mempool = mempool.lock().unwrap();
                                mempool.pending.retain(|tx| !block_txids.contains(&tx.txid));
                            }

                            // Check for reorganization
                            let _ = bc.reorganize_if_needed(&hash);
                        }
                        Err(e) => {
                            warn!(
                                "[WARN] Orphan block still invalid: index={} hash={}, error: {:?}",
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
        chain.orphan_blocks.retain(|hash, (block, timestamp)| {
            let age = now - *timestamp;
            if age > one_hour {
                info!(
                    "[INFO] Removing old orphan block: index={} hash={} (age: {}s)",
                    block.header.index,
                    &hash[..16],
                    age
                );
                false
            } else {
                true
            }
        });

        if !chain.orphan_blocks.is_empty() {
            info!("Orphan pool size: {}", chain.orphan_blocks.len());
        }
    }

    fn start_header_sync(&self, chain_state: Arc<std::sync::Mutex<ChainState>>) {
        let p2p = self.manager.clone();
        tokio::spawn(async move {
            loop {
                let mut locator = Vec::new();
                {
                    let chain = chain_state.lock().unwrap();
                    for b in chain.blockchain.iter().rev().take(10) {
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

