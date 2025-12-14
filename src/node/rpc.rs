//! JSON-RPC HTTP server for Ryther node.

use std::net::SocketAddr;
use std::sync::Arc;
use axum::{
    routing::post,
    Router,
    Json,
    extract::State,
    http::StatusCode,
};
use tower_http::cors::{CorsLayer, Any};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tracing::{info, error};

use super::node::RytherNode;

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
}

/// Start the RPC HTTP server.
pub async fn start_rpc_server(node: Arc<RytherNode>, addr: SocketAddr) -> Result<(), String> {
    let state = RpcState { node };
    
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);
    
    let app = Router::new()
        .route("/", post(handle_rpc))
        .layer(cors)
        .with_state(state);
    
    info!("Starting RPC server on {}", addr);
    
    let listener = tokio::net::TcpListener::bind(addr).await
        .map_err(|e| format!("Failed to bind RPC server: {}", e))?;
    
    axum::serve(listener, app).await
        .map_err(|e| format!("RPC server error: {}", e))?;
    
    Ok(())
}

/// Handle RPC request.
async fn handle_rpc(
    State(state): State<RpcState>,
    Json(request): Json<RpcRequest>,
) -> Json<RpcResponse> {
    let response = process_request(&state.node, request).await;
    Json(response)
}

/// Process an RPC request.
async fn process_request(node: &RytherNode, request: RpcRequest) -> RpcResponse {
    match request.method.as_str() {
        // === Ethereum-compatible methods ===
        "eth_chainId" => eth_chain_id(request.id),
        "eth_blockNumber" => eth_block_number(node, request.id).await,
        "eth_gasPrice" => eth_gas_price(request.id),
        "eth_getBalance" => eth_get_balance(request.id),
        "eth_getTransactionCount" => eth_get_transaction_count(request.id),
        "eth_sendRawTransaction" => eth_send_raw_transaction(request.id),
        "eth_call" => eth_call(request.id),
        "eth_estimateGas" => eth_estimate_gas(request.id),
        
        // === Ryther-specific methods ===
        "ryther_nodeInfo" => ryther_node_info(node, request.id).await,
        "ryther_dagInfo" => ryther_dag_info(node, request.id).await,
        "ryther_getPeers" => ryther_get_peers(node, request.id).await,
        "ryther_poolStatus" => ryther_pool_status(node, request.id).await,
        
        // === Network methods ===
        "net_version" => net_version(request.id),
        "net_peerCount" => net_peer_count(node, request.id).await,
        "net_listening" => net_listening(request.id),
        
        // === Web3 methods ===
        "web3_clientVersion" => web3_client_version(request.id),
        "web3_sha3" => web3_sha3(request.id, request.params),
        
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

async fn eth_block_number(node: &RytherNode, id: Value) -> RpcResponse {
    let stats = node.stats().await;
    RpcResponse::success(id, json!(format!("0x{:x}", stats.current_round)))
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
    RpcResponse::error(id, error_codes::INTERNAL_ERROR, "Not implemented".to_string())
}

fn eth_call(id: Value) -> RpcResponse {
    RpcResponse::success(id, json!("0x"))
}

fn eth_estimate_gas(id: Value) -> RpcResponse {
    RpcResponse::success(id, json!("0x5208")) // 21000
}

// === Ryther-specific implementations ===

async fn ryther_node_info(node: &RytherNode, id: Value) -> RpcResponse {
    let stats = node.stats().await;
    let state = node.state().await;
    
    RpcResponse::success(id, json!({
        "peerId": hex::encode(node.peer_id()),
        "state": format!("{:?}", state),
        "currentRound": stats.current_round,
        "eventsCreated": stats.events_created,
        "eventsReceived": stats.events_received,
        "roundsCommitted": stats.rounds_committed,
        "transactionsProcessed": stats.transactions_processed,
        "connectedPeers": stats.connected_peers,
        "version": "0.1.0"
    }))
}

async fn ryther_dag_info(node: &RytherNode, id: Value) -> RpcResponse {
    let stats = node.stats().await;
    
    RpcResponse::success(id, json!({
        "maxRound": stats.current_round,
        "totalEvents": stats.events_received + stats.events_created,
    }))
}

async fn ryther_get_peers(node: &RytherNode, id: Value) -> RpcResponse {
    let stats = node.stats().await;
    
    RpcResponse::success(id, json!({
        "count": stats.connected_peers,
        "peers": []
    }))
}

async fn ryther_pool_status(node: &RytherNode, id: Value) -> RpcResponse {
    RpcResponse::success(id, json!({
        "pending": 0,
        "queued": 0,
    }))
}

// === Network implementations ===

fn net_version(id: Value) -> RpcResponse {
    RpcResponse::success(id, json!("1"))
}

async fn net_peer_count(node: &RytherNode, id: Value) -> RpcResponse {
    let stats = node.stats().await;
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

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::config::NodeConfig;
    
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
    
    #[tokio::test]
    async fn test_eth_chain_id() {
        let response = eth_chain_id(json!(1));
        assert!(response.result.is_some());
    }
    
    #[tokio::test]
    async fn test_web3_client_version() {
        let response = web3_client_version(json!(1));
        assert!(response.result.is_some());
        assert!(response.result.unwrap().as_str().unwrap().contains("Ryther"));
    }
}
