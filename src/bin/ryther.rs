//! Ryther Node Binary
//!
//! Main entry point for running a Ryther protocol node.

use std::path::PathBuf;
use std::sync::Arc;
use tracing::{info, error, Level};
use tracing_subscriber::FmtSubscriber;

use ryther_core::node::{NodeConfig, RytherNode, start_rpc_server};

#[tokio::main]
async fn main() {
    // Initialize logging
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .with_target(false)
        .with_thread_ids(false)
        .compact()
        .finish();
    
    tracing::subscriber::set_global_default(subscriber)
        .expect("Failed to set tracing subscriber");
    
    println!(r#"
  ____        _   _               
 |  _ \ _   _| |_| |__   ___ _ __ 
 | |_) | | | | __| '_ \ / _ \ '__|
 |  _ <| |_| | |_| | | |  __/ |   
 |_| \_\\__, |\__|_| |_|\___|_|   
        |___/                     
    "#);
    
    info!("Ryther Protocol Node v0.1.0");
    info!("============================");
    
    // Load or create configuration
    let config_path = PathBuf::from("config.json");
    let config = if config_path.exists() {
        match NodeConfig::load(&config_path) {
            Ok(cfg) => {
                info!("Loaded configuration from {}", config_path.display());
                cfg
            }
            Err(e) => {
                error!("Failed to load config: {}", e);
                info!("Using default configuration");
                NodeConfig::default()
            }
        }
    } else {
        info!("No config file found, using defaults");
        let config = NodeConfig::default();
        
        // Save default config for reference
        if let Err(e) = config.save(&config_path) {
            error!("Failed to save default config: {}", e);
        } else {
            info!("Saved default configuration to {}", config_path.display());
        }
        
        config
    };
    
    // Print configuration summary
    info!("Chain ID: {}", config.chain.chain_id);
    info!("Network: {}", config.chain.network_name);
    info!("Listen address: {}", config.network.listen_addr);
    info!("Max peers: {}", config.network.max_peers);
    info!("RPC enabled: {}", config.rpc.enabled);
    if config.rpc.enabled {
        info!("RPC address: {}", config.rpc.listen_addr);
    }
    info!("Validator mode: {}", config.is_validator());
    
    // Create node
    let rpc_addr = config.rpc.listen_addr;
    let rpc_enabled = config.rpc.enabled;
    let node = Arc::new(RytherNode::new(config));
    
    info!("Peer ID: {}", hex::encode(node.peer_id()));
    
    match node.start().await {
        Ok(()) => {
            info!("Node started successfully!");
            
            // Start RPC server if enabled
            if rpc_enabled {
                let rpc_node = Arc::clone(&node);
                tokio::spawn(async move {
                    if let Err(e) = start_rpc_server(rpc_node, rpc_addr).await {
                        error!("RPC server error: {}", e);
                    }
                });
                info!("RPC server started on {}", rpc_addr);
            }
            
            // Wait for shutdown signal
            tokio::signal::ctrl_c().await.expect("Failed to listen for ctrl+c");
            
            info!("Received shutdown signal");
            node.stop().await;
        }
        Err(e) => {
            error!("Failed to start node: {}", e);
            std::process::exit(1);
        }
    }
    
    info!("Goodbye!");
}
