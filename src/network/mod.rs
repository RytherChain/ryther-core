//! P2P Networking Layer for Ryther Protocol.
//!
//! Implements peer-to-peer communication for:
//! - Event gossip (DAG propagation)
//! - Transaction broadcast
//! - Peer discovery and management

pub mod peer;
pub mod message;
pub mod gossip;
pub mod transport;

pub use peer::{PeerId, PeerInfo, PeerManager};
pub use message::{NetworkMessage, MessageType};
pub use gossip::GossipProtocol;
pub use transport::TcpTransport;
