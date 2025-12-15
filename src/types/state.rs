//! Multi-version state types for parallel execution.
//!
//! Implements Software Transactional Memory (STM) semantics
//! for optimistic concurrency control.

use crate::types::{StateKey, U256};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Status of a transaction's execution.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TxStatus {
    /// Speculatively executed, not yet validated
    Pending,

    /// Validated successfully, no conflicts detected
    Committed,

    /// Conflict detected, will be re-executed
    Aborted,
}

/// A single version of a state value.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StateVersion {
    /// Transaction sequence number that wrote this version
    pub tx_sequence: u64,

    /// The value written (None = deletion)
    pub value: Option<U256>,

    /// Execution status of the writing transaction
    pub status: TxStatus,
}

/// Chain of versions for a single state slot.
///
/// Maintains a history of writes for MVCC reads.
///
/// # Invariants
/// - Versions are sorted by tx_sequence descending (newest first)
/// - No duplicate tx_sequence values
#[derive(Clone, Debug, Default)]
pub struct VersionChain {
    /// Versions sorted by tx_sequence descending
    versions: Vec<StateVersion>,
}

impl VersionChain {
    pub fn new() -> Self {
        Self {
            versions: Vec::new(),
        }
    }

    /// Add a new version to the chain.
    /// Maintains sorted order.
    pub fn add_version(&mut self, version: StateVersion) {
        // Binary search for insertion point (descending order)
        let pos = self
            .versions
            .binary_search_by(|v| version.tx_sequence.cmp(&v.tx_sequence))
            .unwrap_or_else(|pos| pos);

        self.versions.insert(pos, version);
    }

    /// Find the appropriate version for a given transaction.
    ///
    /// Returns the latest COMMITTED or PENDING version with tx_sequence < reader_seq.
    /// Skips ABORTED versions.
    pub fn read_for(&self, reader_seq: u64) -> Option<(Option<U256>, u64)> {
        for version in &self.versions {
            if version.tx_sequence < reader_seq {
                match version.status {
                    TxStatus::Committed | TxStatus::Pending => {
                        return Some((version.value, version.tx_sequence));
                    }
                    TxStatus::Aborted => continue,
                }
            }
        }
        None
    }

    /// Mark a specific version as committed.
    pub fn mark_committed(&mut self, tx_seq: u64) {
        for version in &mut self.versions {
            if version.tx_sequence == tx_seq {
                version.status = TxStatus::Committed;
                return;
            }
        }
    }

    /// Mark a specific version as aborted.
    pub fn mark_aborted(&mut self, tx_seq: u64) {
        for version in &mut self.versions {
            if version.tx_sequence == tx_seq {
                version.status = TxStatus::Aborted;
                return;
            }
        }
    }

    /// Remove aborted versions (cleanup).
    pub fn gc_aborted(&mut self) {
        self.versions.retain(|v| v.status != TxStatus::Aborted);
    }

    /// Get count of versions.
    pub fn len(&self) -> usize {
        self.versions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.versions.is_empty()
    }
}

/// Tracks all state reads during a transaction's execution.
#[derive(Clone, Debug, Default)]
pub struct ReadSet {
    /// Map: StateKey -> (value read, version number that was read)
    pub reads: HashMap<StateKey, (Option<U256>, u64)>,
}

impl ReadSet {
    pub fn new() -> Self {
        Self {
            reads: HashMap::new(),
        }
    }

    /// Record a read.
    pub fn record(&mut self, key: StateKey, value: Option<U256>, version: u64) {
        self.reads.insert(key, (value, version));
    }

    /// Get all keys that were read.
    pub fn keys(&self) -> impl Iterator<Item = &StateKey> {
        self.reads.keys()
    }

    /// Number of unique keys read.
    pub fn len(&self) -> usize {
        self.reads.len()
    }

    pub fn is_empty(&self) -> bool {
        self.reads.is_empty()
    }
}

/// Tracks all state writes during a transaction's execution.
#[derive(Clone, Debug, Default)]
pub struct WriteSet {
    /// Map: StateKey -> new value (None = deletion)
    pub writes: HashMap<StateKey, Option<U256>>,
}

impl WriteSet {
    pub fn new() -> Self {
        Self {
            writes: HashMap::new(),
        }
    }

    /// Record a write.
    pub fn record(&mut self, key: StateKey, value: Option<U256>) {
        self.writes.insert(key, value);
    }

    /// Get all keys that were written.
    pub fn keys(&self) -> impl Iterator<Item = &StateKey> {
        self.writes.keys()
    }

    /// Number of unique keys written.
    pub fn len(&self) -> usize {
        self.writes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.writes.is_empty()
    }

    /// Merge another write set into this one.
    pub fn merge(&mut self, other: WriteSet) {
        for (key, value) in other.writes {
            self.writes.insert(key, value);
        }
    }
}

/// Result of executing a single transaction.
#[derive(Clone, Debug)]
pub struct ExecutionResult {
    /// Transaction sequence number
    pub tx_seq: u64,

    /// Execution outcome
    pub status: ExecutionStatus,

    /// All state reads performed
    pub read_set: ReadSet,

    /// All state writes performed
    pub write_set: WriteSet,

    /// Gas consumed
    pub gas_used: u64,

    /// Event logs emitted
    pub logs: Vec<Log>,

    /// Return data (for calls) or deployed code address (for creates)
    pub output: Vec<u8>,
}

/// Outcome of transaction execution.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecutionStatus {
    /// Executed successfully
    Success,

    /// Reverted (REVERT opcode)
    Revert,

    /// Failed (out of gas, invalid opcode, etc.)
    Failure,
}

/// An event log emitted during execution.
#[derive(Clone, Debug)]
pub struct Log {
    /// Contract that emitted the log
    pub address: [u8; 20],

    /// Log topics (up to 4)
    pub topics: Vec<[u8; 32]>,

    /// Log data
    pub data: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_key(addr: u8, slot: u64) -> StateKey {
        StateKey {
            address: [addr; 20],
            slot: U256::from_u64(slot),
        }
    }

    #[test]
    fn test_version_chain_ordering() {
        let mut chain = VersionChain::new();

        // Add out of order
        chain.add_version(StateVersion {
            tx_sequence: 5,
            value: Some(U256::from_u64(500)),
            status: TxStatus::Committed,
        });
        chain.add_version(StateVersion {
            tx_sequence: 10,
            value: Some(U256::from_u64(1000)),
            status: TxStatus::Pending,
        });
        chain.add_version(StateVersion {
            tx_sequence: 3,
            value: Some(U256::from_u64(300)),
            status: TxStatus::Committed,
        });

        // Should be sorted descending
        assert_eq!(chain.versions[0].tx_sequence, 10);
        assert_eq!(chain.versions[1].tx_sequence, 5);
        assert_eq!(chain.versions[2].tx_sequence, 3);
    }

    #[test]
    fn test_read_for_transaction() {
        let mut chain = VersionChain::new();

        chain.add_version(StateVersion {
            tx_sequence: 5,
            value: Some(U256::from_u64(500)),
            status: TxStatus::Committed,
        });
        chain.add_version(StateVersion {
            tx_sequence: 10,
            value: Some(U256::from_u64(1000)),
            status: TxStatus::Committed,
        });

        // Transaction 7 should see version 5
        let (value, version) = chain.read_for(7).unwrap();
        assert_eq!(value, Some(U256::from_u64(500)));
        assert_eq!(version, 5);

        // Transaction 15 should see version 10
        let (value, version) = chain.read_for(15).unwrap();
        assert_eq!(value, Some(U256::from_u64(1000)));
        assert_eq!(version, 10);

        // Transaction 3 should see nothing
        assert!(chain.read_for(3).is_none());
    }

    #[test]
    fn test_skip_aborted_versions() {
        let mut chain = VersionChain::new();

        chain.add_version(StateVersion {
            tx_sequence: 5,
            value: Some(U256::from_u64(500)),
            status: TxStatus::Committed,
        });
        chain.add_version(StateVersion {
            tx_sequence: 8,
            value: Some(U256::from_u64(800)),
            status: TxStatus::Aborted, // Should be skipped
        });

        // Transaction 10 should see version 5 (skipping aborted 8)
        let (value, version) = chain.read_for(10).unwrap();
        assert_eq!(value, Some(U256::from_u64(500)));
        assert_eq!(version, 5);
    }

    #[test]
    fn test_mark_status() {
        let mut chain = VersionChain::new();

        chain.add_version(StateVersion {
            tx_sequence: 5,
            value: Some(U256::from_u64(500)),
            status: TxStatus::Pending,
        });

        chain.mark_committed(5);
        assert_eq!(chain.versions[0].status, TxStatus::Committed);

        chain.mark_aborted(5);
        assert_eq!(chain.versions[0].status, TxStatus::Aborted);
    }
}
