use crate::state::{BlockInfo, TransactionInfo};
use base64::Engine as _;
use chrono::Utc;
use log::{error, info};
use netcoin_core::block::Block;
use netcoin_core::transaction::BINCODE_CONFIG;
use primitive_types::U256;
use reqwest;

/// Parse U256 from hex string (with or without 0x prefix) or decimal string
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

    /// Lightweight counts endpoint
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
    pub async fn fetch_address_info(
        &self,
        address: &str,
    ) -> Result<(U256, U256, U256, usize), String> {
        let url = format!("{}/address/{}/info", self.node_url, address);
        log::info!("🌐 Fetching from Node: {}", url);
        match reqwest::get(&url).await {
            Ok(resp) => match resp.json::<serde_json::Value>().await {
                Ok(v) => {
                    log::info!("📥 Raw JSON from Node: {}", v);

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
                        "✅ Parsed - balance: {}, received: {}, sent: {}, tx_count: {}",
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

    /// Node의 /blockchain/db 엔드포인트에서 실제 블록체인 데이터 조회 (DB에서 직접)
    pub async fn fetch_blocks(&self) -> Result<Vec<BlockInfo>, String> {
        let url = format!("{}/blockchain/db", self.node_url);

        match reqwest::get(&url).await {
            Ok(response) => {
                match response.json::<serde_json::Value>().await {
                    Ok(data) => {
                        // Node에서 base64로 인코딩된 bincode 데이터 획득
                        if let Some(encoded_blockchain) =
                            data.get("blockchain").and_then(|v| v.as_str())
                        {
                            match self.decode_blockchain(encoded_blockchain) {
                                Ok((blocks, _)) => {
                                    info!("✅ Fetched {} blocks from Node", blocks.len());
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

    /// 블록체인 전체 조회 (DB에서 직접, 블록 + 트랜잭션)
    pub async fn fetch_blockchain_with_transactions(
        &self,
    ) -> Result<(Vec<BlockInfo>, Vec<TransactionInfo>), String> {
        let url = format!("{}/blockchain/db", self.node_url);

        match reqwest::get(&url).await {
            Ok(response) => match response.json::<serde_json::Value>().await {
                Ok(data) => {
                    if let Some(encoded_blockchain) =
                        data.get("blockchain").and_then(|v| v.as_str())
                    {
                        match self.decode_blockchain(encoded_blockchain) {
                            Ok((blocks, raw_blocks)) => {
                                let transactions = self.extract_transactions(&raw_blocks);
                                info!(
                                    "✅ Fetched {} blocks and {} transactions from Node",
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
                        error!("No blockchain data in response");
                        Err("No blockchain data in response".to_string())
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

    /// Base64-encoded bincode 데이터 디코딩
    fn decode_blockchain(&self, encoded: &str) -> Result<(Vec<BlockInfo>, Vec<Block>), String> {
        // Base64 디코딩
        let decoded_bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|e| format!("Base64 decode error: {}", e))?;

        // Bincode 디코딩
        let blocks: Vec<Block> = bincode::decode_from_slice(&decoded_bytes, *BINCODE_CONFIG)
            .map(|(blocks, _)| blocks)
            .map_err(|e| format!("Bincode decode error: {}", e))?;

        // Block을 BlockInfo로 변환
        let block_infos: Vec<BlockInfo> = blocks
            .iter()
            .map(|block| {
                let timestamp = chrono::DateTime::<Utc>::from_timestamp(block.header.timestamp, 0)
                    .unwrap_or_else(|| Utc::now());

                // Coinbase 트랜잭션(첫 번째)에서 miner 주소 추출
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
                }
            })
            .collect();

        Ok((block_infos, blocks))
    }

    /// 트랜잭션 정보 조회 (블록에서 추출)
    pub fn extract_transactions(&self, blocks: &[Block]) -> Vec<TransactionInfo> {
        let mut transactions = Vec::new();

        for block in blocks {
            let timestamp = chrono::DateTime::<Utc>::from_timestamp(block.header.timestamp, 0)
                .unwrap_or_else(|| Utc::now());

            for tx in &block.transactions {
                let is_coinbase = tx.inputs.is_empty();

                // Coinbase 트랜잭션: 보상
                if is_coinbase {
                    // 보상 트랜잭션: 모든 output을 분리된 트랜잭션으로 표시
                    for output in &tx.outputs {
                        transactions.push(TransactionInfo {
                            hash: tx.txid.clone(),
                            from: "Block_Reward".to_string(),
                            to: output.to.clone(),
                            amount: output.amount(),
                            fee: U256::zero(),
                            total: output.amount(), // 보상이므로 amount == total
                            timestamp,
                            block_height: Some(block.header.index),
                            status: "confirmed".to_string(),
                        });
                    }
                } else {
                    // 일반 트랜잭션: 모든 output을 표시
                    // Note: fee 계산은 DB 접근이 필요하므로 여기서는 0으로 설정
                    let from = tx
                        .inputs
                        .first()
                        .map(|i| i.pubkey.clone())
                        .unwrap_or_else(|| "Unknown".to_string());

                    for output in &tx.outputs {
                        transactions.push(TransactionInfo {
                            hash: tx.txid.clone(),
                            from: from.clone(),
                            to: output.to.clone(),
                            amount: output.amount(),
                            fee: U256::zero(),      // Fee 계산은 UTXO 조회가 필요
                            total: output.amount(), // 현재는 fee=0이므로 amount == total
                            timestamp,
                            block_height: Some(block.header.index),
                            status: "confirmed".to_string(),
                        });
                    }
                }
            }
        }

        transactions
    }

    /// 노드 상태 정보 조회
    pub async fn fetch_node_status(&self) -> Result<serde_json::Value, String> {
        let url = format!("{}/status", self.node_url);

        match reqwest::get(&url).await {
            Ok(resp) => match resp.json::<serde_json::Value>().await {
                Ok(status) => {
                    info!("✅ Fetched node status from {}", self.node_url);
                    Ok(status)
                }
                Err(e) => Err(format!("Failed to parse node status: {}", e)),
            },
            Err(e) => Err(format!("Network error fetching node status: {}", e)),
        }
    }
}
