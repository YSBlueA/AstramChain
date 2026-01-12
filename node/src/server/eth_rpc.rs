/// Ethereum-compatible JSON-RPC server for MetaMask integration
use crate::NodeHandle;
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
    // Chain ID for NetCoin (use a unique ID, e.g., 8888)
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

            // natoshi and wei are now the same (both 10^18 decimals)
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
    _params: Option<Vec<Value>>,
    _node: NodeHandle,
) -> JsonRpcResponse {
    // TODO: Implement transaction parsing and broadcasting
    // For now, return a mock transaction hash
    let mock_txid = "0x0000000000000000000000000000000000000000000000000000000000000000";
    JsonRpcResponse::success(id, json!(mock_txid))
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
            if let Ok(Some((tx, block_height))) = state.bc.get_transaction(tx_hash) {
                // natoshi and wei are now the same (both 10^18 decimals)
                let amount = tx
                    .outputs
                    .get(0)
                    .map(|o| o.amount())
                    .unwrap_or_else(U256::zero);

                // Convert to Ethereum transaction format
                return JsonRpcResponse::success(
                    id,
                    json!({
                        "hash": format!("0x{}", tx.txid),
                        "nonce": "0x0",
                        "blockHash": null, // Would need block hash
                        "blockNumber": format!("0x{:x}", block_height),
                        "transactionIndex": "0x0",
                        "from": tx.inputs.get(0).map(|i| &i.pubkey).unwrap_or(&String::new()).clone(),
                        "to": tx.outputs.get(0).map(|o| &o.to).unwrap_or(&String::new()).clone(),
                        "value": format!("0x{:x}", amount),
                        "gasPrice": "0x0",
                        "gas": "0x0",
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

            let state = node.lock().unwrap();
            if let Ok(Some((tx, block_height))) = state.bc.get_transaction(tx_hash) {
                return JsonRpcResponse::success(
                    id,
                    json!({
                        "transactionHash": format!("0x{}", tx.txid),
                        "transactionIndex": "0x0",
                        "blockHash": null,
                        "blockNumber": format!("0x{:x}", block_height),
                        "from": tx.inputs.get(0).map(|i| &i.pubkey).unwrap_or(&String::new()).clone(),
                        "to": tx.outputs.get(0).map(|o| &o.to).unwrap_or(&String::new()).clone(),
                        "cumulativeGasUsed": "0x0",
                        "gasUsed": "0x0",
                        "contractAddress": null,
                        "logs": [],
                        "status": "0x1", // success
                    }),
                );
            }
        }
    }

    JsonRpcResponse::success(id, json!(null))
}

fn eth_gas_price(id: Value) -> JsonRpcResponse {
    // Fixed gas price (can be made dynamic)
    JsonRpcResponse::success(id, json!("0x0")) // 0 gas price for now
}

fn eth_estimate_gas(id: Value) -> JsonRpcResponse {
    // Fixed gas estimate
    JsonRpcResponse::success(id, json!("0x5208")) // 21000 gas (standard transfer)
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
    JsonRpcResponse::success(id, json!("NetCoin/v0.1.0/rust"))
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
        .with(warp::log("netcoin::eth_rpc"))
}

/// Run the Ethereum JSON-RPC server on port 8545 (standard Ethereum port)
pub async fn run_eth_rpc_server(node: NodeHandle) {
    let routes = eth_rpc_routes(node);

    let addr = ([127, 0, 0, 1], 8545);
    println!("🦊 Ethereum JSON-RPC server running at http://127.0.0.1:8545");
    println!("   Chain ID: 8888 (0x22b8)");
    println!("   Ready for MetaMask connection!");
    println!("   ✅ CORS enabled for browser access");

    warp::serve(routes).run(addr).await;
}
