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
        existing_utxo_map: &mut std::collections::HashMap<(String, u32), primitive_types::U256>,
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
                                let transactions =
                                    self.extract_transactions(&raw_blocks, existing_utxo_map);
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
    /// UTXO 상태를 추적하여 정확한 수수료 계산
    pub fn extract_transactions(
        &self,
        blocks: &[Block],
        existing_utxo_map: &mut std::collections::HashMap<(String, u32), U256>,
    ) -> Vec<TransactionInfo> {
        let mut transactions = Vec::new();

        // 블록을 높이 순으로 정렬 (매우 중요!)
        let mut sorted_blocks = blocks.to_vec();
        sorted_blocks.sort_by_key(|b| b.header.index);

        log::info!(
            "🔍 Processing {} blocks in order for UTXO tracking (existing UTXO map: {} entries)",
            sorted_blocks.len(),
            existing_utxo_map.len()
        );

        for block in &sorted_blocks {
            let timestamp = chrono::DateTime::<Utc>::from_timestamp(block.header.timestamp, 0)
                .unwrap_or_else(|| Utc::now());

            for tx in &block.transactions {
                let is_coinbase = tx.inputs.is_empty();

                // Coinbase 트랜잭션: 보상
                if is_coinbase {
                    // 보상 트랜잭션: 모든 output의 합계로 하나의 트랜잭션 생성
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
                        hash: tx.eth_hash.clone(), // EVM hash 사용
                        txid: tx.txid.clone(),     // UTXO txid 유지
                        from: "Block_Reward".to_string(),
                        to: to_address,
                        amount: total_amount,
                        fee: U256::zero(),
                        total: total_amount,
                        timestamp,
                        block_height: Some(block.header.index),
                        status: "confirmed".to_string(),
                    });

                    // Coinbase outputs를 UTXO 맵에 추가
                    for (vout, output) in tx.outputs.iter().enumerate() {
                        existing_utxo_map.insert((tx.txid.clone(), vout as u32), output.amount());
                    }
                } else {
                    // 일반 트랜잭션: input 합계와 output 합계로 수수료 계산
                    let from_pubkey = tx
                        .inputs
                        .first()
                        .map(|i| i.pubkey.clone())
                        .unwrap_or_else(|| "Unknown".to_string());

                    // pubkey를 주소로 변환 (거스름돈 비교를 위해)
                    let from_address = if from_pubkey != "Unknown" {
                        netcoin_core::crypto::eth_address_from_pubkey_hex(&from_pubkey)
                            .unwrap_or_else(|_| from_pubkey.clone())
                    } else {
                        from_pubkey.clone()
                    };

                    // Input 총액 계산 (UTXO 맵에서 조회)
                    let mut input_sum = U256::zero();
                    let mut missing_inputs = 0;
                    for (idx, input) in tx.inputs.iter().enumerate() {
                        if let Some(amount) =
                            existing_utxo_map.get(&(input.txid.clone(), input.vout))
                        {
                            input_sum += *amount;
                        } else {
                            missing_inputs += 1;
                            // 처음 3개와 마지막 1개만 상세 로그
                            if idx < 3 || idx == tx.inputs.len() - 1 {
                                log::warn!(
                                    "⚠️ UTXO not found: {}:{} (input #{} of {})",
                                    &input.txid[..8],
                                    input.vout,
                                    idx + 1,
                                    tx.inputs.len()
                                );
                            }
                        }
                    }

                    // 요약 로그
                    if missing_inputs > 0 {
                        log::warn!(
                            "⚠️ TX {}: Missing {}/{} inputs, UTXO map size: {}",
                            &tx.txid[..8],
                            missing_inputs,
                            tx.inputs.len(),
                            existing_utxo_map.len()
                        );
                    }

                    // Output 총액 계산
                    let output_sum = tx
                        .outputs
                        .iter()
                        .fold(U256::zero(), |acc, out| acc + out.amount());

                    // 수수료 = Input 총액 - Output 총액
                    let fee = if input_sum >= output_sum {
                        input_sum - output_sum
                    } else {
                        // Input을 찾지 못한 경우, 실제 트랜잭션 크기로 수수료 계산
                        if missing_inputs > 0 {
                            // 트랜잭션을 serialize해서 실제 크기 측정
                            let tx_size = bincode::encode_to_vec(
                                tx,
                                netcoin_core::blockchain::BINCODE_CONFIG.clone(),
                            )
                            .map(|bytes| bytes.len())
                            .unwrap_or(300); // 기본값 300 bytes

                            // NetCoin 수수료 정책: BASE_MIN_FEE + (size × MIN_RELAY_FEE_NAT_PER_BYTE)
                            // 100 Twei + (size × 200 Gwei)
                            let calculated_fee = U256::from(100_000_000_000_000u64)
                                + U256::from(tx_size as u64) * U256::from(200_000_000_000u64);

                            log::warn!(
                                "⚠️ TX {}: Estimated fee from size: {} bytes = {} natoshi ({} Twei)",
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

                    // 거스름돈 제외한 실제 전송 금액 계산
                    // 보내는 사람(주소)과 다른 주소로 가는 output만 실제 전송으로 간주
                    let mut actual_transfer_amount = U256::zero();
                    let mut recipient_addresses = Vec::new();

                    for output in &tx.outputs {
                        // 받는 주소가 보내는 주소와 다른 경우만 카운트 (거스름돈 제외)
                        if output.to != from_address {
                            actual_transfer_amount += output.amount();
                            recipient_addresses.push(output.to.clone());
                        }
                    }

                    // 만약 모든 output이 같은 주소면 (셀프 전송), output_sum 사용
                    let amount = if recipient_addresses.is_empty() {
                        output_sum
                    } else {
                        actual_transfer_amount
                    };

                    let total = if input_sum > U256::zero() {
                        input_sum
                    } else {
                        // Input 합계를 알 수 없는 경우, output + 추정 수수료
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
                        "💰 TX {}: from_addr={}, outputs={}, actual_transfer={}, change_excluded={}, fee={}",
                        &tx.txid[..8],
                        &from_address[..10],
                        output_sum,
                        amount,
                        output_sum - amount,
                        fee
                    );

                    transactions.push(TransactionInfo {
                        hash: tx.eth_hash.clone(), // EVM hash 사용
                        txid: tx.txid.clone(),     // UTXO txid 유지
                        from: from_address,
                        to: to_address,
                        amount,
                        fee,
                        total,
                        timestamp,
                        block_height: Some(block.header.index),
                        status: "confirmed".to_string(),
                    });

                    // 사용된 inputs를 UTXO 맵에서 제거
                    for input in &tx.inputs {
                        existing_utxo_map.remove(&(input.txid.clone(), input.vout));
                    }

                    // 새로운 outputs를 UTXO 맵에 추가
                    for (vout, output) in tx.outputs.iter().enumerate() {
                        existing_utxo_map.insert((tx.txid.clone(), vout as u32), output.amount());
                    }
                }
            }
        }

        log::info!(
            "✅ Processed {} transactions, UTXO map contains {} entries",
            transactions.len(),
            existing_utxo_map.len()
        );

        transactions
    }

    /// Resolve Ethereum transaction hash to NetCoin txid
    pub async fn resolve_eth_hash(&self, eth_hash: &str) -> Result<String, String> {
        let url = format!("{}/eth_mapping/{}", self.node_url, eth_hash);

        match reqwest::get(&url).await {
            Ok(resp) => match resp.json::<serde_json::Value>().await {
                Ok(v) => {
                    if let Some(found) = v.get("found").and_then(|f| f.as_bool()) {
                        if found {
                            if let Some(txid) = v.get("netcoin_txid").and_then(|t| t.as_str()) {
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
