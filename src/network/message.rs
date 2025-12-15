//! Network message types.

use super::peer::PeerId;
use crate::types::event::DagEvent;
use crate::types::transaction::EncryptedTransaction;
use crate::types::{EventId, ValidatorId};
use serde::{Deserialize, Serialize};

/// Network message types.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum MessageType {
    // === Handshake ===
    /// Initial hello message
    Hello(HelloMessage),
    /// Response to hello
    HelloAck(HelloAckMessage),

    // === DAG Gossip ===
    /// Announce new event
    EventAnnounce(EventAnnounce),
    /// Request event by ID
    EventRequest(EventRequest),
    /// Response with event data
    EventResponse(EventResponse),

    // === Transaction Pool ===
    /// Broadcast new transaction
    TransactionBroadcast(TransactionBroadcast),

    // === Sync ===
    /// Request events from a round range
    SyncRequest(SyncRequest),
    /// Response with events
    SyncResponse(SyncResponse),

    // === Peer Discovery ===
    /// Request peer list
    GetPeers,
    /// Response with known peers
    Peers(PeersMessage),

    // === Utility ===
    /// Ping for liveness
    Ping(u64),
    /// Pong response
    Pong(u64),

    // === Threshold Encryption ===
    /// Share a decryption key for a committed batch
    DecryptionShare(DecryptionShareMessage),
}

/// Share a decryption key for a committed batch.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DecryptionShareMessage {
    /// ID of the event containing the encrypted batch
    pub event_id: EventId,

    /// The secret share for reconstruction
    pub share: crate::crypto::threshold::SecretShare,
}

/// Complete network message with header.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NetworkMessage {
    /// Protocol version
    pub version: u8,

    /// Message ID for deduplication
    pub message_id: [u8; 16],

    /// Sender peer ID
    pub sender: PeerId,

    /// Message payload
    pub payload: MessageType,
}

impl NetworkMessage {
    /// Create a new message.
    pub fn new(sender: PeerId, payload: MessageType) -> Self {
        let mut message_id = [0u8; 16];
        rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut message_id);

        Self {
            version: 1,
            message_id,
            sender,
            payload,
        }
    }

    /// Serialize to bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        bincode::serialize(self).unwrap_or_default()
    }

    /// Deserialize from bytes.
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        bincode::deserialize(bytes).ok()
    }
}

// === Message Payloads ===

/// Hello handshake message.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HelloMessage {
    /// Protocol version
    pub version: u8,

    /// Chain ID
    pub chain_id: u64,

    /// Genesis hash
    pub genesis_hash: [u8; 32],

    /// Our validator ID (if validator)
    pub validator_id: Option<ValidatorId>,

    /// Latest known round
    pub latest_round: u64,
}

/// Hello acknowledgment.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HelloAckMessage {
    /// Accept or reject
    pub accepted: bool,

    /// Rejection reason (if any)
    pub reason: Option<String>,

    /// Their latest round
    pub latest_round: u64,
}

/// Announce a new event.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EventAnnounce {
    /// Event ID
    pub event_id: EventId,

    /// Event creator
    pub creator: ValidatorId,

    /// Event round
    pub round: u64,
}

/// Request an event by ID.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EventRequest {
    /// Requested event IDs
    pub event_ids: Vec<EventId>,
}

/// Response with event data.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EventResponse {
    /// Requested events (may be partial)
    pub events: Vec<DagEvent>,

    /// Missing event IDs
    pub missing: Vec<EventId>,
}

/// Broadcast a transaction.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TransactionBroadcast {
    /// The transaction
    pub transaction: EncryptedTransaction,
}

/// Request events in a round range.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SyncRequest {
    /// Start round (inclusive)
    pub from_round: u64,

    /// End round (inclusive)
    pub to_round: u64,

    /// Maximum events to return
    pub max_events: usize,
}

/// Response with synced events.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SyncResponse {
    /// Events in the requested range
    pub events: Vec<DagEvent>,

    /// Whether more events are available
    pub has_more: bool,

    /// Next round to request (if has_more)
    pub next_round: u64,
}

/// List of known peers.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PeersMessage {
    /// Known peer addresses
    pub peers: Vec<PeerAddress>,
}

/// Peer address info.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PeerAddress {
    /// Peer ID
    pub id: PeerId,

    /// IP address
    pub ip: String,

    /// Port
    pub port: u16,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_peer_id() -> PeerId {
        [0x42; 32]
    }

    #[test]
    fn test_message_serialization() {
        let msg = NetworkMessage::new(make_peer_id(), MessageType::Ping(12345));

        let bytes = msg.to_bytes();
        let recovered = NetworkMessage::from_bytes(&bytes).unwrap();

        assert_eq!(recovered.sender, msg.sender);
        assert_eq!(recovered.message_id, msg.message_id);
    }

    #[test]
    fn test_hello_message() {
        let hello = HelloMessage {
            version: 1,
            chain_id: 1,
            genesis_hash: [0xAB; 32],
            validator_id: None,
            latest_round: 100,
        };

        let msg = NetworkMessage::new(make_peer_id(), MessageType::Hello(hello));

        let bytes = msg.to_bytes();
        assert!(!bytes.is_empty());

        let recovered = NetworkMessage::from_bytes(&bytes).unwrap();
        match recovered.payload {
            MessageType::Hello(h) => {
                assert_eq!(h.chain_id, 1);
                assert_eq!(h.latest_round, 100);
            }
            _ => panic!("Wrong message type"),
        }
    }

    #[test]
    fn test_unique_message_ids() {
        let id1 = NetworkMessage::new(make_peer_id(), MessageType::Ping(1));
        let id2 = NetworkMessage::new(make_peer_id(), MessageType::Ping(2));

        assert_ne!(id1.message_id, id2.message_id);
    }
}
