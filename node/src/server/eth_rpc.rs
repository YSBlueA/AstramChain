/// Ethereum-compatible JSON-RPC server for MetaMask integration
use crate::NodeHandle;
use Astram_core::transaction::{BINCODE_CONFIG, Transaction, TransactionInput, TransactionOutput};
use primitive_types::U256;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use warp::{Filter, Reply};

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    id: Value,
    method: String,
    params: Option<Vec<Value>>,
}

#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
struct JsonRpcError {
    code: i32,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

impl JsonRpcResponse {
    fn success(id: Value, result: Value) -> Self {
        JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }

    fn error(id: Value, code: i32, message: String) -> Self {
        JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message,
                data: None,
            }),
        }
    }
}

/// Handle JSON-RPC requests
async fn handle_rpc(
    request: JsonRpcRequest,
    node: NodeHandle,
) -> Result<impl Reply, warp::Rejection> {
    log::info!("RPC method called: {}", request.method);

    let response = match request.method.as_str() {
        // Chain information
        "eth_chainId" => eth_chain_id(request.id),
        "net_version" => net_version(request.id),
        "eth_blockNumber" => eth_block_number(request.id, node).await,

        // Account information
        "eth_accounts" => eth_accounts(request.id),
        "eth_getBalance" => eth_get_balance(request.id, request.params, node).await,
        "eth_getTransactionCount" => {
            eth_get_transaction_count(request.id, request.params, node).await
        }

        // Transaction
        "eth_sendRawTransaction" => {
            eth_send_raw_transaction(request.id, request.params, node).await
        }
        "eth_getTransactionByHash" => {
            eth_get_transaction_by_hash(request.id, request.params, node).await
        }
        "eth_getTransactionReceipt" => {
            eth_get_transaction_receipt(request.id, request.params, node).await
        }

        // Block information
        "eth_getBlockByNumber" => eth_get_block_by_number(request.id, request.params, node).await,
        "eth_getBlockByHash" => eth_get_block_by_hash(request.id, request.params, node).await,

        // Gas
        "eth_gasPrice" => eth_gas_price(request.id),
        "eth_estimateGas" => eth_estimate_gas(request.id),

        // Call & Code
        "eth_call" => eth_call(request.id),
        "eth_getCode" => eth_get_code(request.id),

        // Other
        "web3_clientVersion" => web3_client_version(request.id),

        _ => JsonRpcResponse::error(
            request.id,
            -32601,
            format!("Method '{}' not found", request.method),
        ),
    };

    Ok(warp::reply::json(&response))
}

// RPC Method implementations

fn eth_chain_id(id: Value) -> JsonRpcResponse {
    // Chain ID for Astram (use a unique ID, e.g., 8888)
    JsonRpcResponse::success(id, json!("0x22b8")) // 8888 in hex
}

fn net_version(id: Value) -> JsonRpcResponse {
    JsonRpcResponse::success(id, json!("8888"))
}

async fn eth_block_number(id: Value, node: NodeHandle) -> JsonRpcResponse {
    let state = node.lock().unwrap();
    let height = state.bc.get_all_blocks().map(|b| b.len()).unwrap_or(0);
    JsonRpcResponse::success(id, json!(format!("0x{:x}", height)))
}

fn eth_accounts(id: Value) -> JsonRpcResponse {
    // MetaMask manages accounts, return empty array
    JsonRpcResponse::success(id, json!([]))
}

async fn eth_get_balance(
    id: Value,
    params: Option<Vec<Value>>,
    node: NodeHandle,
) -> JsonRpcResponse {
    if let Some(params) = params {
        if let Some(address) = params.get(0).and_then(|v| v.as_str()) {
            // Keep 0x prefix - addresses are stored with 0x in DB
            let address = address.to_lowercase();

            let state = node.lock().unwrap();
            let balance = state
                .bc
                .get_address_balance_from_db(&address)
                .unwrap_or_else(|_| U256::zero());

            // ram and wei are now the same (both 10^18 decimals)
            // Convert U256 to hex string with 0x prefix
            return JsonRpcResponse::success(id, json!(format!("0x{:x}", balance)));
        }
    }

    JsonRpcResponse::error(id, -32602, "Invalid params".to_string())
}

async fn eth_get_transaction_count(
    id: Value,
    params: Option<Vec<Value>>,
    node: NodeHandle,
) -> JsonRpcResponse {
    if let Some(params) = params {
        if let Some(address) = params.get(0).and_then(|v| v.as_str()) {
            // Keep 0x prefix - addresses are stored with 0x in DB
            let address = address.to_lowercase();

            let state = node.lock().unwrap();
            let count = state
                .bc
                .get_address_transaction_count_from_db(&address)
                .unwrap_or(0);

            return JsonRpcResponse::success(id, json!(format!("0x{:x}", count)));
        }
    }

    JsonRpcResponse::error(id, -32602, "Invalid params".to_string())
}

async fn eth_send_raw_transaction(
    id: Value,
    params: Option<Vec<Value>>,
    node: NodeHandle,
) -> JsonRpcResponse {
    if let Some(params) = params {
        if let Some(raw_tx_hex) = params.get(0).and_then(|v| v.as_str()) {
            // Parse Ethereum raw transaction
            let raw_tx = match raw_tx_hex.strip_prefix("0x") {
                Some(hex) => hex,
                None => raw_tx_hex,
            };

            let tx_bytes = match hex::decode(raw_tx) {
                Ok(bytes) => bytes,
                Err(e) => {
                    log::warn!("Failed to decode raw transaction hex: {}", e);
                    return JsonRpcResponse::error(id, -32602, format!("Invalid hex: {}", e));
                }
            };

            // Calculate Ethereum transaction hash (for MetaMask compatibility)
            use tiny_keccak::{Hasher, Keccak};
            let mut hasher = Keccak::v256();
            hasher.update(&tx_bytes);
            let mut eth_tx_hash = [0u8; 32];
            hasher.finalize(&mut eth_tx_hash);
            let eth_tx_hash_hex = hex::encode(&eth_tx_hash);

            log::info!("Ethereum transaction hash: 0x{}", eth_tx_hash_hex);

            // Decode Ethereum transaction (RLP encoded)
            let eth_tx = match decode_ethereum_transaction(&tx_bytes) {
                Ok(tx) => tx,
                Err(e) => {
                    log::warn!("Failed to decode Ethereum transaction: {}", e);
                    return JsonRpcResponse::error(
                        id,
                        -32602,
                        format!("Invalid transaction: {}", e),
                    );
                }
            };

            log::info!(
                "[INFO] MetaMask transaction: from={}, to={}, value={}, nonce={}",
                eth_tx.from,
                eth_tx.to,
                eth_tx.value,
                eth_tx.nonce
            );

            // Convert Ethereum transaction to Astram UTXO transaction
            let Astram_tx = match convert_eth_to_utxo_transaction(eth_tx, node.clone()).await {
                Ok(tx) => tx,
                Err(e) => {
                    log::error!("Failed to convert Ethereum tx to UTXO: {}", e);
                    return JsonRpcResponse::error(
                        id,
                        -32000,
                        format!("Transaction conversion failed: {}", e),
                    );
                }
            };

            log::info!(
                "[INFO] Converted to Astram UTXO transaction: txid={}, eth_hash={}",
                Astram_tx.txid,
                Astram_tx.eth_hash
            );

            // Add to mempool
            let mut state = node.lock().unwrap();

            // Check if already seen
            if state.seen_tx.contains_key(&Astram_tx.txid) {
                log::warn!("Transaction already seen: {}", Astram_tx.txid);
                return JsonRpcResponse::success(id, json!(Astram_tx.eth_hash));
            }

            // Verify signatures
            if !Astram_tx.verify_signatures().unwrap_or(false) {
                log::error!("Transaction signature verification failed");
                return JsonRpcResponse::error(id, -32000, "Invalid signature".to_string());
            }

            // Security: Check for double-spending in mempool
            let mut tx_utxos = std::collections::HashSet::new();
            for inp in &Astram_tx.inputs {
                tx_utxos.insert(format!("{}:{}", inp.txid, inp.vout));
            }

            for pending_tx in &state.pending {
                for pending_inp in &pending_tx.inputs {
                    let pending_utxo = format!("{}:{}", pending_inp.txid, pending_inp.vout);
                    if tx_utxos.contains(&pending_utxo) {
                        log::warn!(
                            "Double-spend attempt via eth_sendRawTransaction: TX {} tries to use UTXO {} already used by pending TX {}",
                            Astram_tx.txid,
                            pending_utxo,
                            pending_tx.txid
                        );
                        return JsonRpcResponse::error(
                            id,
                            -32000,
                            format!(
                                "Double-spend: UTXO {} already used in mempool",
                                pending_utxo
                            ),
                        );
                    }
                }
            }

            // Add to pending
            let now = chrono::Utc::now().timestamp();
            state.seen_tx.insert(Astram_tx.txid.clone(), now);
            state.pending.push(Astram_tx.clone());

            // Store mapping: eth_hash -> txid
            state
                .eth_to_Astram_tx
                .insert(Astram_tx.eth_hash.clone(), Astram_tx.txid.clone());

            log::info!(
                "[INFO] Stored mapping: ETH hash {} -> Astram txid {}",
                Astram_tx.eth_hash,
                Astram_tx.txid
            );
            log::info!("[INFO] Transaction added to mempool: {}", Astram_tx.txid);
            log::info!("[INFO] Current mapping size: {}", state.eth_to_Astram_tx.len());

            // Broadcast to peers
            let p2p_clone = state.p2p.clone();
            let tx_clone = Astram_tx.clone();
            let eth_hash_result = Astram_tx.eth_hash.clone();
            drop(state); // Release lock before spawning

            tokio::spawn(async move {
                p2p_clone.broadcast_tx(&tx_clone).await;
            });

            // Return Ethereum transaction hash (what MetaMask expects)
            log::info!(
                "[INFO] Returning ETH transaction hash to MetaMask: {}",
                eth_hash_result
            );
            return JsonRpcResponse::success(id, json!(eth_hash_result));
        }
    }

    JsonRpcResponse::error(id, -32602, "Invalid params".to_string())
}

/// Ethereum transaction structure (simplified)
#[derive(Debug)]
struct EthereumTransaction {
    nonce: u64,
    gas_price: U256,
    gas_limit: u64,
    to: String,
    value: U256,
    data: Vec<u8>,
    v: u64,
    r: Vec<u8>,
    s: Vec<u8>,
    from: String, // Recovered from signature
}

/// Decode Ethereum RLP transaction
fn decode_ethereum_transaction(tx_bytes: &[u8]) -> Result<EthereumTransaction, String> {
    use rlp::Rlp;

    let rlp = Rlp::new(tx_bytes);

    // Legacy transaction format: [nonce, gasPrice, gasLimit, to, value, data, v, r, s]
    let nonce: u64 = rlp
        .at(0)
        .map_err(|e| format!("nonce: {}", e))?
        .as_val()
        .map_err(|e| format!("nonce parse: {}", e))?;

    let gas_price_bytes: Vec<u8> = rlp
        .at(1)
        .map_err(|e| format!("gas_price: {}", e))?
        .data()
        .map_err(|e| format!("gas_price data: {}", e))?
        .to_vec();
    let gas_price = U256::from_big_endian(&gas_price_bytes);

    let gas_limit: u64 = rlp
        .at(2)
        .map_err(|e| format!("gas_limit: {}", e))?
        .as_val()
        .map_err(|e| format!("gas_limit parse: {}", e))?;

    let to_bytes: Vec<u8> = rlp
        .at(3)
        .map_err(|e| format!("to: {}", e))?
        .data()
        .map_err(|e| format!("to data: {}", e))?
        .to_vec();
    let to = if to_bytes.is_empty() {
        String::new()
    } else {
        format!("0x{}", hex::encode(&to_bytes))
    };

    let value_bytes: Vec<u8> = rlp
        .at(4)
        .map_err(|e| format!("value: {}", e))?
        .data()
        .map_err(|e| format!("value data: {}", e))?
        .to_vec();
    let value = U256::from_big_endian(&value_bytes);

    let data: Vec<u8> = rlp
        .at(5)
        .map_err(|e| format!("data: {}", e))?
        .data()
        .map_err(|e| format!("data parse: {}", e))?
        .to_vec();

    let v: u64 = rlp
        .at(6)
        .map_err(|e| format!("v: {}", e))?
        .as_val()
        .map_err(|e| format!("v parse: {}", e))?;

    let r: Vec<u8> = rlp
        .at(7)
        .map_err(|e| format!("r: {}", e))?
        .data()
        .map_err(|e| format!("r data: {}", e))?
        .to_vec();

    let s: Vec<u8> = rlp
        .at(8)
        .map_err(|e| format!("s: {}", e))?
        .data()
        .map_err(|e| format!("s data: {}", e))?
        .to_vec();

    // Recover sender address from signature
    // Need to reconstruct unsigned transaction for proper signature recovery
    // RLP may strip leading zeros, so pad r and s to 32 bytes
    let r_padded = pad_to_32_bytes(&r);
    let s_padded = pad_to_32_bytes(&s);

    let (from, pubkey) = recover_sender_address_eip155(
        nonce,
        &gas_price_bytes,
        gas_limit,
        &to_bytes,
        &value_bytes,
        &data,
        v,
        &r_padded,
        &s_padded,
    )
    .unwrap_or_else(|e| {
        log::warn!("Failed to recover sender address: {}", e);
        (
            "0x0000000000000000000000000000000000000000".to_string(),
            String::new(),
        )
    });

    Ok(EthereumTransaction {
        nonce,
        gas_price,
        gas_limit,
        to,
        value,
        data,
        v,
        r,
        s,
        from: format!("{};{}", from, pubkey), // Store both address and pubkey
    })
}
/// Pad signature component to 32 bytes (RLP strips leading zeros)
fn pad_to_32_bytes(data: &[u8]) -> [u8; 32] {
    let mut result = [0u8; 32];
    let start = 32 - data.len().min(32);
    result[start..].copy_from_slice(&data[..data.len().min(32)]);
    result
}
/// Recover sender address and public key from Ethereum signature
fn recover_sender_address_eip155(
    nonce: u64,
    gas_price_bytes: &[u8],
    gas_limit: u64,
    to_bytes: &[u8],
    value_bytes: &[u8],
    data: &[u8],
    v: u64,
    r: &[u8],
    s: &[u8],
) -> Result<(String, String), String> {
    use rlp::RlpStream;
    use secp256k1::{Message, Secp256k1, ecdsa::RecoverableSignature};
    use tiny_keccak::{Hasher, Keccak};

    // Calculate chain_id from v (EIP-155)
    let chain_id = if v >= 35 {
        (v - 35) / 2
    } else {
        0 // Legacy transaction
    };

    // Reconstruct unsigned transaction for EIP-155
    let mut stream = RlpStream::new();
    stream.begin_list(if chain_id > 0 { 9 } else { 6 });

    stream.append(&nonce);
    stream.append(&gas_price_bytes);
    stream.append(&gas_limit);
    stream.append(&to_bytes);
    stream.append(&value_bytes);
    stream.append(&data);

    // For EIP-155, append chain_id, 0, 0
    if chain_id > 0 {
        stream.append(&chain_id);
        stream.append(&0u8);
        stream.append(&0u8);
    }

    let unsigned_tx = stream.out();

    // Hash the unsigned transaction
    let mut hasher = Keccak::v256();
    hasher.update(&unsigned_tx);
    let mut tx_hash = [0u8; 32];
    hasher.finalize(&mut tx_hash);

    // Parse signature
    if r.len() != 32 || s.len() != 32 {
        return Err("Invalid signature length".to_string());
    }

    let recovery_id = if v >= 35 {
        ((v - 35) % 2) as i32
    } else {
        (v - 27) as i32
    };

    let mut sig_data = [0u8; 64];
    sig_data[..32].copy_from_slice(r);
    sig_data[32..].copy_from_slice(s);

    let secp = Secp256k1::new();
    let rec_id = secp256k1::ecdsa::RecoveryId::from_i32(recovery_id)
        .map_err(|e| format!("Invalid recovery id: {}", e))?;

    let recoverable_sig = RecoverableSignature::from_compact(&sig_data, rec_id)
        .map_err(|e| format!("Invalid signature: {}", e))?;

    let message =
        Message::from_digest_slice(&tx_hash).map_err(|e| format!("Invalid message: {}", e))?;

    let public_key = secp
        .recover_ecdsa(&message, &recoverable_sig)
        .map_err(|e| format!("Recovery failed: {}", e))?;

    // Convert to Ethereum address
    let public_key_bytes = public_key.serialize_uncompressed();

    let mut hasher = Keccak::v256();
    hasher.update(&public_key_bytes[1..]); // Skip 0x04
    let mut pub_hash = [0u8; 32];
    hasher.finalize(&mut pub_hash);

    let address = format!("0x{}", hex::encode(&pub_hash[12..]));

    log::info!(
        "Recovered sender: {} (chain_id={}, v={})",
        address,
        chain_id,
        v
    );

    // Return both address and public key hex
    let pubkey_hex = hex::encode(public_key.serialize_uncompressed());
    Ok((address, pubkey_hex))
}

/// Convert Ethereum transaction to Astram UTXO transaction
async fn convert_eth_to_utxo_transaction(
    eth_tx: EthereumTransaction,
    node: NodeHandle,
) -> Result<Transaction, String> {
    // Extract address and public key from combined from field
    let parts: Vec<&str> = eth_tx.from.split(';').collect();
    let from_addr = parts[0].to_lowercase();
    let pubkey_hex = if parts.len() > 1 {
        parts[1].to_string()
    } else {
        return Err("Missing public key in transaction".to_string());
    };

    let to_addr = eth_tx.to.to_lowercase();
    let amount = eth_tx.value;

    if to_addr.is_empty() {
        return Err("Contract creation not supported".to_string());
    }

    log::info!(
        "Converting: {} ASRM from {} to {}",
        amount,
        from_addr,
        to_addr
    );

    // Get UTXOs for sender
    let utxos = {
        let state = node.lock().unwrap();
        state
            .bc
            .get_utxos(&from_addr)
            .map_err(|e| format!("Failed to get UTXOs: {}", e))?
    };

    if utxos.is_empty() {
        return Err(format!("No UTXOs found for address {}", from_addr));
    }

    // Use fee from Ethereum gas parameters (MetaMask already calculated this)
    // MetaMask sends: gasPrice (in ram/gas) × gasLimit (in gas units)
    let fee_from_eth = eth_tx.gas_price * U256::from(eth_tx.gas_limit);

    log::info!(
        "ETH transaction fee: {} ram (gasPrice={}, gasLimit={})",
        fee_from_eth,
        eth_tx.gas_price,
        eth_tx.gas_limit
    );

    let total_needed = amount + fee_from_eth;

    let mut selected_utxos = Vec::new();
    let mut total_input = U256::zero();

    for utxo in utxos {
        selected_utxos.push(utxo.clone());
        total_input = total_input + utxo.amount();

        if total_input >= total_needed {
            break;
        }
    }

    if total_input < total_needed {
        return Err(format!(
            "Insufficient funds: have {}, need {} (amount {} + fee {})",
            total_input, total_needed, amount, fee_from_eth
        ));
    }

    // Create inputs with Ethereum signature
    // We create a special signature format: eth_sig:v:r:s
    // This will be validated differently for Ethereum-originated transactions
    let eth_sig = format!(
        "eth_sig:{}:{}:{}",
        eth_tx.v,
        hex::encode(&eth_tx.r),
        hex::encode(&eth_tx.s)
    );

    let inputs: Vec<TransactionInput> = selected_utxos
        .iter()
        .map(|utxo| TransactionInput {
            txid: utxo.txid.clone(),
            vout: utxo.vout,
            pubkey: pubkey_hex.clone(), // Keep original format for verify_signatures()
            signature: Some(eth_sig.clone()),
        })
        .collect();

    // Create outputs (temporary, will recalculate after measuring actual tx size)
    let mut outputs = vec![TransactionOutput::new(to_addr.clone(), amount)];

    // Add temporary change output
    let temp_change = total_input - amount - fee_from_eth;
    if temp_change > U256::zero() {
        outputs.push(TransactionOutput::new(from_addr.clone(), temp_change));
    }

    // Create transaction to measure actual size
    let mut tx = Transaction {
        txid: String::new(),
        eth_hash: String::new(),
        inputs,
        outputs,
        timestamp: chrono::Utc::now().timestamp(),
    };

    tx = tx.with_hashes();

    // Calculate actual transaction size in bytes using bincode v2
    let tx_bytes = bincode::encode_to_vec(&tx, *BINCODE_CONFIG)
        .map_err(|e| format!("Failed to serialize transaction: {}", e))?;
    let actual_tx_size = tx_bytes.len();

    // Calculate minimum required fee based on actual size
    let min_fee_required = Astram_core::config::calculate_min_fee(actual_tx_size);

    log::info!(
        "Transaction size: {} bytes, ETH fee: {} ram, Astram min required: {} ram",
        actual_tx_size,
        fee_from_eth,
        min_fee_required
    );

    // Verify fee is sufficient
    if fee_from_eth < min_fee_required {
        return Err(format!(
            "Insufficient fee: provided {} ram, but need {} ram (base 100 Twei + {} bytes × 200 Gwei/byte)",
            fee_from_eth, min_fee_required, actual_tx_size
        ));
    }

    // Recalculate change with actual fee
    let final_change = total_input - amount - fee_from_eth;

    // Recreate outputs with correct change
    let mut final_outputs = vec![TransactionOutput::new(to_addr.clone(), amount)];
    if final_change > U256::zero() {
        final_outputs.push(TransactionOutput::new(from_addr.clone(), final_change));
    }

    // Recreate transaction with final outputs
    tx.outputs = final_outputs;
    tx = tx.with_hashes();

    log::info!(
        "Created UTXO tx: {} inputs, {} outputs, {} bytes, fee={} ram, txid={}",
        tx.inputs.len(),
        tx.outputs.len(),
        actual_tx_size,
        fee_from_eth,
        tx.txid
    );

    Ok(tx)
}

async fn eth_get_transaction_by_hash(
    id: Value,
    params: Option<Vec<Value>>,
    node: NodeHandle,
) -> JsonRpcResponse {
    if let Some(params) = params {
        if let Some(tx_hash) = params.get(0).and_then(|v| v.as_str()) {
            let tx_hash = tx_hash.strip_prefix("0x").unwrap_or(tx_hash);

            let state = node.lock().unwrap();

            // Try to resolve Ethereum tx hash to Astram txid
            let Astram_txid = state
                .eth_to_Astram_tx
                .get(tx_hash)
                .map(|s| s.as_str())
                .unwrap_or(tx_hash);

            if let Ok(Some((tx, block_height))) = state.bc.get_transaction(Astram_txid) {
                // ram and wei are now the same (both 10^18 decimals)
                let amount = tx
                    .outputs
                    .get(0)
                    .map(|o| o.amount())
                    .unwrap_or_else(U256::zero);

                // Convert to Ethereum transaction format
                return JsonRpcResponse::success(
                    id,
                    json!({
                        "hash": format!("0x{}", tx_hash), // Return original ETH hash
                        "nonce": "0x0",
                        "blockHash": null, // Would need block hash
                        "blockNumber": format!("0x{:x}", block_height),
                        "transactionIndex": "0x0",
                        "from": tx.inputs.get(0).map(|i| &i.pubkey).unwrap_or(&String::new()).clone(),
                        "to": tx.outputs.get(0).map(|o| &o.to).unwrap_or(&String::new()).clone(),
                        "value": format!("0x{:x}", amount),
                        "gasPrice": "0x2540be400", // 10 Gwei
                        "gas": "0x5208", // 21000 gas
                        "input": "0x",
                    }),
                );
            }
        }
    }

    JsonRpcResponse::success(id, json!(null))
}

async fn eth_get_transaction_receipt(
    id: Value,
    params: Option<Vec<Value>>,
    node: NodeHandle,
) -> JsonRpcResponse {
    if let Some(params) = params {
        if let Some(tx_hash) = params.get(0).and_then(|v| v.as_str()) {
            let tx_hash = tx_hash.strip_prefix("0x").unwrap_or(tx_hash);

            log::info!("[INFO] eth_getTransactionReceipt called for: 0x{}", tx_hash);

            let state = node.lock().unwrap();

            // Try to find transaction by eth_hash first (recommended)
            match state
                .bc
                .get_transaction_by_eth_hash(&format!("0x{}", tx_hash))
            {
                Ok(Some((tx, block_height))) => {
                    log::info!(
                        "[INFO] Transaction found by eth_hash in block {}: txid={}",
                        block_height,
                        tx.txid
                    );

                    // Get block hash
                    let block_hash = match state.bc.get_all_blocks() {
                        Ok(blocks) => {
                            if let Some(block) = blocks.get(block_height) {
                                format!("0x{}", block.hash)
                            } else {
                                "0x0000000000000000000000000000000000000000000000000000000000000000"
                                    .to_string()
                            }
                        }
                        Err(_) => {
                            "0x0000000000000000000000000000000000000000000000000000000000000000"
                                .to_string()
                        }
                    };

                    // Extract sender address from pubkey (first input)
                    // Input pubkey format: "address;publickey" or just Ethereum address
                    let from_addr = tx
                        .inputs
                        .get(0)
                        .map(|i| {
                            // If pubkey contains semicolon, extract address part
                            if let Some(pos) = i.pubkey.find(';') {
                                i.pubkey[..pos].to_string()
                            } else if i.pubkey.starts_with("0x") && i.pubkey.len() == 42 {
                                // Already an Ethereum address
                                i.pubkey.clone()
                            } else {
                                // Fallback: assume it's an address
                                i.pubkey.clone()
                            }
                        })
                        .unwrap_or_else(|| {
                            "0x0000000000000000000000000000000000000000".to_string()
                        });

                    let receipt = json!({
                        "transactionHash": format!("0x{}", tx_hash),
                        "transactionIndex": "0x0",
                        "blockHash": block_hash,
                        "blockNumber": format!("0x{:x}", block_height),
                        "from": from_addr,
                        "to": tx.outputs.get(0).map(|o| &o.to).unwrap_or(&String::new()).clone(),
                        "cumulativeGasUsed": "0x5208", // 21000 gas
                        "gasUsed": "0x5208", // 21000 gas
                        "contractAddress": null,
                        "logs": [],
                        "logsBloom": "0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
                        "status": "0x1",
                        "effectiveGasPrice": "0x2540be400", // 10 Gwei (10,000,000,000 ram/gas)
                    });

                    log::info!("[INFO] Returning receipt: {:?}", receipt);

                    return JsonRpcResponse::success(id, receipt);
                }
                Ok(None) => {
                    log::info!("[WARN] Transaction not found by eth_hash: 0x{}", tx_hash);
                }
                Err(e) => {
                    log::error!(
                        "[ERROR] Error querying transaction by eth_hash 0x{}: {}",
                        tx_hash,
                        e
                    );
                }
            }
        }
    }

    log::info!("[INFO] Returning null receipt");
    JsonRpcResponse::success(id, json!(null))
}

fn eth_gas_price(id: Value) -> JsonRpcResponse {
    // Astram fee structure (EVM-compatible 18 decimals):
    // - Base fee: 100,000,000,000,000 ram (100 Twei = 0.0001 ASRM)
    // - Per-byte fee: 200,000,000,000 ram/byte (200 Gwei/byte)
    // - Typical UTXO tx size: ~300-1000 bytes
    // - Total typical fee: ~160,000,000,000,000-300,000,000,000,000 ram
    //
    // For Ethereum compatibility:
    // - Standard transfer gas: 21,000
    // - Target fee for 300 byte tx: ~160,000,000,000,000 ram
    // - Required gasPrice: 160,000,000,000,000 / 21,000 = ~7,619,047,619 ram/gas
    // - Use 10,000,000,000 (10 Gwei) for safety margin
    //
    // This gives: 21,000 × 10,000,000,000 = 210,000,000,000,000 ram (~0.00021 ASRM)
    // Hex: 10,000,000,000 = 0x2540BE400
    JsonRpcResponse::success(id, json!("0x2540be400")) // 10 Gwei (10,000,000,000 ram/gas)
}

fn eth_estimate_gas(id: Value) -> JsonRpcResponse {
    // Astram UTXO transactions are larger than Ethereum account model
    // Typical UTXO tx size: 300-1000 bytes
    // Required fee for 1000 byte tx: 100 Twei + (1000 × 200 Gwei) = 300 Twei
    // With gasPrice of 10 Gwei: 300 Twei ÷ 10 Gwei = 30,000 gas
    // Use 50,000 gas for safety margin (covers up to 1450 byte transactions)
    // Total fee: 50,000 × 10 Gwei = 500 Twei (0.0005 ASRM)
    JsonRpcResponse::success(id, json!("0xc350")) // 50,000 gas (UTXO transaction)
}

async fn eth_get_block_by_number(
    id: Value,
    params: Option<Vec<Value>>,
    node: NodeHandle,
) -> JsonRpcResponse {
    if let Some(params) = params {
        if let Some(block_param) = params.get(0).and_then(|v| v.as_str()) {
            let state = node.lock().unwrap();

            // Parse block number or handle "latest", "earliest", "pending"
            let block_number = match block_param {
                "latest" | "pending" => state
                    .bc
                    .get_all_blocks()
                    .map(|b| b.len())
                    .unwrap_or(0)
                    .saturating_sub(1),
                "earliest" => 0,
                _ => {
                    // Parse hex number
                    let num_str = block_param.strip_prefix("0x").unwrap_or(block_param);
                    u64::from_str_radix(num_str, 16).unwrap_or(0) as usize
                }
            };

            // Get full transaction details flag
            let _full_tx = params.get(1).and_then(|v| v.as_bool()).unwrap_or(false);

            if let Ok(blocks) = state.bc.get_all_blocks() {
                if let Some(block) = blocks.get(block_number) {
                    return JsonRpcResponse::success(
                        id,
                        json!({
                            "number": format!("0x{:x}", block_number),
                            "hash": format!("0x{}", block.hash),
                            "parentHash": format!("0x{}", block.header.previous_hash),
                            "nonce": "0x0000000000000000",
                            "sha3Uncles": "0x0000000000000000000000000000000000000000000000000000000000000000",
                            "logsBloom": "0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
                            "transactionsRoot": "0x0000000000000000000000000000000000000000000000000000000000000000",
                            "stateRoot": "0x0000000000000000000000000000000000000000000000000000000000000000",
                            "receiptsRoot": "0x0000000000000000000000000000000000000000000000000000000000000000",
                            "miner": block.transactions.get(0).and_then(|tx| tx.outputs.get(0)).map(|o| &o.to).unwrap_or(&String::new()).clone(),
                            "difficulty": "0x1",
                            "totalDifficulty": format!("0x{:x}", block_number + 1),
                            "extraData": "0x",
                            "size": "0x400",
                            "gasLimit": "0x1fffffffffffff",
                            "gasUsed": "0x0",
                            "timestamp": format!("0x{:x}", block.header.timestamp),
                            "transactions": block.transactions.iter().map(|tx| format!("0x{}", tx.txid)).collect::<Vec<_>>(),
                            "uncles": []
                        }),
                    );
                }
            }
        }
    }

    JsonRpcResponse::success(id, json!(null))
}

async fn eth_get_block_by_hash(
    id: Value,
    params: Option<Vec<Value>>,
    node: NodeHandle,
) -> JsonRpcResponse {
    if let Some(params) = params {
        if let Some(block_hash) = params.get(0).and_then(|v| v.as_str()) {
            let block_hash = block_hash.strip_prefix("0x").unwrap_or(block_hash);
            let _full_tx = params.get(1).and_then(|v| v.as_bool()).unwrap_or(false);

            let state = node.lock().unwrap();
            if let Ok(blocks) = state.bc.get_all_blocks() {
                if let Some((block_number, block)) = blocks
                    .iter()
                    .enumerate()
                    .find(|(_, b)| b.hash == block_hash)
                {
                    return JsonRpcResponse::success(
                        id,
                        json!({
                            "number": format!("0x{:x}", block_number),
                            "hash": format!("0x{}", block.hash),
                            "parentHash": format!("0x{}", block.header.previous_hash),
                            "timestamp": format!("0x{:x}", block.header.timestamp),
                            "transactions": block.transactions.iter().map(|tx| format!("0x{}", tx.txid)).collect::<Vec<_>>(),
                            "miner": block.transactions.get(0).and_then(|tx| tx.outputs.get(0)).map(|o| &o.to).unwrap_or(&String::new()).clone(),
                            "gasLimit": "0x1fffffffffffff",
                            "gasUsed": "0x0",
                        }),
                    );
                }
            }
        }
    }

    JsonRpcResponse::success(id, json!(null))
}

fn eth_call(id: Value) -> JsonRpcResponse {
    // For UTXO-based blockchain, eth_call is not directly applicable
    // Return empty result for contract calls
    JsonRpcResponse::success(id, json!("0x"))
}

fn eth_get_code(id: Value) -> JsonRpcResponse {
    // No smart contracts in UTXO model
    JsonRpcResponse::success(id, json!("0x"))
}

fn web3_client_version(id: Value) -> JsonRpcResponse {
    JsonRpcResponse::success(id, json!("Astram/v0.1.0/rust"))
}

/// Create the Ethereum JSON-RPC server
pub fn eth_rpc_routes(
    node: NodeHandle,
) -> impl Filter<Extract = impl Reply, Error = warp::Rejection> + Clone {
    let node_filter = warp::any().map(move || node.clone());

    // CORS configuration for MetaMask
    let cors = warp::cors()
        .allow_any_origin()
        .allow_methods(vec!["GET", "POST", "OPTIONS"])
        .allow_headers(vec!["Content-Type", "Authorization"]);

    warp::post()
        .and(warp::path::end())
        .and(warp::body::json())
        .and(node_filter)
        .and_then(handle_rpc)
        .with(cors)
        .with(warp::log("Astram::eth_rpc"))
}

/// Run the Ethereum JSON-RPC server on port 8545 (standard Ethereum port)
pub async fn run_eth_rpc_server(node: NodeHandle) {
    let routes = eth_rpc_routes(node);

    let addr = ([127, 0, 0, 1], 8545);
    println!("[INFO] Ethereum JSON-RPC server running at http://127.0.0.1:8545");
    println!("   Chain ID: 8888 (0x22b8)");
    println!("   Ready for MetaMask connection!");
    println!("   [INFO] CORS enabled for browser access");

    warp::serve(routes).run(addr).await;
}

