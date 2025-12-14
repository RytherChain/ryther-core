//! Node configuration.

use std::net::SocketAddr;
use std::path::PathBuf;
use serde::{Deserialize, Serialize};

use crate::network::peer::PeerId;

/// Full node configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeConfig {
    /// Chain configuration
    pub chain: ChainConfig,
    
    /// Network configuration
    pub network: NetworkConfig,
    
    /// Storage configuration
    pub storage: StorageConfig,
    
    /// Execution configuration
    pub execution: ExecutionConfig,
    
    /// RPC configuration
    pub rpc: RpcConfig,
    
    /// Validator configuration (if running as validator)
    pub validator: Option<ValidatorConfig>,
}

impl Default for NodeConfig {
    fn default() -> Self {
        Self {
            chain: ChainConfig::default(),
            network: NetworkConfig::default(),
            storage: StorageConfig::default(),
            execution: ExecutionConfig::default(),
            rpc: RpcConfig::default(),
            validator: None,
        }
    }
}

/// Chain-specific configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChainConfig {
    /// Chain ID
    pub chain_id: u64,
    
    /// Genesis block hash
    pub genesis_hash: [u8; 32],
    
    /// Network name
    pub network_name: String,
}

impl Default for ChainConfig {
    fn default() -> Self {
        Self {
            chain_id: 1,
            genesis_hash: [0; 32],
            network_name: "ryther-mainnet".to_string(),
        }
    }
}

/// Network configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NetworkConfig {
    /// Address to listen on
    pub listen_addr: SocketAddr,
    
    /// Bootstrap peers
    pub bootstrap_peers: Vec<String>,
    
    /// Maximum connected peers
    pub max_peers: usize,
    
    /// Enable peer discovery
    pub enable_discovery: bool,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            listen_addr: "0.0.0.0:30303".parse().unwrap(),
            bootstrap_peers: vec![],
            max_peers: 50,
            enable_discovery: true,
        }
    }
}

/// Storage configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StorageConfig {
    /// Data directory
    pub data_dir: PathBuf,
    
    /// State cache size (MB)
    pub state_cache_mb: usize,
    
    /// Enable pruning
    pub enable_pruning: bool,
    
    /// Blocks to keep when pruning
    pub prune_blocks: u64,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            data_dir: PathBuf::from("./data"),
            state_cache_mb: 512,
            enable_pruning: false,
            prune_blocks: 10000,
        }
    }
}

/// Execution configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExecutionConfig {
    /// Number of parallel execution threads
    pub parallel_threads: usize,
    
    /// Maximum transactions per block
    pub max_txs_per_block: usize,
    
    /// Block gas limit
    pub block_gas_limit: u64,
}

impl Default for ExecutionConfig {
    fn default() -> Self {
        Self {
            parallel_threads: num_cpus::get(),
            max_txs_per_block: 10000,
            block_gas_limit: 30_000_000,
        }
    }
}

/// RPC server configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RpcConfig {
    /// Enable RPC server
    pub enabled: bool,
    
    /// RPC listen address
    pub listen_addr: SocketAddr,
    
    /// Enable WebSocket
    pub enable_ws: bool,
    
    /// WebSocket address
    pub ws_addr: SocketAddr,
    
    /// Allowed origins (CORS)
    pub allowed_origins: Vec<String>,
}

impl Default for RpcConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            listen_addr: "127.0.0.1:8646".parse().unwrap(),
            enable_ws: true,
            ws_addr: "127.0.0.1:8647".parse().unwrap(),
            allowed_origins: vec!["*".to_string()],
        }
    }
}

/// Validator-specific configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ValidatorConfig {
    /// Validator private key path
    pub key_path: PathBuf,
    
    /// Enable block production
    pub produce_blocks: bool,
    
    /// Minimum stake required
    pub min_stake: u64,
}

impl NodeConfig {
    /// Load configuration from file.
    pub fn load(path: &PathBuf) -> Result<Self, String> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read config: {}", e))?;
        
        serde_json::from_str(&content)
            .map_err(|e| format!("Failed to parse config: {}", e))
    }
    
    /// Save configuration to file.
    pub fn save(&self, path: &PathBuf) -> Result<(), String> {
        let content = serde_json::to_string_pretty(self)
            .map_err(|e| format!("Failed to serialize config: {}", e))?;
        
        std::fs::write(path, content)
            .map_err(|e| format!("Failed to write config: {}", e))
    }
    
    /// Check if running as validator.
    pub fn is_validator(&self) -> bool {
        self.validator.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_default_config() {
        let config = NodeConfig::default();
        
        assert_eq!(config.chain.chain_id, 1);
        assert_eq!(config.network.max_peers, 50);
        assert!(config.rpc.enabled);
        assert!(!config.is_validator());
    }
    
    #[test]
    fn test_config_serialization() {
        let config = NodeConfig::default();
        let json = serde_json::to_string(&config).unwrap();
        let recovered: NodeConfig = serde_json::from_str(&json).unwrap();
        
        assert_eq!(config.chain.chain_id, recovered.chain.chain_id);
    }
}
