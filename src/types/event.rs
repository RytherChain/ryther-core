//! DAG Event types and construction.
//!
//! Events are the fundamental unit of the Helix consensus DAG.
//! Each event represents a validator's contribution at a point in time.

pub use super::transaction::EncryptedBatch;
use crate::types::{sha256, BlsSignature, EventId, ValidatorId};
use serde::{Deserialize, Serialize};

/// A DAG event created by a validator.
///
/// # Invariants
/// - `lamport_time > max(parent.lamport_time)` for all parents
/// - `strong_parents` is non-empty (except genesis)
/// - At least one strong parent is from the same creator (self-parent)
/// - `id` is computed deterministically from contents
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DagEvent {
    // === Identity ===
    /// Unique identifier: sha256(creator || round || payload_hash || parents)
    pub id: EventId,

    /// Public key of the creating validator
    pub creator: ValidatorId,

    // === Causal Structure ===
    /// Lamport timestamp encoding causal order
    /// INVARIANT: lamport_time > 0 for all non-genesis events
    pub lamport_time: u64,

    /// DAG round (depth-based)
    /// INVARIANT: round >= 1 for non-genesis
    pub round: u64,

    /// Strong parent references (causal dependencies)
    /// INVARIANT: |strong_parents| >= 1 (except genesis)
    pub strong_parents: Vec<EventId>,

    /// Weak parent references (availability, no causal dependency)
    pub weak_parents: Vec<EventId>,

    // === Payload ===
    /// Encrypted transaction batch
    pub payload: EncryptedBatch,

    // === Signature ===
    /// BLS signature over event contents
    pub signature: BlsSignature,
}

impl DagEvent {
    /// Compute the canonical event ID from contents.
    pub fn compute_id(
        creator: &ValidatorId,
        round: u64,
        payload_hash: &[u8; 32],
        strong_parents: &[EventId],
    ) -> EventId {
        let mut data = Vec::new();

        // Creator
        data.extend_from_slice(creator.as_bytes());

        // Round
        data.extend_from_slice(&round.to_le_bytes());

        // Payload hash
        data.extend_from_slice(payload_hash);

        // Parents (sorted for determinism)
        let mut sorted_parents = strong_parents.to_vec();
        sorted_parents.sort();
        for parent in sorted_parents {
            data.extend_from_slice(&parent);
        }

        sha256(&data)
    }

    /// Message bytes for BLS signing.
    pub fn signing_message(&self) -> Vec<u8> {
        let mut msg = Vec::new();
        msg.extend_from_slice(&self.id);
        msg.extend_from_slice(&self.lamport_time.to_le_bytes());
        msg.extend_from_slice(&self.round.to_le_bytes());
        for parent in &self.strong_parents {
            msg.extend_from_slice(parent);
        }
        msg.extend_from_slice(&self.payload.batch_root);
        msg
    }

    /// Check if this event is a genesis event (no parents).
    pub fn is_genesis(&self) -> bool {
        self.strong_parents.is_empty() && self.round == 0
    }

    /// Validate structural invariants (not signature).
    pub fn validate_structure(&self) -> Result<(), EventValidationError> {
        // Genesis events have special rules
        if self.is_genesis() {
            if self.lamport_time != 0 {
                return Err(EventValidationError::InvalidGenesisTimestamp);
            }
            return Ok(());
        }

        // Non-genesis must have parents
        if self.strong_parents.is_empty() {
            return Err(EventValidationError::NoParents);
        }

        // Lamport time must be positive
        if self.lamport_time == 0 {
            return Err(EventValidationError::ZeroLamportTime);
        }

        // Round must be positive
        if self.round == 0 {
            return Err(EventValidationError::ZeroRound);
        }

        Ok(())
    }
}

/// Errors during event validation.
#[derive(Debug, Clone, thiserror::Error)]
pub enum EventValidationError {
    #[error("Genesis event must have lamport_time = 0")]
    InvalidGenesisTimestamp,

    #[error("Non-genesis event must have at least one parent")]
    NoParents,

    #[error("Non-genesis event must have lamport_time > 0")]
    ZeroLamportTime,

    #[error("Non-genesis event must have round > 0")]
    ZeroRound,

    #[error("Lamport time {event_time} not greater than parent time {parent_time}")]
    LamportViolation { event_time: u64, parent_time: u64 },

    #[error("Missing self-parent")]
    MissingSelfParent,

    #[error("Missing parent: {0}")]
    MissingParent(String),

    #[error("Invalid signature")]
    InvalidSignature,
}

/// Builder for constructing valid DAG events.
pub struct EventBuilder {
    creator: ValidatorId,
    strong_parents: Vec<EventId>,
    weak_parents: Vec<EventId>,
    payload: EncryptedBatch,
    parent_times: Vec<u64>,
}

impl EventBuilder {
    pub fn new(creator: ValidatorId) -> Self {
        Self {
            creator,
            strong_parents: Vec::new(),
            weak_parents: Vec::new(),
            payload: EncryptedBatch::empty(),
            parent_times: Vec::new(),
        }
    }

    /// Add a strong parent with its Lamport time.
    pub fn add_strong_parent(mut self, id: EventId, lamport_time: u64) -> Self {
        self.strong_parents.push(id);
        self.parent_times.push(lamport_time);
        self
    }

    /// Add a weak parent (for availability, not causal).
    pub fn add_weak_parent(mut self, id: EventId) -> Self {
        self.weak_parents.push(id);
        self
    }

    /// Set the transaction payload.
    pub fn with_payload(mut self, payload: EncryptedBatch) -> Self {
        self.payload = payload;
        self
    }

    /// Compute round from parent rounds.
    /// Round = max(parent rounds) + 1, or 1 if first non-genesis.
    fn compute_round(&self, parent_rounds: &[u64]) -> u64 {
        parent_rounds.iter().max().copied().unwrap_or(0) + 1
    }

    /// Build the event (without signature - caller must sign).
    pub fn build(self, parent_rounds: &[u64]) -> (DagEvent, Vec<u8>) {
        let lamport_time = self.parent_times.iter().max().copied().unwrap_or(0) + 1;
        let round = self.compute_round(parent_rounds);

        let id = DagEvent::compute_id(
            &self.creator,
            round,
            &self.payload.batch_root,
            &self.strong_parents,
        );

        let event = DagEvent {
            id,
            creator: self.creator,
            lamport_time,
            round,
            strong_parents: self.strong_parents,
            weak_parents: self.weak_parents,
            payload: self.payload,
            signature: BlsSignature::default(),
        };

        let signing_msg = event.signing_message();
        (event, signing_msg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_validator() -> ValidatorId {
        ValidatorId([0x42; 48])
    }

    #[test]
    fn test_genesis_validation() {
        let genesis = DagEvent {
            id: [0; 32],
            creator: dummy_validator(),
            lamport_time: 0,
            round: 0,
            strong_parents: vec![],
            weak_parents: vec![],
            payload: EncryptedBatch::empty(),
            signature: BlsSignature::default(),
        };

        assert!(genesis.is_genesis());
        assert!(genesis.validate_structure().is_ok());
    }

    #[test]
    fn test_non_genesis_needs_parents() {
        let event = DagEvent {
            id: [0; 32],
            creator: dummy_validator(),
            lamport_time: 1,
            round: 1,
            strong_parents: vec![], // Invalid!
            weak_parents: vec![],
            payload: EncryptedBatch::empty(),
            signature: BlsSignature::default(),
        };

        assert!(!event.is_genesis());
        assert!(matches!(
            event.validate_structure(),
            Err(EventValidationError::NoParents)
        ));
    }

    #[test]
    fn test_event_id_determinism() {
        let creator = dummy_validator();
        let payload_hash = [0xAB; 32];
        let parents = vec![[1u8; 32], [2u8; 32]];

        let id1 = DagEvent::compute_id(&creator, 5, &payload_hash, &parents);
        let id2 = DagEvent::compute_id(&creator, 5, &payload_hash, &parents);

        assert_eq!(id1, id2, "ID computation must be deterministic");
    }

    #[test]
    fn test_parent_order_independence() {
        let creator = dummy_validator();
        let payload_hash = [0xAB; 32];

        // Different order, same parents
        let parents1 = vec![[1u8; 32], [2u8; 32]];
        let parents2 = vec![[2u8; 32], [1u8; 32]];

        let id1 = DagEvent::compute_id(&creator, 5, &payload_hash, &parents1);
        let id2 = DagEvent::compute_id(&creator, 5, &payload_hash, &parents2);

        assert_eq!(id1, id2, "Parent order should not affect ID");
    }
}
