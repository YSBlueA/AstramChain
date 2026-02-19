use crate::state::{BlockInfo, TransactionInfo};
use Astram_core::block::Block;
use Astram_core::transaction::BINCODE_CONFIG;
use base64::Engine as _;
use chrono::Utc;
use log::{error, info};
use primitive_types::U256;
use reqwest;

/// Parse U256 from hex string (with or without 0x prefix) or decimal string
#[allow(dead_code)]
fn parse_u256_from_json(value: &serde_json::Value) -> Option<U256> {
    if let Some(s) = value.as_str() {
        // Try hex first (0x prefix)
        if let Some(hex_str) = s.strip_prefix("0x") {
            if let Ok(u) = U256::from_str_radix(hex_str, 16) {
                return Some(u);
            }
        }
        // Try decimal
        if let Ok(u) = U256::from_dec_str(s) {
            return Some(u);
        }
    }
    // Try as number
    value.as_u64().map(U256::from)
}

pub struct NodeRpcClient {
    node_url: String,
}

impl NodeRpcClient {
    pub fn new(node_url: &str) -> Self {
        NodeRpcClient {
            node_url: node_url.to_string(),
        }
    }

    /// Fetch node status snapshot
    pub async fn fetch_status(&self) -> Result<serde_json::Value, String> {
        let url = format!("{}/status", self.node_url);
        match reqwest::get(&url).await {
            Ok(resp) => resp
                .json::<serde_json::Value>()
                .await
                .map_err(|e| format!("Failed to parse status response: {}", e)),
            Err(e) => Err(format!("Network error fetching status: {}", e)),
        }
    }

    /// Lightweight counts endpoint
    #[allow(dead_code)]
    pub async fn fetch_counts(&self) -> Result<(u64, u64), String> {
        let url = format!("{}/counts", self.node_url);
        match reqwest::get(&url).await {
            Ok(resp) => match resp.json::<serde_json::Value>().await {
                Ok(v) => {
                    let blocks = v.get("blocks").and_then(|b| b.as_u64()).unwrap_or(0);
                    let transactions = v.get("transactions").and_then(|t| t.as_u64()).unwrap_or(0);
                    Ok((blocks, transactions))
                }
                Err(e) => Err(format!("Failed to parse counts response: {}", e)),
            },
            Err(e) => Err(format!("Network error fetching counts: {}", e)),
        }
    }

    /// Fetch total volume from Node DB
    #[allow(dead_code)]
    pub async fn fetch_total_volume(&self) -> Result<U256, String> {
        let url = format!("{}/counts", self.node_url);
        match reqwest::get(&url).await {
            Ok(resp) => match resp.json::<serde_json::Value>().await {
                Ok(v) => {
                    let volume = v
                        .get("total_volume")
                        .and_then(|vol| parse_u256_from_json(vol))
                        .unwrap_or_else(U256::zero);
                    Ok(volume)
                }
                Err(e) => Err(format!("Failed to parse volume response: {}", e)),
            },
            Err(e) => Err(format!("Network error fetching volume: {}", e)),
        }
    }

    /// Fetch address info from Node DB
    #[allow(dead_code)]
    pub async fn fetch_address_info(
        &self,
        address: &str,
    ) -> Result<(U256, U256, U256, usize), String> {
        let url = format!("{}/address/{}/info", self.node_url, address);
        log::info!("Fetching from Node: {}", url);
        match reqwest::get(&url).await {
            Ok(resp) => match resp.json::<serde_json::Value>().await {
                Ok(v) => {
                    log::info!("Raw JSON from Node: {}", v);

                    // Parse U256 values from JSON (hex or decimal)
                    let balance = v
                        .get("balance")
                        .and_then(|b| {
                            log::info!("Balance field: {:?}", b);
                            parse_u256_from_json(b)
                        })
                        .unwrap_or_else(U256::zero);
                    let received = v
                        .get("received")
                        .and_then(|r| {
                            log::info!("Received field: {:?}", r);
                            parse_u256_from_json(r)
                        })
                        .unwrap_or_else(U256::zero);
                    let sent = v
                        .get("sent")
                        .and_then(|s| {
                            log::info!("Sent field: {:?}", s);
                            parse_u256_from_json(s)
                        })
                        .unwrap_or_else(U256::zero);
                    let tx_count = v
                        .get("transaction_count")
                        .and_then(|t| t.as_u64())
                        .unwrap_or(0) as usize;

                    log::info!(
                        "Parsed - balance: {}, received: {}, sent: {}, tx_count: {}",
                        balance,
                        received,
                        sent,
                        tx_count
                    );

                    Ok((balance, received, sent, tx_count))
                }
                Err(e) => Err(format!("Failed to parse address info response: {}", e)),
            },
            Err(e) => Err(format!("Network error fetching address info: {}", e)),
        }
    }

    /// Query blockchain data from Node /blockchain/db (direct DB)
    #[allow(dead_code)]
    pub async fn fetch_blocks(&self) -> Result<Vec<BlockInfo>, String> {
        let url = format!("{}/blockchain/db", self.node_url);

        match reqwest::get(&url).await {
            Ok(response) => {
                match response.json::<serde_json::Value>().await {
                    Ok(data) => {
                        // Decode base64-encoded bincode from Node
                        if let Some(encoded_blockchain) =
                            data.get("blockchain").and_then(|v| v.as_str())
                        {
                            match self.decode_blockchain(encoded_blockchain) {
                                Ok((blocks, _)) => {
                                    info!("Fetched {} blocks from Node", blocks.len());
                                    Ok(blocks)
                                }
                                Err(e) => {
                                    error!("Failed to decode blockchain: {}", e);
                                    Err(e)
                                }
                            }
                        } else {
                            error!("No blockchain data in response");
                            Err("No blockchain data in response".to_string())
                        }
                    }
                    Err(e) => {
                        error!("Failed to parse blockchain response: {}", e);
                        Err(format!("Parse error: {}", e))
                    }
                }
            }
            Err(e) => {
                error!("Failed to fetch from Node: {}", e);
                Err(format!(
                    "Network error: {}. Make sure Node is running on {}",
                    e, self.node_url
                ))
            }
        }
    }

    /// Fetch blocks in a specific height range
    pub async fn fetch_blocks_range(
        &self,
        from_height: u64,
        existing_utxo_map: &mut std::collections::HashMap<(String, u32), primitive_types::U256>,
    ) -> Result<(Vec<BlockInfo>, Vec<TransactionInfo>), String> {
        let url = format!("{}/blockchain/range?from={}", self.node_url, from_height);

        match reqwest::get(&url).await {
            Ok(response) => match response.json::<serde_json::Value>().await {
                Ok(data) => {
                    if let Some(encoded_blockchain) =
                        data.get("blockchain").and_then(|v| v.as_str())
                    {
                        match self.decode_blockchain(encoded_blockchain) {
                            Ok((blocks, raw_blocks)) => {
                                let transactions =
                                    self.extract_transactions(&raw_blocks, existing_utxo_map);
                                info!(
                                    "Fetched {} blocks (from height {}) and {} transactions from Node",
                                    blocks.len(),
                                    from_height,
                                    transactions.len()
                                );
                                Ok((blocks, transactions))
                            }
                            Err(e) => {
                                error!("Failed to decode blockchain: {}", e);
                                Err(e)
                            }
                        }
                    } else {
                        // No data: return empty result (normal)
                        Ok((vec![], vec![]))
                    }
                }
                Err(e) => {
                    error!("Failed to parse blockchain response: {}", e);
                    Err(format!("Parse error: {}", e))
                }
            },
            Err(e) => {
                error!("Failed to fetch from Node: {}", e);
                Err(format!(
                    "Network error: {}. Make sure Node is running on {}",
                    e, self.node_url
                ))
            }
        }
    }

    /// Fetch full blockchain (direct DB, blocks + transactions)
    pub async fn fetch_blockchain_with_transactions(
        &self,
        existing_utxo_map: &mut std::collections::HashMap<(String, u32), primitive_types::U256>,
    ) -> Result<(Vec<BlockInfo>, Vec<TransactionInfo>), String> {
        let url = format!("{}/blockchain/db", self.node_url);

        info!("Fetching blockchain from: {}", url);

        match reqwest::get(&url).await {
            Ok(response) => {
                let status = response.status();
                info!("Node response status: {}", status);

                match response.json::<serde_json::Value>().await {
                    Ok(data) => {
                        info!("Response data: {:?}", data);

                        if let Some(count) = data.get("count").and_then(|v| v.as_u64()) {
                            info!("Node reports {} blocks", count);
                        }

                        if let Some(encoded_blockchain) =
                            data.get("blockchain").and_then(|v| v.as_str())
                        {
                            info!(
                                "Found blockchain data (encoded length: {})",
                                encoded_blockchain.len()
                            );

                            if encoded_blockchain.is_empty() {
                                info!("Blockchain data is empty");
                                return Ok((vec![], vec![]));
                            }

                            match self.decode_blockchain(encoded_blockchain) {
                                Ok((blocks, raw_blocks)) => {
                                    let transactions =
                                        self.extract_transactions(&raw_blocks, existing_utxo_map);
                                    info!(
                                        "Fetched {} blocks and {} transactions from Node",
                                        blocks.len(),
                                        transactions.len()
                                    );
                                    Ok((blocks, transactions))
                                }
                                Err(e) => {
                                    error!("Failed to decode blockchain: {}", e);
                                    Err(e)
                                }
                            }
                        } else {
                            error!("No 'blockchain' field in response");
                            Err("No blockchain data in response".to_string())
                        }
                    }
                    Err(e) => {
                        error!("Failed to parse blockchain response: {}", e);
                        Err(format!("Parse error: {}", e))
                    }
                }
            }
            Err(e) => {
                error!("Failed to fetch from Node: {}", e);
                Err(format!(
                    "Network error: {}. Make sure Node is running on {}",
                    e, self.node_url
                ))
            }
        }
    }

    /// Decode base64-encoded bincode payload
    fn decode_blockchain(&self, encoded: &str) -> Result<(Vec<BlockInfo>, Vec<Block>), String> {
        // Base64 decode
        let decoded_bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|e| format!("Base64 decode error: {}", e))?;

        // Bincode decode
        let blocks: Vec<Block> = bincode::decode_from_slice(&decoded_bytes, *BINCODE_CONFIG)
            .map(|(blocks, _)| blocks)
            .map_err(|e| format!("Bincode decode error: {}", e))?;

        // Convert Block to BlockInfo
        let block_infos: Vec<BlockInfo> = blocks
            .iter()
            .map(|block| {
                let timestamp = chrono::DateTime::<Utc>::from_timestamp(block.header.timestamp, 0)
                    .unwrap_or_else(|| Utc::now());

                // Coinbase transaction (first) -> extract miner address
                let miner = block
                    .transactions
                    .first()
                    .and_then(|tx| tx.outputs.first())
                    .map(|output| output.to.clone())
                    .unwrap_or_else(|| "Unknown_Miner".to_string());

                BlockInfo {
                    height: block.header.index,
                    hash: block.hash.clone(),
                    timestamp,
                    transactions: block.transactions.len(),
                    miner,
                    difficulty: block.header.difficulty,
                    nonce: block.header.nonce,
                    previous_hash: block.header.previous_hash.clone(),
                    confirmations: 0, // Will be calculated when queried from DB
                }
            })
            .collect();

        Ok((block_infos, blocks))
    }

    /// Extract transaction info from blocks
    /// Track UTXO state to calculate accurate fees
    pub fn extract_transactions(
        &self,
        blocks: &[Block],
        existing_utxo_map: &mut std::collections::HashMap<(String, u32), U256>,
    ) -> Vec<TransactionInfo> {
        let mut transactions = Vec::new();

        // Sort by height (important)
        let mut sorted_blocks = blocks.to_vec();
        sorted_blocks.sort_by_key(|b| b.header.index);

        log::info!(
            "Processing {} blocks in order for UTXO tracking (existing UTXO map: {} entries)",
            sorted_blocks.len(),
            existing_utxo_map.len()
        );

        for block in &sorted_blocks {
            let timestamp = chrono::DateTime::<Utc>::from_timestamp(block.header.timestamp, 0)
                .unwrap_or_else(|| Utc::now());

            for tx in &block.transactions {
                let is_coinbase = tx.inputs.is_empty();

                // Coinbase transaction: reward
                if is_coinbase {
                    // Reward tx: sum all outputs as total amount
                    let total_amount = tx
                        .outputs
                        .iter()
                        .fold(U256::zero(), |acc, out| acc + out.amount());
                    let to_address = if tx.outputs.len() == 1 {
                        tx.outputs[0].to.clone()
                    } else {
                        format!("{} recipients", tx.outputs.len())
                    };

                    transactions.push(TransactionInfo {
                        hash: tx.eth_hash.clone(), // EVM hash
                        txid: tx.txid.clone(),     // UTXO txid
                        from: "Block_Reward".to_string(),
                        to: to_address,
                        amount: total_amount,
                        fee: U256::zero(),
                        total: total_amount,
                        timestamp,
                        block_height: Some(block.header.index),
                        status: "confirmed".to_string(),
                        confirmations: Some(0), // Will be calculated when queried
                    });

                    // Insert coinbase outputs into UTXO map
                    for (vout, output) in tx.outputs.iter().enumerate() {
                        existing_utxo_map.insert((tx.txid.clone(), vout as u32), output.amount());
                    }
                } else {
                    // Standard tx: compute input/output sums and fee
                    let from_pubkey = tx
                        .inputs
                        .first()
                        .map(|i| i.pubkey.clone())
                        .unwrap_or_else(|| "Unknown".to_string());

                    // Convert pubkey to address (for change exclusion)
                    let from_address = if from_pubkey != "Unknown" {
                        Astram_core::crypto::eth_address_from_pubkey_hex(&from_pubkey)
                            .unwrap_or_else(|_| from_pubkey.clone())
                    } else {
                        from_pubkey.clone()
                    };

                    // Sum inputs (lookup from UTXO map)
                    let mut input_sum = U256::zero();
                    let mut missing_inputs = 0;
                    for (idx, input) in tx.inputs.iter().enumerate() {
                        if let Some(amount) =
                            existing_utxo_map.get(&(input.txid.clone(), input.vout))
                        {
                            input_sum += *amount;
                        } else {
                            missing_inputs += 1;
                            // Log details for first 3 and last missing inputs
                            if idx < 3 || idx == tx.inputs.len() - 1 {
                                log::warn!(
                                    "UTXO not found: {}:{} (input #{} of {})",
                                    &input.txid[..8],
                                    input.vout,
                                    idx + 1,
                                    tx.inputs.len()
                                );
                            }
                        }
                    }

                    // Summary log
                    if missing_inputs > 0 {
                        log::warn!(
                            "TX {}: Missing {}/{} inputs, UTXO map size: {}",
                            &tx.txid[..8],
                            missing_inputs,
                            tx.inputs.len(),
                            existing_utxo_map.len()
                        );
                    }

                    // Sum outputs
                    let output_sum = tx
                        .outputs
                        .iter()
                        .fold(U256::zero(), |acc, out| acc + out.amount());

                    // Fee = input sum - output sum
                    let fee = if input_sum >= output_sum {
                        input_sum - output_sum
                    } else {
                        // Missing inputs: estimate fee by tx size
                        if missing_inputs > 0 {
                            // Measure actual size by serialization
                            let tx_size = bincode::encode_to_vec(
                                tx,
                                Astram_core::blockchain::BINCODE_CONFIG.clone(),
                            )
                            .map(|bytes| bytes.len())
                            .unwrap_or(300); // default 300 bytes

                            // Astram fee policy: BASE_MIN_FEE + (size × MIN_RELAY_FEE_NAT_PER_BYTE)
                            // 100 Twei + (size × 200 Gwei)
                            let calculated_fee = U256::from(100_000_000_000_000u64)
                                + U256::from(tx_size as u64) * U256::from(200_000_000_000u64);

                            log::warn!(
                                "TX {}: Estimated fee from size: {} bytes = {} natoshi ({} Twei)",
                                &tx.txid[..8],
                                tx_size,
                                calculated_fee,
                                calculated_fee / U256::from(1_000_000_000_000u64)
                            );

                            calculated_fee
                        } else {
                            U256::zero()
                        }
                    };

                    // Exclude change outputs to compute actual transfer amount
                    // Outputs to different addresses are treated as transfers
                    let mut actual_transfer_amount = U256::zero();
                    let mut recipient_addresses = Vec::new();

                    for output in &tx.outputs {
                        // Count outputs to other addresses only (exclude change)
                        if output.to != from_address {
                            actual_transfer_amount += output.amount();
                            recipient_addresses.push(output.to.clone());
                        }
                    }

                    // If all outputs are to the sender, use total output sum
                    let amount = if recipient_addresses.is_empty() {
                        output_sum
                    } else {
                        actual_transfer_amount
                    };

                    let total = if input_sum > U256::zero() {
                        input_sum
                    } else {
                        // Missing input sums: output + estimated fee
                        output_sum + fee
                    };

                    let to_address = if recipient_addresses.len() == 1 {
                        recipient_addresses[0].clone()
                    } else if recipient_addresses.len() > 1 {
                        format!("{} recipients", recipient_addresses.len())
                    } else if tx.outputs.len() == 1 {
                        tx.outputs[0].to.clone()
                    } else {
                        format!("{} outputs", tx.outputs.len())
                    };

                    log::info!(
                        "TX {}: from_addr={}, outputs={}, actual_transfer={}, change_excluded={}, fee={}",
                        &tx.txid[..8],
                        &from_address[..10],
                        output_sum,
                        amount,
                        output_sum - amount,
                        fee
                    );

                    transactions.push(TransactionInfo {
                        hash: tx.eth_hash.clone(), // EVM hash
                        txid: tx.txid.clone(),     // UTXO txid
                        from: from_address,
                        to: to_address,
                        amount,
                        fee,
                        total,
                        timestamp,
                        block_height: Some(block.header.index),
                        status: "confirmed".to_string(),
                        confirmations: Some(0), // Will be calculated when queried
                    });

                    // Remove spent inputs from UTXO map
                    for input in &tx.inputs {
                        existing_utxo_map.remove(&(input.txid.clone(), input.vout));
                    }

                    // Add new outputs to UTXO map
                    for (vout, output) in tx.outputs.iter().enumerate() {
                        existing_utxo_map.insert((tx.txid.clone(), vout as u32), output.amount());
                    }
                }
            }
        }

        log::info!(
            "Processed {} transactions, UTXO map contains {} entries",
            transactions.len(),
            existing_utxo_map.len()
        );

        transactions
    }

    /// Resolve Ethereum transaction hash to Astram txid
    #[allow(dead_code)]
    pub async fn resolve_eth_hash(&self, eth_hash: &str) -> Result<String, String> {
        let url = format!("{}/eth_mapping/{}", self.node_url, eth_hash);

        match reqwest::get(&url).await {
            Ok(resp) => match resp.json::<serde_json::Value>().await {
                Ok(v) => {
                    if let Some(found) = v.get("found").and_then(|f| f.as_bool()) {
                        if found {
                            if let Some(txid) = v.get("Astram_txid").and_then(|t| t.as_str()) {
                                return Ok(txid.to_string());
                            }
                        }
                    }
                    Err("Mapping not found".to_string())
                }
                Err(e) => Err(format!("Failed to parse mapping response: {}", e)),
            },
            Err(e) => Err(format!("Network error resolving ETH hash: {}", e)),
        }
    }
}
