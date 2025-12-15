use ryther_core::crypto::bls::SecretKey;
use ryther_core::node::{NodeConfig, RytherNode, ValidatorConfig};
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;

pub struct TestNode {
    pub node: Arc<RytherNode>,
    pub rpc_port: u16,
    pub p2p_port: u16,
    pub _temp_dir: TempDir, // Kept to prevent deletion until drop
}

use ryther_core::types::{Address, ValidatorId};

// ... existing struct ...

impl TestNode {
    pub async fn start(
        index: u16,
        validator_key: Option<SecretKey>,
        peers: Vec<String>,
        initial_validators: Vec<(ValidatorId, u64, Address)>,
    ) -> Self {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("db");
        let rpc_port = 20000 + index;
        let p2p_port = 10000 + index;

        let mut config = NodeConfig::default();
        config.chain.chain_id = 1337;
        config.chain.initial_validators = initial_validators;
        config.network.listen_addr = format!("127.0.0.1:{}", p2p_port).parse().unwrap();
        config.network.bootstrap_peers = peers.into_iter().map(|s| s.parse().unwrap()).collect();
        config.network.connect_timeout_ms = 200; // Fast fail for local tests
        config.storage.data_dir = db_path;
        config.rpc.enabled = true;
        config.rpc.listen_addr = format!("127.0.0.1:{}", rpc_port).parse().unwrap();

        if let Some(secret_key) = validator_key {
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
        let node_start = Arc::clone(&node);
        node_start.start().await.unwrap();

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
