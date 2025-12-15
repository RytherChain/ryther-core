//! Peer identity and management.

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Unique peer identifier (32 bytes).
pub type PeerId = [u8; 32];

/// Connection state of a peer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PeerState {
    /// Not connected
    Disconnected,
    /// Connection in progress
    Connecting,
    /// Fully connected
    Connected,
    /// Temporarily banned
    Banned,
}

/// Information about a peer.
#[derive(Clone, Debug)]
pub struct PeerInfo {
    /// Unique peer ID
    pub id: PeerId,

    /// Network address
    pub addr: SocketAddr,

    /// Connection state
    pub state: PeerState,

    /// Last seen timestamp
    pub last_seen: Instant,

    /// Round-trip time estimate
    pub rtt: Option<Duration>,

    /// Number of successful messages
    pub messages_sent: u64,

    /// Number of messages received
    pub messages_received: u64,

    /// Reputation score (higher is better)
    pub reputation: i32,
}

impl PeerInfo {
    /// Create new peer info.
    pub fn new(id: PeerId, addr: SocketAddr) -> Self {
        Self {
            id,
            addr,
            state: PeerState::Disconnected,
            last_seen: Instant::now(),
            rtt: None,
            messages_sent: 0,
            messages_received: 0,
            reputation: 0,
        }
    }

    /// Update last seen time.
    pub fn touch(&mut self) {
        self.last_seen = Instant::now();
    }

    /// Check if peer is stale (not seen recently).
    pub fn is_stale(&self, timeout: Duration) -> bool {
        self.last_seen.elapsed() > timeout
    }

    /// Increase reputation.
    pub fn reward(&mut self, amount: i32) {
        self.reputation = self.reputation.saturating_add(amount);
    }

    /// Decrease reputation.
    pub fn penalize(&mut self, amount: i32) {
        self.reputation = self.reputation.saturating_sub(amount);
    }
}

/// Manages known peers and their connections.
pub struct PeerManager {
    /// Known peers by ID
    peers: RwLock<HashMap<PeerId, PeerInfo>>,

    /// Maximum number of peers
    max_peers: usize,

    /// Stale timeout
    stale_timeout: Duration,

    /// Our own peer ID
    local_id: PeerId,
}

impl PeerManager {
    /// Create a new peer manager.
    pub fn new(local_id: PeerId, max_peers: usize) -> Self {
        Self {
            peers: RwLock::new(HashMap::new()),
            max_peers,
            stale_timeout: Duration::from_secs(300), // 5 minutes
            local_id,
        }
    }

    /// Get our local peer ID.
    pub fn local_id(&self) -> PeerId {
        self.local_id
    }

    /// Add or update a peer.
    pub fn add_peer(&self, id: PeerId, addr: SocketAddr) -> bool {
        if id == self.local_id {
            return false; // Don't add ourselves
        }

        let mut peers = self.peers.write();

        if peers.len() >= self.max_peers && !peers.contains_key(&id) {
            return false; // At capacity
        }

        peers
            .entry(id)
            .and_modify(|p| {
                p.addr = addr;
                p.touch();
            })
            .or_insert_with(|| PeerInfo::new(id, addr));

        true
    }

    /// Remove a peer.
    pub fn remove_peer(&self, id: &PeerId) -> Option<PeerInfo> {
        self.peers.write().remove(id)
    }

    /// Get peer info.
    pub fn get_peer(&self, id: &PeerId) -> Option<PeerInfo> {
        self.peers.read().get(id).cloned()
    }

    /// Update peer state.
    pub fn set_state(&self, id: &PeerId, state: PeerState) {
        if let Some(peer) = self.peers.write().get_mut(id) {
            peer.state = state;
            peer.touch();
        }
    }

    /// Get all connected peers.
    pub fn connected_peers(&self) -> Vec<PeerInfo> {
        self.peers
            .read()
            .values()
            .filter(|p| p.state == PeerState::Connected)
            .cloned()
            .collect()
    }

    /// Get all peer IDs.
    pub fn all_peer_ids(&self) -> Vec<PeerId> {
        self.peers.read().keys().copied().collect()
    }

    /// Get number of peers.
    pub fn peer_count(&self) -> usize {
        self.peers.read().len()
    }

    /// Get number of connected peers.
    pub fn connected_count(&self) -> usize {
        self.peers
            .read()
            .values()
            .filter(|p| p.state == PeerState::Connected)
            .count()
    }

    /// Record message sent to peer.
    pub fn record_sent(&self, id: &PeerId) {
        if let Some(peer) = self.peers.write().get_mut(id) {
            peer.messages_sent += 1;
            peer.touch();
        }
    }

    /// Record message received from peer.
    pub fn record_received(&self, id: &PeerId) {
        if let Some(peer) = self.peers.write().get_mut(id) {
            peer.messages_received += 1;
            peer.touch();
        }
    }

    /// Reward a peer for good behavior.
    pub fn reward_peer(&self, id: &PeerId, amount: i32) {
        if let Some(peer) = self.peers.write().get_mut(id) {
            peer.reward(amount);
        }
    }

    /// Penalize a peer for bad behavior.
    pub fn penalize_peer(&self, id: &PeerId, amount: i32) {
        if let Some(peer) = self.peers.write().get_mut(id) {
            peer.penalize(amount);
        }
    }

    /// Ban a peer temporarily.
    pub fn ban_peer(&self, id: &PeerId) {
        self.set_state(id, PeerState::Banned);
    }

    /// Remove stale peers.
    pub fn prune_stale(&self) -> Vec<PeerId> {
        let mut peers = self.peers.write();
        let stale: Vec<_> = peers
            .iter()
            .filter(|(_, p)| p.is_stale(self.stale_timeout))
            .map(|(id, _)| *id)
            .collect();

        for id in &stale {
            peers.remove(id);
        }

        stale
    }

    /// Get best peers by reputation.
    pub fn best_peers(&self, count: usize) -> Vec<PeerInfo> {
        let peers = self.peers.read();
        let mut sorted: Vec<_> = peers
            .values()
            .filter(|p| p.state == PeerState::Connected)
            .cloned()
            .collect();

        sorted.sort_by(|a, b| b.reputation.cmp(&a.reputation));
        sorted.truncate(count);
        sorted
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    fn make_addr(port: u16) -> SocketAddr {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), port)
    }

    fn make_peer_id(id: u8) -> PeerId {
        let mut arr = [0u8; 32];
        arr[0] = id;
        arr
    }

    #[test]
    fn test_add_peer() {
        let local_id = make_peer_id(0);
        let manager = PeerManager::new(local_id, 100);

        let peer_id = make_peer_id(1);
        assert!(manager.add_peer(peer_id, make_addr(8000)));
        assert_eq!(manager.peer_count(), 1);
    }

    #[test]
    fn test_no_self_connect() {
        let local_id = make_peer_id(0);
        let manager = PeerManager::new(local_id, 100);

        // Should not add ourselves
        assert!(!manager.add_peer(local_id, make_addr(8000)));
        assert_eq!(manager.peer_count(), 0);
    }

    #[test]
    fn test_connected_peers() {
        let local_id = make_peer_id(0);
        let manager = PeerManager::new(local_id, 100);

        let peer1 = make_peer_id(1);
        let peer2 = make_peer_id(2);

        manager.add_peer(peer1, make_addr(8001));
        manager.add_peer(peer2, make_addr(8002));

        manager.set_state(&peer1, PeerState::Connected);

        assert_eq!(manager.connected_count(), 1);
    }

    #[test]
    fn test_reputation() {
        let local_id = make_peer_id(0);
        let manager = PeerManager::new(local_id, 100);

        let peer_id = make_peer_id(1);
        manager.add_peer(peer_id, make_addr(8000));

        manager.reward_peer(&peer_id, 10);
        assert_eq!(manager.get_peer(&peer_id).unwrap().reputation, 10);

        manager.penalize_peer(&peer_id, 5);
        assert_eq!(manager.get_peer(&peer_id).unwrap().reputation, 5);
    }

    #[test]
    fn test_max_peers() {
        let local_id = make_peer_id(0);
        let manager = PeerManager::new(local_id, 2);

        assert!(manager.add_peer(make_peer_id(1), make_addr(8001)));
        assert!(manager.add_peer(make_peer_id(2), make_addr(8002)));
        assert!(!manager.add_peer(make_peer_id(3), make_addr(8003))); // At capacity

        assert_eq!(manager.peer_count(), 2);
    }
}
