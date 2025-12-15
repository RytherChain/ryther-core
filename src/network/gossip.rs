//! Gossip protocol for DAG event propagation.

use parking_lot::RwLock;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

use super::message::{EventAnnounce, EventRequest, EventResponse, MessageType, NetworkMessage};
use super::peer::{PeerId, PeerManager};
use crate::dag::DagStore;
use crate::types::event::DagEvent;
use crate::types::EventId;

/// Configuration for gossip protocol.
#[derive(Clone, Debug)]
pub struct GossipConfig {
    /// Number of peers to forward to
    pub fanout: usize,

    /// Max events to request at once
    pub max_request_batch: usize,

    /// Timeout for event requests
    pub request_timeout: Duration,

    /// How long to remember seen message IDs
    pub seen_cache_ttl: Duration,
}

impl Default for GossipConfig {
    fn default() -> Self {
        Self {
            fanout: 8,
            max_request_batch: 100,
            request_timeout: Duration::from_secs(5),
            seen_cache_ttl: Duration::from_secs(300),
        }
    }
}

/// Event pending fetch.
struct PendingEvent {
    /// When we first learned about this event
    announced_at: Instant,

    /// Peers that announced this event
    sources: Vec<PeerId>,

    /// Whether we're currently fetching
    fetching: bool,
}

/// Gossip protocol handler.
pub struct GossipProtocol {
    /// Configuration
    config: GossipConfig,

    /// Peer manager
    peers: Arc<PeerManager>,

    /// DAG store
    dag: Arc<DagStore>,

    /// Our peer ID
    local_id: PeerId,

    /// Seen message IDs (for deduplication)
    seen_messages: RwLock<HashMap<[u8; 16], Instant>>,

    /// Events we know about but haven't fetched
    pending_events: RwLock<HashMap<EventId, PendingEvent>>,

    /// Events we've already processed
    processed_events: RwLock<HashSet<EventId>>,
}

impl GossipProtocol {
    /// Create a new gossip protocol handler.
    pub fn new(
        local_id: PeerId,
        peers: Arc<PeerManager>,
        dag: Arc<DagStore>,
        config: GossipConfig,
    ) -> Self {
        Self {
            config,
            peers,
            dag,
            local_id,
            seen_messages: RwLock::new(HashMap::new()),
            pending_events: RwLock::new(HashMap::new()),
            processed_events: RwLock::new(HashSet::new()),
        }
    }

    /// Check if we've seen this message before.
    pub fn is_seen(&self, message_id: &[u8; 16]) -> bool {
        self.seen_messages.read().contains_key(message_id)
    }

    /// Mark a message as seen.
    pub fn mark_seen(&self, message_id: [u8; 16]) {
        self.seen_messages
            .write()
            .insert(message_id, Instant::now());
    }

    /// Handle incoming gossip message.
    pub fn handle_message(&self, msg: &NetworkMessage) -> Option<NetworkMessage> {
        // Check for duplicates
        if self.is_seen(&msg.message_id) {
            return None;
        }
        self.mark_seen(msg.message_id);

        match &msg.payload {
            MessageType::EventAnnounce(announce) => {
                self.handle_event_announce(&msg.sender, announce)
            }

            MessageType::EventRequest(request) => self.handle_event_request(&msg.sender, request),

            MessageType::EventResponse(response) => {
                self.handle_event_response(&msg.sender, response);
                None
            }

            _ => None,
        }
    }

    /// Handle event announcement.
    fn handle_event_announce(
        &self,
        sender: &PeerId,
        announce: &EventAnnounce,
    ) -> Option<NetworkMessage> {
        let event_id = announce.event_id;

        // Already have this event?
        if self.dag.contains(&event_id) {
            return None;
        }

        // Already processed?
        if self.processed_events.read().contains(&event_id) {
            return None;
        }

        // Add to pending
        {
            let mut pending = self.pending_events.write();
            pending
                .entry(event_id)
                .and_modify(|p| p.sources.push(*sender))
                .or_insert_with(|| PendingEvent {
                    announced_at: Instant::now(),
                    sources: vec![*sender],
                    fetching: false,
                });
        }

        // Request the event
        Some(NetworkMessage::new(
            self.local_id,
            MessageType::EventRequest(EventRequest {
                event_ids: vec![event_id],
            }),
        ))
    }

    /// Handle event request.
    fn handle_event_request(
        &self,
        _sender: &PeerId,
        request: &EventRequest,
    ) -> Option<NetworkMessage> {
        let mut events = Vec::new();
        let mut missing = Vec::new();

        for event_id in &request.event_ids {
            if let Some(event) = self.dag.get(event_id) {
                events.push(event);
            } else {
                missing.push(*event_id);
            }
        }

        Some(NetworkMessage::new(
            self.local_id,
            MessageType::EventResponse(EventResponse { events, missing }),
        ))
    }

    /// Handle event response.
    fn handle_event_response(&self, sender: &PeerId, response: &EventResponse) {
        for event in &response.events {
            // Validate and insert event
            if event.validate_structure().is_ok() {
                self.dag.insert(event.clone());
                self.pending_events.write().remove(&event.id);
                self.processed_events.write().insert(event.id);

                // Reward sender for providing valid event
                self.peers.reward_peer(sender, 1);
            } else {
                // Penalize for invalid event
                self.peers.penalize_peer(sender, 10);
            }
        }
    }

    /// Broadcast a new event to peers.
    pub fn broadcast_event(&self, event: &DagEvent) -> Vec<(PeerId, NetworkMessage)> {
        // Insert into our DAG
        self.dag.insert(event.clone());
        self.processed_events.write().insert(event.id);

        // Create announcement
        let announce = EventAnnounce {
            event_id: event.id,
            creator: event.creator,
            round: event.round,
        };

        // Select peers to forward to
        let peers = self.peers.best_peers(self.config.fanout);

        peers
            .into_iter()
            .map(|peer| {
                let msg = NetworkMessage::new(
                    self.local_id,
                    MessageType::EventAnnounce(announce.clone()),
                );
                (peer.id, msg)
            })
            .collect()
    }

    /// Get events that need to be fetched.
    pub fn pending_fetch_count(&self) -> usize {
        self.pending_events
            .read()
            .values()
            .filter(|p| !p.fetching)
            .count()
    }

    /// Clean up stale data.
    pub fn cleanup(&self) {
        let now = Instant::now();

        // Remove old seen messages
        self.seen_messages
            .write()
            .retain(|_, time| now.duration_since(*time) < self.config.seen_cache_ttl);

        // Remove timed-out pending events
        self.pending_events.write().retain(|_, pending| {
            now.duration_since(pending.announced_at) < self.config.request_timeout * 3
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::event::EncryptedBatch;
    use crate::types::BlsSignature;

    fn make_peer_id(id: u8) -> PeerId {
        let mut arr = [0u8; 32];
        arr[0] = id;
        arr
    }

    fn make_event(creator: u8, round: u64) -> DagEvent {
        let mut creator_id = [0u8; 48];
        creator_id[0] = creator;

        let id = crate::types::sha256(&[creator, round as u8]);

        DagEvent {
            id,
            creator: crate::types::ValidatorId(creator_id),
            lamport_time: round,
            round,
            strong_parents: vec![],
            weak_parents: vec![],
            payload: EncryptedBatch::empty(),
            signature: BlsSignature::default(),
        }
    }

    #[test]
    fn test_gossip_creation() {
        let local_id = make_peer_id(0);
        let peers = Arc::new(PeerManager::new(local_id, 100));
        let dag = Arc::new(DagStore::new());

        let gossip = GossipProtocol::new(local_id, peers, dag, GossipConfig::default());

        assert_eq!(gossip.pending_fetch_count(), 0);
    }

    #[test]
    fn test_broadcast_event() {
        let local_id = make_peer_id(0);
        let peers = Arc::new(PeerManager::new(local_id, 100));
        let dag = Arc::new(DagStore::new());

        // Add some connected peers
        use std::net::{IpAddr, Ipv4Addr, SocketAddr};
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 8000);

        for i in 1..=5 {
            peers.add_peer(make_peer_id(i), addr);
            peers.set_state(&make_peer_id(i), super::super::peer::PeerState::Connected);
        }

        let gossip = GossipProtocol::new(
            local_id,
            peers.clone(),
            dag.clone(),
            GossipConfig::default(),
        );

        let event = make_event(0, 1);
        let messages = gossip.broadcast_event(&event);

        // Should broadcast to connected peers
        assert!(!messages.is_empty());
        assert!(dag.contains(&event.id));
    }

    #[test]
    fn test_deduplication() {
        let local_id = make_peer_id(0);
        let peers = Arc::new(PeerManager::new(local_id, 100));
        let dag = Arc::new(DagStore::new());

        let gossip = GossipProtocol::new(local_id, peers, dag, GossipConfig::default());

        let msg_id = [0xAB; 16];

        assert!(!gossip.is_seen(&msg_id));
        gossip.mark_seen(msg_id);
        assert!(gossip.is_seen(&msg_id));
    }

    #[test]
    fn test_event_request_response() {
        let local_id = make_peer_id(0);
        let peers = Arc::new(PeerManager::new(local_id, 100));
        let dag = Arc::new(DagStore::new());

        // Add an event to DAG
        let event = make_event(1, 1);
        dag.insert(event.clone());

        let gossip = GossipProtocol::new(local_id, peers, dag, GossipConfig::default());

        // Request the event
        let request = EventRequest {
            event_ids: vec![event.id],
        };

        let response = gossip.handle_event_request(&make_peer_id(2), &request);
        assert!(response.is_some());

        match response.unwrap().payload {
            MessageType::EventResponse(resp) => {
                assert_eq!(resp.events.len(), 1);
                assert!(resp.missing.is_empty());
            }
            _ => panic!("Expected EventResponse"),
        }
    }
}
