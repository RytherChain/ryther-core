//! P2P Networking Layer for Ryther Protocol.
//!
//! Implements peer-to-peer communication for:
//! - Event gossip (DAG propagation)
//! - Transaction broadcast
//! - Peer discovery and management

pub mod gossip;
pub mod message;
pub mod peer;
pub mod transport;

pub use gossip::GossipProtocol;
pub use message::{DecryptionShareMessage, MessageType, NetworkMessage};
pub use peer::{PeerId, PeerInfo, PeerManager};
pub use transport::TcpTransport;
