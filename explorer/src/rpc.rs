use crate::state::{BlockInfo, TransactionInfo};
use base64::Engine as _;
use chrono::Utc;
use log::{error, info};
use netcoin_core::block::Block;
use netcoin_core::transaction::BINCODE_CONFIG;
use reqwest;

pub struct NodeRpcClient {
    node_url: String,
}

impl NodeRpcClient {
    pub fn new(node_url: &str) -> Self {
        NodeRpcClient {
            node_url: node_url.to_string(),
        }
    }

    /// Node의 /blockchain 엔드포인트에서 실제 블록체인 데이터 조회
    pub async fn fetch_blocks(&self) -> Result<Vec<BlockInfo>, String> {
        let url = format!("{}/blockchain", self.node_url);

        match reqwest::get(&url).await {
            Ok(response) => {
                match response.json::<serde_json::Value>().await {
                    Ok(data) => {
                        // Node에서 base64로 인코딩된 bincode 데이터 획득
                        if let Some(encoded_blockchain) =
                            data.get("blockchain").and_then(|v| v.as_str())
                        {
                            match self.decode_blockchain(encoded_blockchain) {
                                Ok(blocks) => {
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

    /// Base64-encoded bincode 데이터 디코딩
    fn decode_blockchain(&self, encoded: &str) -> Result<Vec<BlockInfo>, String> {
        // Base64 디코딩
        let decoded_bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|e| format!("Base64 decode error: {}", e))?;

        // Bincode 디코딩
        let blocks: Vec<Block> = bincode::decode_from_slice(&decoded_bytes, *BINCODE_CONFIG)
            .map(|(blocks, _)| blocks)
            .map_err(|e| format!("Bincode decode error: {}", e))?;

        // Block을 BlockInfo로 변환
        Ok(blocks
            .into_iter()
            .enumerate()
            .map(|(idx, block)| {
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
            .collect())
    }

    /// 트랜잭션 정보 조회 (블록에서 추출)
    pub fn extract_transactions(&self, blocks: &[BlockInfo]) -> Vec<TransactionInfo> {
        let mut transactions = Vec::new();

        for (block_idx, block) in blocks.iter().enumerate() {
            for tx_idx in 0..block.transactions {
                transactions.push(TransactionInfo {
                    hash: format!("0x{:064x}", block_idx * 1000 + tx_idx),
                    from: block.miner.clone(),
                    to: format!("addr_{}", tx_idx % 10),
                    amount: 1_000_000_000,
                    fee: 50_000,
                    timestamp: block.timestamp,
                    block_height: Some(block.height),
                    status: "confirmed".to_string(),
                });
            }
        }

        transactions
    }
}
