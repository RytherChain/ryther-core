//! Multi-Version Concurrency Control (MVCC) State Store.
//!
//! This is the core of RytherVM's parallel execution engine.
//! Provides lock-free concurrent access to state while tracking
//! read/write sets for conflict detection.

use crate::types::state::{ReadSet, StateVersion, TxStatus, VersionChain, WriteSet};
use crate::types::{StateKey, U256};
use dashmap::DashMap;
use std::sync::Arc;

/// Multi-version state store for parallel transaction execution.
///
/// Thread-safe and lock-free for concurrent reads and writes.
///
/// # Design
/// - Each state key maps to a version chain
/// - Versions are indexed by transaction sequence number
/// - Readers see the latest committed/pending version before their sequence
/// - Writers create new pending versions
#[derive(Clone)]
pub struct MultiVersionState {
    /// Version chains indexed by state key
    data: Arc<DashMap<StateKey, VersionChain>>,

    /// Base state (committed state from previous block)
    base_state: Arc<DashMap<StateKey, U256>>,

    /// Jellyfish Merkle Tree for authenticated persistence
    jmt: Arc<parking_lot::RwLock<crate::storage::jmt::JellyfishMerkleTree>>,
}

impl MultiVersionState {
    /// Create a new empty MVCC state.
    pub fn new() -> Self {
        Self {
            data: Arc::new(DashMap::new()),
            base_state: Arc::new(DashMap::new()),
            jmt: Arc::new(parking_lot::RwLock::new(
                crate::storage::jmt::JellyfishMerkleTree::new(),
            )),
        }
    }

    /// Create from existing base state.
    pub fn with_base_state(base: impl IntoIterator<Item = (StateKey, U256)>) -> Self {
        let base_state = Arc::new(DashMap::new());
        let jmt = crate::storage::jmt::JellyfishMerkleTree::new();
        // Since we don't bulk load JMT here easily, we start empty or would need to loop insert.
        // For tests, empty is fine or we can loop.
        let jmt = Arc::new(parking_lot::RwLock::new(jmt));

        for (key, value) in base {
            base_state.insert(key, value);
        }

        Self {
            data: Arc::new(DashMap::new()),
            base_state,
            jmt,
        }
    }

    // ========================================================================
    // READ OPERATIONS
    // ========================================================================

    /// Read state value for a specific transaction.
    ///
    /// Returns the value and the version number it came from.
    /// Version 0 indicates base state.
    ///
    /// # Read Resolution
    /// 1. Look for versions in MVCC with tx_seq < reader_seq
    /// 2. Skip ABORTED versions
    /// 3. Return latest COMMITTED or PENDING version
    /// 4. If no version found, fall back to base state
    pub fn read(&self, key: &StateKey, reader_seq: u64) -> (Option<U256>, u64) {
        // Check MVCC versions first
        if let Some(chain) = self.data.get(key) {
            if let Some((value, version)) = chain.read_for(reader_seq) {
                return (value, version);
            }
        }

        // Fall back to base state
        let value = self.base_state.get(key).map(|v| *v);
        (value, 0)
    }

    /// Read and record in a read set.
    pub fn read_tracked(
        &self,
        key: &StateKey,
        reader_seq: u64,
        read_set: &mut ReadSet,
    ) -> Option<U256> {
        let (value, version) = self.read(key, reader_seq);
        read_set.record(key.clone(), value, version);
        value
    }

    // ========================================================================
    // WRITE OPERATIONS
    // ========================================================================

    /// Write a new version of state.
    ///
    /// Creates a PENDING version that will be validated later.
    pub fn write(&self, key: StateKey, writer_seq: u64, value: Option<U256>) {
        let version = StateVersion {
            tx_sequence: writer_seq,
            value,
            status: TxStatus::Pending,
        };

        self.data
            .entry(key)
            .or_insert_with(VersionChain::new)
            .add_version(version);
    }

    /// Write and record in a write set.
    pub fn write_tracked(
        &self,
        key: StateKey,
        writer_seq: u64,
        value: Option<U256>,
        write_set: &mut WriteSet,
    ) {
        write_set.record(key.clone(), value);
        self.write(key, writer_seq, value);
    }

    // ========================================================================
    // STATUS MANAGEMENT
    // ========================================================================

    /// Mark a transaction's writes as committed.
    pub fn mark_committed(&self, key: &StateKey, tx_seq: u64) {
        if let Some(mut chain) = self.data.get_mut(key) {
            chain.mark_committed(tx_seq);
        }
    }

    /// Mark a transaction's writes as aborted.
    pub fn mark_aborted(&self, key: &StateKey, tx_seq: u64) {
        if let Some(mut chain) = self.data.get_mut(key) {
            chain.mark_aborted(tx_seq);
        }
    }

    /// Commit all writes from a write set.
    pub fn commit_write_set(&self, tx_seq: u64, write_set: &WriteSet) {
        for key in write_set.keys() {
            self.mark_committed(key, tx_seq);
        }
    }

    /// Abort all writes from a write set.
    pub fn abort_write_set(&self, tx_seq: u64, write_set: &WriteSet) {
        for key in write_set.keys() {
            self.mark_aborted(key, tx_seq);
        }
    }

    // ========================================================================
    // FINALIZATION
    // ========================================================================

    /// Apply all committed writes to base state.
    ///
    /// Called after a block is fully validated.
    /// Clears MVCC versions and updates base state.
    pub fn finalize(&self) {
        // For each key with versions, apply the latest committed value
        for entry in self.data.iter() {
            let key = entry.key().clone();
            let chain = entry.value();

            // Find the latest committed version
            if let Some((value, _)) = chain.read_for(u64::MAX) {
                match value {
                    Some(v) => {
                        self.base_state.insert(key.clone(), v);
                        // Update JMT
                        self.jmt.write().put(&key, &v);
                    }
                    None => {
                        self.base_state.remove(&key);
                        // JMT delete not implemented in mock yet, but we could put 0 or specialized delete
                    }
                }
            }
        }

        // Clear MVCC data
        self.data.clear();
    }

    /// Get current base state value (for debugging/testing).
    pub fn get_base(&self, key: &StateKey) -> Option<U256> {
        self.base_state.get(key).map(|v| *v)
    }

    /// Number of keys with MVCC versions.
    pub fn mvcc_key_count(&self) -> usize {
        self.data.len()
    }

    /// Number of keys in base state.
    pub fn base_key_count(&self) -> usize {
        self.base_state.len()
    }
}

impl Default for MultiVersionState {
    fn default() -> Self {
        Self::new()
    }
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
    fn test_read_write_basic() {
        let state = MultiVersionState::new();
        let key = make_key(1, 100);

        // Write value from tx 5
        state.write(key.clone(), 5, Some(U256::from_u64(500)));

        // Tx 10 should see it
        let (value, version) = state.read(&key, 10);
        assert_eq!(value, Some(U256::from_u64(500)));
        assert_eq!(version, 5);

        // Tx 3 should not see it
        let (value, version) = state.read(&key, 3);
        assert_eq!(value, None);
        assert_eq!(version, 0);
    }

    #[test]
    fn test_base_state_fallback() {
        let base = vec![(make_key(1, 100), U256::from_u64(1000))];
        let state = MultiVersionState::with_base_state(base);

        let key = make_key(1, 100);

        // Should read from base state
        let (value, version) = state.read(&key, 5);
        assert_eq!(value, Some(U256::from_u64(1000)));
        assert_eq!(version, 0); // Base state version
    }

    #[test]
    fn test_mvcc_overrides_base() {
        let base = vec![(make_key(1, 100), U256::from_u64(1000))];
        let state = MultiVersionState::with_base_state(base);
        let key = make_key(1, 100);

        // Write new version
        state.write(key.clone(), 5, Some(U256::from_u64(2000)));

        // Tx 10 should see MVCC version
        let (value, version) = state.read(&key, 10);
        assert_eq!(value, Some(U256::from_u64(2000)));
        assert_eq!(version, 5);

        // Tx 3 should still see base
        let (value, version) = state.read(&key, 3);
        assert_eq!(value, Some(U256::from_u64(1000)));
        assert_eq!(version, 0);
    }

    #[test]
    fn test_aborted_skipped() {
        let state = MultiVersionState::new();
        let key = make_key(1, 100);

        // Write from tx 5 (will be committed)
        state.write(key.clone(), 5, Some(U256::from_u64(500)));
        state.mark_committed(&key, 5);

        // Write from tx 8 (will be aborted)
        state.write(key.clone(), 8, Some(U256::from_u64(800)));
        state.mark_aborted(&key, 8);

        // Tx 10 should see tx 5's value (skipping aborted tx 8)
        let (value, version) = state.read(&key, 10);
        assert_eq!(value, Some(U256::from_u64(500)));
        assert_eq!(version, 5);
    }

    #[test]
    fn test_finalize() {
        let state = MultiVersionState::new();
        let key = make_key(1, 100);

        // Write and commit
        state.write(key.clone(), 5, Some(U256::from_u64(500)));
        state.mark_committed(&key, 5);

        // Finalize
        state.finalize();

        // MVCC should be clear
        assert_eq!(state.mvcc_key_count(), 0);

        // Base state should have the value
        assert_eq!(state.get_base(&key), Some(U256::from_u64(500)));
    }

    #[test]
    fn test_concurrent_writes() {
        use std::sync::Arc;
        use std::thread;

        let state = Arc::new(MultiVersionState::new());
        let mut handles = vec![];

        // 10 threads, each writing 100 keys
        for thread_id in 0..10 {
            let state = Arc::clone(&state);
            handles.push(thread::spawn(move || {
                for i in 0..100 {
                    let key = make_key(thread_id, i);
                    let value = U256::from_u64((thread_id as u64) * 1000 + i);
                    state.write(key, thread_id as u64, Some(value));
                }
            }));
        }

        for handle in handles {
            handle.join().unwrap();
        }

        // All writes should be present
        assert_eq!(state.mvcc_key_count(), 1000);
    }
}
