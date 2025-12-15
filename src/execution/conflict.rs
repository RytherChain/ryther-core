//! Conflict detection for parallel execution.
//!
//! Validates that speculative execution results are consistent
//! with sequential execution semantics.

use crate::types::state::{ExecutionResult, ReadSet, WriteSet};
use crate::types::StateKey;
use std::collections::HashMap;

/// Conflict detector for validating parallel execution.
///
/// # Conflict Definition
/// A transaction T conflicts if it read a value V from version X,
/// but a transaction with sequence S where X < S < T.seq wrote
/// to the same key. This means T read stale data.
#[derive(Debug, Default)]
pub struct ConflictDetector {
    /// Tracks committed writes: key -> sequence number
    committed_writes: HashMap<StateKey, u64>,
}

impl ConflictDetector {
    pub fn new() -> Self {
        Self {
            committed_writes: HashMap::new(),
        }
    }

    /// Check if a transaction's reads are still valid.
    ///
    /// Returns true if there's a conflict (stale read detected).
    pub fn has_conflict(&self, tx_seq: u64, read_set: &ReadSet) -> bool {
        for (key, (_, read_version)) in &read_set.reads {
            if let Some(&write_seq) = self.committed_writes.get(key) {
                // Conflict if:
                // 1. A write happened after the version we read
                // 2. That write was from a transaction before us
                if write_seq > *read_version && write_seq < tx_seq {
                    return true;
                }
            }
        }
        false
    }

    /// Record that a transaction's writes are now committed.
    pub fn record_commit(&mut self, tx_seq: u64, write_set: &WriteSet) {
        for key in write_set.keys() {
            self.committed_writes.insert(key.clone(), tx_seq);
        }
    }

    /// Get all keys with committed writes.
    pub fn committed_keys(&self) -> impl Iterator<Item = &StateKey> {
        self.committed_writes.keys()
    }

    /// Clear all tracked state (for new block).
    pub fn reset(&mut self) {
        self.committed_writes.clear();
    }
}

/// Result of conflict detection for a batch.
#[derive(Debug)]
pub struct ValidationResult {
    /// Transactions that passed validation
    pub committed: Vec<u64>,

    /// Transactions that need re-execution
    pub aborted: Vec<u64>,
}

/// Validate a batch of execution results in sequence order.
///
/// Returns which transactions should be committed vs aborted.
pub fn validate_batch(results: &[ExecutionResult]) -> ValidationResult {
    let mut detector = ConflictDetector::new();
    let mut committed = Vec::new();
    let mut aborted = Vec::new();

    // Must validate in sequence order
    let mut sorted_results: Vec<_> = results.iter().collect();
    sorted_results.sort_by_key(|r| r.tx_seq);

    for result in sorted_results {
        if detector.has_conflict(result.tx_seq, &result.read_set) {
            aborted.push(result.tx_seq);
        } else {
            detector.record_commit(result.tx_seq, &result.write_set);
            committed.push(result.tx_seq);
        }
    }

    ValidationResult { committed, aborted }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::state::ExecutionStatus;
    use crate::types::U256;

    fn make_key(id: u8) -> StateKey {
        StateKey {
            address: [id; 20],
            slot: U256::ZERO,
        }
    }

    fn make_result(seq: u64, reads: Vec<(u8, u64)>, writes: Vec<u8>) -> ExecutionResult {
        let mut read_set = ReadSet::new();
        for (key_id, version) in reads {
            read_set.record(make_key(key_id), None, version);
        }

        let mut write_set = WriteSet::new();
        for key_id in writes {
            write_set.record(make_key(key_id), Some(U256::from_u64(seq)));
        }

        ExecutionResult {
            tx_seq: seq,
            status: ExecutionStatus::Success,
            read_set,
            write_set,
            gas_used: 21000,
            logs: vec![],
            output: vec![],
        }
    }

    #[test]
    fn test_no_conflict_independent_txs() {
        // Two transactions touching different keys
        let results = vec![
            make_result(0, vec![(1, 0)], vec![1]), // Reads/writes key 1
            make_result(1, vec![(2, 0)], vec![2]), // Reads/writes key 2
        ];

        let validation = validate_batch(&results);

        assert_eq!(validation.committed, vec![0, 1]);
        assert!(validation.aborted.is_empty());
    }

    #[test]
    fn test_conflict_detected() {
        // Tx 1 writes to key 1 at seq 1
        // Tx 2 reads key 1 from version 0 (stale because tx 1 wrote at seq 1)
        // For conflict: tx 2 must have read from version < tx 1's write seq
        let results = vec![
            make_result(1, vec![], vec![1]),       // Writes key 1 at seq 1
            make_result(2, vec![(1, 0)], vec![2]), // Reads key 1 from version 0 (stale!)
        ];

        let validation = validate_batch(&results);

        // Tx 1 commits (no reads to conflict)
        // Tx 2: read version 0, tx 1 wrote at seq 1 -> 1 > 0 AND 1 < 2 -> CONFLICT
        assert_eq!(validation.committed, vec![1]);
        assert_eq!(validation.aborted, vec![2]);
    }

    #[test]
    fn test_read_same_version_no_conflict() {
        // Tx 0 writes to key 1
        // Tx 1 reads key 1 but saw tx 0's write (read version = 0 means base, not tx 0's write)
        // If tx 1 read from tx 0's write (version 0), there's no stale read
        let results = vec![
            make_result(0, vec![], vec![1]),
            make_result(1, vec![(1, 0)], vec![]), // Read from base (version 0)
        ];

        let validation = validate_batch(&results);
        // write_seq=0, read_version=0, check: 0 > 0 = false -> no conflict
        assert_eq!(validation.committed, vec![0, 1]);
        assert!(validation.aborted.is_empty());
    }

    #[test]
    fn test_cascading_scenario() {
        // Tx 1 writes key 1
        // Tx 2 reads key 1 from version 0 -> conflicts
        // Tx 3 reads key 2 (independent) -> no conflict
        let results = vec![
            make_result(1, vec![], vec![1]),       // Writes key 1
            make_result(2, vec![(1, 0)], vec![2]), // Stale read of key 1
            make_result(3, vec![(3, 0)], vec![3]), // Independent key 3
        ];

        let validation = validate_batch(&results);

        // Tx 2 conflicts (read version 0, tx 1 wrote at 1, 1 > 0 && 1 < 2)
        // Tx 3 is fine (different key)
        assert_eq!(validation.committed, vec![1, 3]);
        assert_eq!(validation.aborted, vec![2]);
    }
}
