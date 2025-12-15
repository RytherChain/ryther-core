//! JSON-RPC HTTP server for Ryther node.

use axum::{
    extract::State,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};
use tracing::info;

use super::blocks::BlockStore;
use super::node::RytherNode;
use crate::types::block::{Block, BlockId, LogFilter};
use crate::types::Address;

/// RPC request.
#[derive(Debug, Deserialize)]
pub struct RpcRequest {
    pub jsonrpc: String,
    pub method: String,
    pub params: Option<Value>,
    pub id: Value,
}

/// RPC response.
#[derive(Debug, Serialize)]
pub struct RpcResponse {
    pub jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
    pub id: Value,
}

/// RPC error.
#[derive(Debug, Serialize)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl RpcResponse {
    pub fn success(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            result: Some(result),
            error: None,
            id,
        }
    }

    pub fn error(id: Value, code: i32, message: String) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            result: None,
            error: Some(RpcError {
                code,
                message,
                data: None,
            }),
            id,
        }
    }
}

/// Standard JSON-RPC error codes.
pub mod error_codes {
    pub const PARSE_ERROR: i32 = -32700;
    pub const INVALID_REQUEST: i32 = -32600;
    pub const METHOD_NOT_FOUND: i32 = -32601;
    pub const INVALID_PARAMS: i32 = -32602;
    pub const INTERNAL_ERROR: i32 = -32603;
}

/// RPC server state.
#[derive(Clone)]
pub struct RpcState {
    pub node: Arc<RytherNode>,
    pub blocks: Arc<BlockStore>,
}

/// Start the RPC HTTP server.
pub async fn start_rpc_server(
    node: Arc<RytherNode>,
    blocks: Arc<BlockStore>,
    addr: SocketAddr,
) -> Result<(), String> {
    let state = RpcState { node, blocks };

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = Router::new()
        .route("/", post(handle_rpc))
        .route("/metrics", get(handle_metrics))
        .layer(cors)
        .with_state(state);

    info!("Starting RPC server on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| format!("Failed to bind RPC server: {}", e))?;

    axum::serve(listener, app)
        .await
        .map_err(|e| format!("RPC server error: {}", e))?;

    Ok(())
}

/// Handle RPC request.
async fn handle_rpc(
    State(state): State<RpcState>,
    Json(request): Json<RpcRequest>,
) -> Json<RpcResponse> {
    let response = process_request(&state, request).await;
    Json(response)
}

/// Process an RPC request.
async fn process_request(state: &RpcState, request: RpcRequest) -> RpcResponse {
    let params = request.params.clone();

    match request.method.as_str() {
        // === Ethereum-compatible methods ===
        "eth_chainId" => eth_chain_id(request.id),
        "eth_blockNumber" => eth_block_number(state, request.id).await,
        "eth_gasPrice" => eth_gas_price(request.id),
        "eth_getBalance" => eth_get_balance(request.id),
        "eth_getTransactionCount" => eth_get_transaction_count(request.id),
        "eth_sendRawTransaction" => eth_send_raw_transaction(request.id),
        "eth_call" => eth_call(request.id),
        "eth_estimateGas" => eth_estimate_gas(request.id),

        // === Block methods ===
        "eth_getBlockByNumber" => eth_get_block_by_number(state, request.id, params).await,
        "eth_getBlockByHash" => eth_get_block_by_hash(state, request.id, params).await,
        "eth_getTransactionByHash" => eth_get_transaction_by_hash(state, request.id, params).await,
        "eth_getTransactionReceipt" => eth_get_transaction_receipt(state, request.id, params).await,
        "eth_getLogs" => eth_get_logs(state, request.id, params).await,
        "eth_getCode" => eth_get_code(request.id),
        "eth_getStorageAt" => eth_get_storage_at(request.id),

        // === Ryther-specific methods ===
        "ryther_nodeInfo" => ryther_node_info(state, request.id).await,
        "ryther_dagInfo" => ryther_dag_info(state, request.id).await,
        "ryther_getPeers" => ryther_get_peers(state, request.id).await,
        "ryther_poolStatus" => ryther_pool_status(request.id),

        // === Network methods ===
        "net_version" => net_version(request.id),
        "net_peerCount" => net_peer_count(state, request.id).await,
        "net_listening" => net_listening(request.id),

        // === Web3 methods ===
        "web3_clientVersion" => web3_client_version(request.id),
        "web3_sha3" => web3_sha3(request.id, params),

        _ => RpcResponse::error(
            request.id,
            error_codes::METHOD_NOT_FOUND,
            format!("Method not found: {}", request.method),
        ),
    }
}

// === Ethereum-compatible implementations ===

fn eth_chain_id(id: Value) -> RpcResponse {
    RpcResponse::success(id, json!("0x1"))
}

async fn eth_block_number(state: &RpcState, id: Value) -> RpcResponse {
    let latest = state.blocks.latest_number();
    RpcResponse::success(id, json!(format!("0x{:x}", latest)))
}

fn eth_gas_price(id: Value) -> RpcResponse {
    RpcResponse::success(id, json!("0x3b9aca00")) // 1 gwei
}

fn eth_get_balance(id: Value) -> RpcResponse {
    RpcResponse::success(id, json!("0x0"))
}

fn eth_get_transaction_count(id: Value) -> RpcResponse {
    RpcResponse::success(id, json!("0x0"))
}

fn eth_send_raw_transaction(id: Value) -> RpcResponse {
    RpcResponse::error(
        id,
        error_codes::INTERNAL_ERROR,
        "Not implemented".to_string(),
    )
}

fn eth_call(id: Value) -> RpcResponse {
    RpcResponse::success(id, json!("0x"))
}

fn eth_estimate_gas(id: Value) -> RpcResponse {
    RpcResponse::success(id, json!("0x5208")) // 21000
}

fn eth_get_code(id: Value) -> RpcResponse {
    RpcResponse::success(id, json!("0x"))
}

fn eth_get_storage_at(id: Value) -> RpcResponse {
    RpcResponse::success(
        id,
        json!("0x0000000000000000000000000000000000000000000000000000000000000000"),
    )
}

// === Block methods ===

async fn eth_get_block_by_number(
    state: &RpcState,
    id: Value,
    params: Option<Value>,
) -> RpcResponse {
    let (block_id, _full_tx) = match parse_block_params(params) {
        Ok(p) => p,
        Err(e) => return RpcResponse::error(id, error_codes::INVALID_PARAMS, e),
    };

    let block_num = match block_id {
        BlockId::Number(n) => n,
        BlockId::Tag(tag) if tag == "latest" => state.blocks.latest_number(),
        BlockId::Tag(tag) if tag == "earliest" => 0,
        BlockId::Tag(tag) if tag == "pending" => state.blocks.latest_number(),
        _ => {
            return RpcResponse::error(
                id,
                error_codes::INVALID_PARAMS,
                "Invalid block id".to_string(),
            )
        }
    };

    match state.blocks.get_by_number(block_num) {
        Some(block) => {
            RpcResponse::success(id, serde_json::to_value(&*block).unwrap_or(json!(null)))
        }
        None => RpcResponse::success(id, json!(null)),
    }
}

async fn eth_get_block_by_hash(state: &RpcState, id: Value, params: Option<Value>) -> RpcResponse {
    let hash = match parse_hash_param(params) {
        Ok(h) => h,
        Err(e) => return RpcResponse::error(id, error_codes::INVALID_PARAMS, e),
    };

    match state.blocks.get_by_hash(&hash) {
        Some(block) => {
            RpcResponse::success(id, serde_json::to_value(&*block).unwrap_or(json!(null)))
        }
        None => RpcResponse::success(id, json!(null)),
    }
}

async fn eth_get_transaction_by_hash(
    state: &RpcState,
    id: Value,
    params: Option<Value>,
) -> RpcResponse {
    let hash = match parse_hash_param(params) {
        Ok(h) => h,
        Err(e) => return RpcResponse::error(id, error_codes::INVALID_PARAMS, e),
    };

    // Check if tx exists in any block
    if let Some(block_num) = state.blocks.get_tx_block(&hash) {
        if let Some(block) = state.blocks.get_by_number(block_num) {
            for (i, tx) in block.transactions.iter().enumerate() {
                if tx.0 == hash {
                    return RpcResponse::success(
                        id,
                        json!({
                            "hash": format!("0x{}", hex::encode(&hash)),
                            "blockHash": format!("0x{}", hex::encode(&block.hash)),
                            "blockNumber": format!("0x{:x}", block.number),
                            "transactionIndex": format!("0x{:x}", i),
                        }),
                    );
                }
            }
        }
    }

    RpcResponse::success(id, json!(null))
}

async fn eth_get_transaction_receipt(
    state: &RpcState,
    id: Value,
    params: Option<Value>,
) -> RpcResponse {
    let hash = match parse_hash_param(params) {
        Ok(h) => h,
        Err(e) => return RpcResponse::error(id, error_codes::INVALID_PARAMS, e),
    };

    match state.blocks.get_receipt(&hash) {
        Some(receipt) => {
            RpcResponse::success(id, serde_json::to_value(&receipt).unwrap_or(json!(null)))
        }
        None => RpcResponse::success(id, json!(null)),
    }
}

async fn eth_get_logs(state: &RpcState, id: Value, params: Option<Value>) -> RpcResponse {
    let filter = match parse_log_filter(params) {
        Ok(f) => f,
        Err(e) => return RpcResponse::error(id, error_codes::INVALID_PARAMS, e),
    };

    let latest = state.blocks.latest_number();
    let from_block = match filter.from_block {
        Some(BlockId::Number(n)) => n,
        Some(BlockId::Tag(ref t)) if t == "latest" => latest,
        Some(BlockId::Tag(ref t)) if t == "earliest" => 0,
        _ => 0,
    };
    let to_block = match filter.to_block {
        Some(BlockId::Number(n)) => n,
        Some(BlockId::Tag(ref t)) if t == "latest" => latest,
        Some(BlockId::Tag(ref t)) if t == "earliest" => 0,
        _ => latest,
    };

    let addresses = filter.address.as_ref().map(|v| v.as_slice());
    let topics: Option<Vec<Option<Vec<[u8; 32]>>>> = filter.topics.as_ref().map(|t| {
        t.iter()
            .map(|opt| opt.as_ref().map(|v| v.iter().map(|h| h.0).collect()))
            .collect()
    });

    let logs = state
        .blocks
        .get_logs(from_block, to_block, addresses, topics.as_deref());

    RpcResponse::success(id, serde_json::to_value(&logs).unwrap_or(json!([])))
}

// === Helper functions ===

fn parse_block_params(params: Option<Value>) -> Result<(BlockId, bool), String> {
    let arr = match params {
        Some(Value::Array(a)) => a,
        _ => return Ok((BlockId::Tag("latest".to_string()), false)),
    };

    let block_id = match arr.get(0) {
        Some(Value::String(s)) => {
            if s.starts_with("0x") && s.len() == 66 {
                // Hash
                let bytes = hex::decode(s.trim_start_matches("0x")).map_err(|_| "Invalid hash")?;
                let mut hash = [0u8; 32];
                hash.copy_from_slice(&bytes);
                BlockId::Hash(crate::types::block::TxHash(hash))
            } else if s.starts_with("0x") {
                // Number
                let n = u64::from_str_radix(s.trim_start_matches("0x"), 16)
                    .map_err(|_| "Invalid number")?;
                BlockId::Number(n)
            } else {
                // Tag
                BlockId::Tag(s.clone())
            }
        }
        Some(Value::Number(n)) => BlockId::Number(n.as_u64().unwrap_or(0)),
        _ => BlockId::Tag("latest".to_string()),
    };

    let full_tx = arr.get(1).and_then(|v| v.as_bool()).unwrap_or(false);

    Ok((block_id, full_tx))
}

fn parse_hash_param(params: Option<Value>) -> Result<[u8; 32], String> {
    let arr = match params {
        Some(Value::Array(a)) => a,
        _ => return Err("Missing params".to_string()),
    };

    let hash_str = arr.get(0).and_then(|v| v.as_str()).ok_or("Missing hash")?;

    let bytes =
        hex::decode(hash_str.trim_start_matches("0x")).map_err(|_| "Invalid hash format")?;

    if bytes.len() != 32 {
        return Err("Hash must be 32 bytes".to_string());
    }

    let mut hash = [0u8; 32];
    hash.copy_from_slice(&bytes);
    Ok(hash)
}

fn parse_log_filter(params: Option<Value>) -> Result<LogFilter, String> {
    let arr = match params {
        Some(Value::Array(a)) => a,
        _ => return Ok(LogFilter::default()),
    };

    let filter_obj = arr.get(0).cloned().unwrap_or(json!({}));
    serde_json::from_value(filter_obj).map_err(|e| format!("Invalid filter: {}", e))
}

// === Ryther-specific implementations ===

async fn ryther_node_info(state: &RpcState, id: Value) -> RpcResponse {
    let stats = state.node.stats().await;
    let node_state = state.node.state().await;

    RpcResponse::success(
        id,
        json!({
            "peerId": hex::encode(state.node.peer_id()),
            "state": format!("{:?}", node_state),
            "currentRound": stats.current_round,
            "eventsCreated": stats.events_created,
            "eventsReceived": stats.events_received,
            "roundsCommitted": stats.rounds_committed,
            "transactionsProcessed": stats.transactions_processed,
            "connectedPeers": stats.connected_peers,
            "blocksStored": state.blocks.block_count(),
            "version": "0.1.0"
        }),
    )
}

async fn ryther_dag_info(state: &RpcState, id: Value) -> RpcResponse {
    let stats = state.node.stats().await;

    RpcResponse::success(
        id,
        json!({
            "maxRound": stats.current_round,
            "totalEvents": stats.events_received + stats.events_created,
            "latestBlock": state.blocks.latest_number(),
        }),
    )
}

async fn ryther_get_peers(state: &RpcState, id: Value) -> RpcResponse {
    let stats = state.node.stats().await;

    RpcResponse::success(
        id,
        json!({
            "count": stats.connected_peers,
            "peers": []
        }),
    )
}

fn ryther_pool_status(id: Value) -> RpcResponse {
    RpcResponse::success(
        id,
        json!({
            "pending": 0,
            "queued": 0,
        }),
    )
}

// === Network implementations ===

fn net_version(id: Value) -> RpcResponse {
    RpcResponse::success(id, json!("1"))
}

async fn net_peer_count(state: &RpcState, id: Value) -> RpcResponse {
    let stats = state.node.stats().await;
    RpcResponse::success(id, json!(format!("0x{:x}", stats.connected_peers)))
}

fn net_listening(id: Value) -> RpcResponse {
    RpcResponse::success(id, json!(true))
}

// === Web3 implementations ===

fn web3_client_version(id: Value) -> RpcResponse {
    RpcResponse::success(id, json!("Ryther/v0.1.0"))
}

fn web3_sha3(id: Value, params: Option<Value>) -> RpcResponse {
    if let Some(Value::Array(arr)) = params {
        if let Some(Value::String(data)) = arr.first() {
            if let Ok(bytes) = hex::decode(data.trim_start_matches("0x")) {
                let hash = crate::types::keccak256(&bytes);
                return RpcResponse::success(id, json!(format!("0x{}", hex::encode(hash))));
            }
        }
    }
    RpcResponse::error(id, error_codes::INVALID_PARAMS, "Invalid input".to_string())
}

/// Handle metrics request (Prometheus format)
async fn handle_metrics(State(state): State<RpcState>) -> String {
    let stats = state.node.stats().await;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let uptime = now.saturating_sub(stats.start_time).max(1);
    let tps = stats.transactions_processed as f64 / uptime as f64;

    format!(
        "# HELP ryther_current_round Current DAG round\n\
         # TYPE ryther_current_round gauge\n\
         ryther_current_round {}\n\
         # HELP ryther_transactions_total Total transactions processed\n\
         # TYPE ryther_transactions_total counter\n\
         ryther_transactions_total {}\n\
         # HELP ryther_tps Transactions per second (average)\n\
         # TYPE ryther_tps gauge\n\
         ryther_tps {:.2}\n\
         # HELP ryther_peers Connected peers\n\
         # TYPE ryther_peers gauge\n\
         ryther_peers {}\n",
        stats.current_round, stats.transactions_processed, tps, stats.connected_peers
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rpc_response_success() {
        let response = RpcResponse::success(json!(1), json!("result"));

        assert!(response.result.is_some());
        assert!(response.error.is_none());
        assert_eq!(response.jsonrpc, "2.0");
    }

    #[test]
    fn test_rpc_response_error() {
        let response = RpcResponse::error(json!(1), -32600, "Invalid Request".to_string());

        assert!(response.result.is_none());
        assert!(response.error.is_some());
        assert_eq!(response.error.unwrap().code, -32600);
    }

    #[test]
    fn test_parse_hash_param() {
        let params = Some(json!([
            "0x0000000000000000000000000000000000000000000000000000000000000000"
        ]));
        let result = parse_hash_param(params);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), [0u8; 32]);
    }

    #[test]
    fn test_parse_block_params() {
        let params = Some(json!(["latest", true]));
        let result = parse_block_params(params);
        assert!(result.is_ok());
        let (block_id, full_tx) = result.unwrap();
        assert!(matches!(block_id, BlockId::Tag(_)));
        assert!(full_tx);
    }

    #[tokio::test]
    async fn test_eth_chain_id() {
        let response = eth_chain_id(json!(1));
        assert!(response.result.is_some());
    }

    #[tokio::test]
    async fn test_web3_client_version() {
        let response = web3_client_version(json!(1));
        assert!(response.result.is_some());
        assert!(response
            .result
            .unwrap()
            .as_str()
            .unwrap()
            .contains("Ryther"));
    }
}
