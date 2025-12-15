//! Ryther Protocol Core Library
//!
//! A high-fidelity asynchronous Layer 1 blockchain utilizing:
//! - Helix DAG Consensus (leaderless aBFT)
//! - Native Parallel EVM (RytherVM)
//! - Async I/O State Storage (RytherDB)
//! - Gossip-based P2P Networking

pub mod consensus;
pub mod crypto;
pub mod dag;
pub mod evm;
pub mod execution;
pub mod network;
pub mod node;
pub mod storage;
pub mod types;

// Re-export core types for convenience
pub use evm::{RytherVm, VmError, VmResult};
pub use network::{GossipProtocol, PeerId, PeerManager, TcpTransport};
pub use node::{NodeConfig, RytherNode, TransactionPool};
pub use types::lamport::LamportClock;
pub use types::{Address, BlsSignature, EventId, StateKey, ValidatorId, U256};
