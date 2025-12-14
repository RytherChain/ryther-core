use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;
use ryther_core::node::{NodeConfig, RytherNode, ValidatorConfig};
use ryther_core::crypto::bls::SecretKey;

pub struct TestNode {
    pub node: Arc<RytherNode>,
    pub rpc_port: u16,
    pub p2p_port: u16,
    pub _temp_dir: TempDir, // Kept to prevent deletion until drop
}

impl TestNode {
    pub async fn start(index: u16, validator: bool, peers: Vec<String>) -> Self {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("db");
        let rpc_port = 20000 + index;
        let p2p_port = 10000 + index;
        
        let mut config = NodeConfig::default();
        config.chain.chain_id = 1337;
        config.network.listen_addr = format!("127.0.0.1:{}", p2p_port).parse().unwrap();
        config.network.bootstrap_peers = peers.into_iter().map(|s| s.parse().unwrap()).collect();
        config.storage.data_dir = db_path;
        config.rpc.enabled = true;
        config.rpc.listen_addr = format!("127.0.0.1:{}", rpc_port).parse().unwrap();
        
        if validator {
            let secret_key = SecretKey::generate();
            
            // Write key to file
            let key_path = temp_dir.path().join("validator.key");
            std::fs::write(&key_path, secret_key.to_bytes()).unwrap();
            
            config.validator = Some(ValidatorConfig {
                key_path,
                produce_blocks: true,
                min_stake: 0,
            });
        }
        
        let node = Arc::new(RytherNode::new(config));
        
        // Start node
        // RytherNode::start() returns quickly after spawning background tasks
        node.start().await.unwrap();
        
        // Let it startup
        tokio::time::sleep(Duration::from_millis(100)).await;
        
        Self {
            node,
            rpc_port,
            p2p_port,
            _temp_dir: temp_dir,
        }
    }
    
    pub async fn stop(&self) {
        self.node.stop().await;
    }
}
