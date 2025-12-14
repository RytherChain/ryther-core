//! Ryther Protocol Core Library
//!
//! A high-fidelity asynchronous Layer 1 blockchain utilizing:
//! - Helix DAG Consensus (leaderless aBFT)
//! - Native Parallel EVM (RytherVM)
//! - Async I/O State Storage (RytherDB)
//! - Gossip-based P2P Networking

pub mod types;
pub mod crypto;
pub mod dag;
pub mod consensus;
pub mod execution;
pub mod storage;
pub mod evm;
pub mod network;
pub mod node;

// Re-export core types for convenience
pub use types::{EventId, ValidatorId, BlsSignature, Address, StateKey, U256};
pub use types::lamport::LamportClock;
pub use evm::{RytherVm, VmResult, VmError};
pub use network::{PeerId, PeerManager, GossipProtocol, TcpTransport};
pub use node::{NodeConfig, RytherNode, TransactionPool};
