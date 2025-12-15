//! TCP transport layer for network communication.

use parking_lot::RwLock;
use std::collections::HashMap;
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;

use super::message::NetworkMessage;
use super::peer::{PeerId, PeerManager, PeerState};

/// Connection state for a peer.
struct Connection {
    /// Peer ID
    peer_id: PeerId,

    /// Sender to write to this connection
    sender: mpsc::Sender<Vec<u8>>,
}

/// TCP transport configuration.
#[derive(Clone, Debug)]
pub struct TransportConfig {
    /// Address to listen on
    pub listen_addr: SocketAddr,

    /// Maximum message size (bytes)
    pub max_message_size: usize,

    /// Connection timeout
    pub connect_timeout_ms: u64,
}

impl Default for TransportConfig {
    fn default() -> Self {
        Self {
            listen_addr: "0.0.0.0:30303".parse().unwrap(),
            max_message_size: 16 * 1024 * 1024, // 16 MB
            connect_timeout_ms: 5000,
        }
    }
}

/// TCP transport layer.
pub struct TcpTransport {
    /// Configuration
    config: TransportConfig,

    /// Our peer ID
    local_id: PeerId,

    /// Peer manager
    peers: Arc<PeerManager>,

    /// Active connections by peer ID
    connections: RwLock<HashMap<PeerId, Connection>>,

    /// Channel for incoming messages
    incoming_tx: mpsc::Sender<(PeerId, NetworkMessage)>,
    incoming_rx: RwLock<Option<mpsc::Receiver<(PeerId, NetworkMessage)>>>,
}

impl TcpTransport {
    /// Create a new TCP transport.
    pub fn new(local_id: PeerId, peers: Arc<PeerManager>, config: TransportConfig) -> Self {
        let (tx, rx) = mpsc::channel(10000);

        Self {
            config,
            local_id,
            peers,
            connections: RwLock::new(HashMap::new()),
            incoming_tx: tx,
            incoming_rx: RwLock::new(Some(rx)),
        }
    }

    /// Take the incoming message receiver.
    pub fn take_receiver(&self) -> Option<mpsc::Receiver<(PeerId, NetworkMessage)>> {
        self.incoming_rx.write().take()
    }

    /// Check if connected to a peer.
    pub fn is_connected(&self, peer_id: &PeerId) -> bool {
        self.connections.read().contains_key(peer_id)
    }

    /// Get number of active connections.
    pub fn connection_count(&self) -> usize {
        self.connections.read().len()
    }

    /// Send a message to a peer.
    pub async fn send(&self, peer_id: &PeerId, message: NetworkMessage) -> io::Result<()> {
        let sender = {
            let connections = self.connections.read();
            connections.get(peer_id).map(|c| c.sender.clone())
        };

        let sender = sender
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "Peer not connected"))?;

        let bytes = message.to_bytes();
        sender
            .send(bytes)
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "Connection closed"))?;

        self.peers.record_sent(peer_id);
        Ok(())
    }

    /// Broadcast a message to all connected peers.
    pub async fn broadcast(&self, message: NetworkMessage) -> usize {
        let peer_ids: Vec<_> = self.connections.read().keys().copied().collect();
        let mut sent = 0;

        for peer_id in peer_ids {
            if self.send(&peer_id, message.clone()).await.is_ok() {
                sent += 1;
            }
        }

        sent
    }

    /// Connect to a peer.
    pub async fn connect(&self, peer_id: PeerId, addr: SocketAddr) -> io::Result<()> {
        // Already connected?
        if self.is_connected(&peer_id) {
            return Ok(());
        }

        // Update peer state
        self.peers.add_peer(peer_id, addr);
        self.peers.set_state(&peer_id, PeerState::Connecting);

        // Connect with timeout
        let stream = tokio::time::timeout(
            std::time::Duration::from_millis(self.config.connect_timeout_ms),
            TcpStream::connect(addr),
        )
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "Connection timeout"))??;

        self.setup_connection(peer_id, stream).await
    }

    /// Set up a new connection.
    async fn setup_connection(&self, peer_id: PeerId, stream: TcpStream) -> io::Result<()> {
        let (reader, writer) = stream.into_split();

        // Create channel for outgoing messages
        let (tx, mut rx) = mpsc::channel::<Vec<u8>>(1000);

        // Store connection
        {
            let mut connections = self.connections.write();
            connections.insert(
                peer_id,
                Connection {
                    peer_id,
                    sender: tx,
                },
            );
        }

        self.peers.set_state(&peer_id, PeerState::Connected);

        // Spawn writer task
        let writer_handle = tokio::spawn(async move {
            let mut writer = writer;
            while let Some(data) = rx.recv().await {
                // Write length prefix (4 bytes)
                let len = (data.len() as u32).to_be_bytes();
                if writer.write_all(&len).await.is_err() {
                    break;
                }
                if writer.write_all(&data).await.is_err() {
                    break;
                }
            }
        });

        // Spawn reader task
        let incoming_tx = self.incoming_tx.clone();
        let max_size = self.config.max_message_size;
        let peers = Arc::clone(&self.peers);

        tokio::spawn(async move {
            let mut reader = reader;
            let mut len_buf = [0u8; 4];

            loop {
                // Read length prefix
                if reader.read_exact(&mut len_buf).await.is_err() {
                    break;
                }

                let len = u32::from_be_bytes(len_buf) as usize;

                // Validate message size
                if len > max_size {
                    break;
                }

                // Read message
                let mut data = vec![0u8; len];
                if reader.read_exact(&mut data).await.is_err() {
                    break;
                }

                // Parse message
                if let Some(msg) = NetworkMessage::from_bytes(&data) {
                    peers.record_received(&peer_id);
                    let _ = incoming_tx.send((peer_id, msg)).await;
                }
            }

            // Clean up
            writer_handle.abort();
        });

        Ok(())
    }

    /// Disconnect from a peer.
    pub fn disconnect(&self, peer_id: &PeerId) {
        self.connections.write().remove(peer_id);
        self.peers.set_state(peer_id, PeerState::Disconnected);
    }

    /// Start listening for incoming connections.
    pub async fn listen(&self) -> io::Result<()> {
        let listener = TcpListener::bind(self.config.listen_addr).await?;

        loop {
            let (stream, addr) = listener.accept().await?;

            // Generate peer ID from address (temporary until handshake)
            let peer_id = self.addr_to_peer_id(&addr);

            self.peers.add_peer(peer_id, addr);

            if let Err(e) = self.setup_connection(peer_id, stream).await {
                tracing::warn!("Failed to setup connection from {}: {}", addr, e);
            }
        }
    }

    /// Convert address to temporary peer ID.
    fn addr_to_peer_id(&self, addr: &SocketAddr) -> PeerId {
        use crate::types::sha256;
        let addr_str = addr.to_string();
        sha256(addr_str.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    fn make_peer_id(id: u8) -> PeerId {
        let mut arr = [0u8; 32];
        arr[0] = id;
        arr
    }

    fn make_addr(port: u16) -> SocketAddr {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), port)
    }

    #[test]
    fn test_transport_creation() {
        let local_id = make_peer_id(0);
        let peers = Arc::new(PeerManager::new(local_id, 100));
        let config = TransportConfig {
            listen_addr: make_addr(0),
            ..Default::default()
        };

        let transport = TcpTransport::new(local_id, peers, config);

        assert_eq!(transport.connection_count(), 0);
        assert!(transport.take_receiver().is_some());
    }

    #[tokio::test]
    async fn test_connect_nonexistent() {
        let local_id = make_peer_id(0);
        let peers = Arc::new(PeerManager::new(local_id, 100));
        let config = TransportConfig {
            listen_addr: make_addr(0),
            connect_timeout_ms: 100,
            ..Default::default()
        };

        let transport = TcpTransport::new(local_id, peers, config);

        // Try to connect to non-existent peer
        let result = transport.connect(make_peer_id(1), make_addr(59999)).await;

        // Should fail (connection refused or timeout)
        assert!(result.is_err());
    }
}
