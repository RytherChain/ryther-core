//! Ryther full node implementation.

use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use tracing::{info, warn, error};

use crate::types::{ValidatorId, EventId};
use crate::types::event::DagEvent;
use crate::types::validator::ValidatorSet;
use crate::types::lamport::LamportClock;
use crate::crypto::bls::SecretKey;
use crate::dag::DagStore;
use crate::consensus::leader::elect_leader;
use crate::consensus::commit::{CommitDetector, CommitResult};
use crate::execution::mvcc::MultiVersionState;
use crate::execution::executor::{ParallelExecutor, ExecutorConfig};
use crate::evm::RytherVm;
use crate::network::peer::{PeerId, PeerManager};
use crate::network::gossip::{GossipProtocol, GossipConfig};
use crate::network::transport::{TcpTransport, TransportConfig};
use crate::network::message::{NetworkMessage, MessageType};

use super::config::NodeConfig;
use super::mempool::TransactionPool;

/// Node state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeState {
    /// Node is starting up
    Starting,
    
    /// Syncing with network
    Syncing,
    
    /// Fully synced and running
    Running,
    
    /// Shutting down
    Stopping,
    
    /// Stopped
    Stopped,
}

/// Statistics about node operation.
#[derive(Default, Clone, Debug)]
pub struct NodeStats {
    pub events_created: u64,
    pub events_received: u64,
    pub transactions_processed: u64,
    pub rounds_committed: u64,
    pub current_round: u64,
    pub connected_peers: usize,
}

/// Ryther full node.
pub struct RytherNode {
    /// Configuration
    config: NodeConfig,
    
    /// Current state
    state: RwLock<NodeState>,
    
    /// Our peer ID
    peer_id: PeerId,
    
    /// Our validator ID (if validator)
    validator_id: Option<ValidatorId>,
    
    /// Validator secret key (if validator)
    validator_key: Option<SecretKey>,
    
    /// DAG store
    dag: Arc<DagStore>,
    
    /// Validator set
    validators: Arc<RwLock<ValidatorSet>>,
    
    /// Lamport clock
    clock: Arc<LamportClock>,
    
    /// Commit detector
    commit_detector: RwLock<CommitDetector>,
    
    /// MVCC state
    state_db: Arc<MultiVersionState>,
    
    /// EVM executor
    vm: Arc<RytherVm>,
    
    /// Transaction pool
    mempool: Arc<TransactionPool>,
    
    /// Peer manager
    peers: Arc<PeerManager>,
    
    /// Gossip protocol
    gossip: Arc<GossipProtocol>,
    
    /// Network transport
    transport: Arc<TcpTransport>,
    
    /// Statistics
    stats: RwLock<NodeStats>,
    
    /// Shutdown signal
    shutdown_tx: mpsc::Sender<()>,
    shutdown_rx: RwLock<Option<mpsc::Receiver<()>>>,
}

impl RytherNode {
    /// Create a new node.
    pub fn new(config: NodeConfig) -> Self {
        // Generate peer ID
        let peer_id = Self::generate_peer_id();
        
        // Initialize components
        let dag = Arc::new(DagStore::new());
        let validators = Arc::new(RwLock::new(ValidatorSet::new()));
        let clock = Arc::new(LamportClock::new());
        let state_db = Arc::new(MultiVersionState::new());
        let vm = Arc::new(RytherVm::new(config.chain.chain_id));
        let mempool = Arc::new(TransactionPool::new(10000));
        
        // Network components
        let peers = Arc::new(PeerManager::new(peer_id, config.network.max_peers));
        
        let gossip = Arc::new(GossipProtocol::new(
            peer_id,
            Arc::clone(&peers),
            Arc::clone(&dag),
            GossipConfig::default(),
        ));
        
        let transport = Arc::new(TcpTransport::new(
            peer_id,
            Arc::clone(&peers),
            TransportConfig {
                listen_addr: config.network.listen_addr,
                ..Default::default()
            },
        ));
        
        // Commit detector
        let commit_detector = CommitDetector::new(ValidatorSet::new(), [0u8; 32]);
        
        let (shutdown_tx, shutdown_rx) = mpsc::channel(1);
        
        Self {
            config,
            state: RwLock::new(NodeState::Starting),
            peer_id,
            validator_id: None,
            validator_key: None,
            dag,
            validators,
            clock,
            commit_detector: RwLock::new(commit_detector),
            state_db,
            vm,
            mempool,
            peers,
            gossip,
            transport,
            stats: RwLock::new(NodeStats::default()),
            shutdown_tx,
            shutdown_rx: RwLock::new(Some(shutdown_rx)),
        }
    }
    
    /// Generate a random peer ID.
    fn generate_peer_id() -> PeerId {
        use rand::RngCore;
        let mut id = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut id);
        id
    }
    
    /// Get current node state.
    pub async fn state(&self) -> NodeState {
        *self.state.read().await
    }
    
    /// Get node statistics.
    pub async fn stats(&self) -> NodeStats {
        let mut stats = self.stats.read().await.clone();
        stats.connected_peers = self.peers.connected_count();
        stats
    }
    
    /// Get our peer ID.
    pub fn peer_id(&self) -> PeerId {
        self.peer_id
    }
    
    /// Start the node.
    pub async fn start(&self) -> Result<(), String> {
        info!("Starting Ryther node...");
        
        *self.state.write().await = NodeState::Syncing;
        
        // Start network listener
        let transport = Arc::clone(&self.transport);
        tokio::spawn(async move {
            if let Err(e) = transport.listen().await {
                error!("Network listener error: {}", e);
            }
        });
        
        // Connect to bootstrap peers
        self.connect_bootstrap_peers().await;
        
        // Start message handler
        self.start_message_handler().await;
        
        // Start block producer (if validator)
        if self.config.is_validator() {
            self.start_block_producer().await;
        }
        
        *self.state.write().await = NodeState::Running;
        info!("Ryther node started successfully");
        
        Ok(())
    }
    
    /// Connect to bootstrap peers.
    async fn connect_bootstrap_peers(&self) {
        for peer_addr in &self.config.network.bootstrap_peers {
            if let Ok(addr) = peer_addr.parse() {
                let peer_id = crate::types::sha256(peer_addr.as_bytes());
                if let Err(e) = self.transport.connect(peer_id, addr).await {
                    warn!("Failed to connect to bootstrap peer {}: {}", peer_addr, e);
                }
            }
        }
    }
    
    /// Start message handler task.
    async fn start_message_handler(&self) {
        let mut receiver = match self.transport.take_receiver() {
            Some(rx) => rx,
            None => return,
        };
        
        let gossip = Arc::clone(&self.gossip);
        let transport = Arc::clone(&self.transport);
        let stats = Arc::new(&self.stats);
        
        tokio::spawn(async move {
            while let Some((peer_id, message)) = receiver.recv().await {
                // Handle the message
                if let Some(response) = gossip.handle_message(&message) {
                    let _ = transport.send(&peer_id, response).await;
                }
            }
        });
    }
    
    /// Start block producer task (validators only).
    async fn start_block_producer(&self) {
        info!("Starting block producer...");
        
        let dag = Arc::clone(&self.dag);
        let clock = Arc::clone(&self.clock);
        let gossip = Arc::clone(&self.gossip);
        let transport = Arc::clone(&self.transport);
        
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_millis(100));
            
            loop {
                interval.tick().await;
                
                // Event creation logic would go here
                // For now, just tick the clock
                clock.tick();
            }
        });
    }
    
    /// Create a new DAG event.
    pub async fn create_event(&self) -> Option<DagEvent> {
        // Get latest events from other validators for parents
        let parents: Vec<EventId> = self.dag
            .events_at_round(self.dag.max_round())
            .iter()
            .map(|e| e.id)
            .collect();
        
        // Tick clock
        let lamport_time = self.clock.tick();
        let round = self.dag.max_round() + 1;
        
        // Get pending transactions from mempool
        let pending = self.mempool.best(1000);
        
        // Build event (placeholder - need validator key)
        // In real implementation, this would sign the event
        
        None // Placeholder
    }
    
    /// Process committed round.
    pub async fn process_commit(&self, commit: CommitResult) {
        info!("Processing commit for round {}", commit.round);
        
        // Get all events to include
        let events = self.dag.events_at_round(commit.round);
        
        // Collect transactions from events
        let mut all_txs = Vec::new();
        for event in events {
            for tx in event.payload.transactions {
                all_txs.push(tx);
            }
        }
        
        // Mark transactions as included
        let commitments: Vec<_> = all_txs.iter().map(|tx| tx.commitment).collect();
        self.mempool.mark_included(&commitments);
        
        // Update stats
        let mut stats = self.stats.write().await;
        stats.rounds_committed += 1;
        stats.current_round = commit.round;
        stats.transactions_processed += all_txs.len() as u64;
    }
    
    /// Stop the node.
    pub async fn stop(&self) {
        info!("Stopping Ryther node...");
        
        *self.state.write().await = NodeState::Stopping;
        
        let _ = self.shutdown_tx.send(()).await;
        
        *self.state.write().await = NodeState::Stopped;
        
        info!("Ryther node stopped");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_node_creation() {
        let config = NodeConfig::default();
        let node = RytherNode::new(config);
        
        assert!(!node.peer_id.iter().all(|&b| b == 0));
    }
    
    #[tokio::test]
    async fn test_node_state() {
        let config = NodeConfig::default();
        let node = RytherNode::new(config);
        
        assert_eq!(node.state().await, NodeState::Starting);
    }
    
    #[tokio::test]
    async fn test_node_stats() {
        let config = NodeConfig::default();
        let node = RytherNode::new(config);
        
        let stats = node.stats().await;
        assert_eq!(stats.events_created, 0);
        assert_eq!(stats.rounds_committed, 0);
    }
}
