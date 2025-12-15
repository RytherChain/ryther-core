//! Parallel Executor for RytherVM.
//!
//! Dispatches transactions for parallel speculative execution,
//! then validates in sequence order and re-executes conflicts.

use crossbeam::channel;
use std::collections::HashMap;
use std::sync::Arc;

use super::conflict::{validate_batch, ConflictDetector};
use super::mvcc::MultiVersionState;
use crate::evm::RytherVm;
use crate::types::state::{ExecutionResult, ExecutionStatus, Log, ReadSet, WriteSet};
use crate::types::transaction::DecryptedTransaction;
use crate::types::{Address, StateKey, U256};

/// Configuration for the parallel executor.
#[derive(Clone, Debug)]
pub struct ExecutorConfig {
    /// Number of worker threads for parallel execution
    pub worker_threads: usize,

    /// Maximum re-execution attempts per transaction
    pub max_retries: usize,
}

impl Default for ExecutorConfig {
    fn default() -> Self {
        Self {
            worker_threads: num_cpus::get().max(4),
            max_retries: 3,
        }
    }
}

/// Parallel execution engine for RytherVM.
pub struct ParallelExecutor {
    /// Configuration
    config: ExecutorConfig,

    /// MVCC state store
    state: Arc<MultiVersionState>,

    /// EVM execution engine
    vm: crate::evm::RytherVm,
}

impl ParallelExecutor {
    /// Create a new executor with given state.
    pub fn new(state: Arc<MultiVersionState>, vm: RytherVm, config: ExecutorConfig) -> Self {
        Self { config, state, vm }
    }

    /// Execute a batch of transactions in parallel.
    pub fn execute_batch(
        &self,
        transactions: Vec<DecryptedTransaction>,
        block: &crate::evm::vm::BlockContext,
    ) -> Vec<ExecutionResult> {
        if transactions.is_empty() {
            return vec![];
        }

        // Track transactions by sequence number
        let tx_map: HashMap<u64, DecryptedTransaction> = transactions
            .into_iter()
            .map(|tx| (tx.sequence_number, tx))
            .collect();

        let mut results: HashMap<u64, ExecutionResult> = HashMap::new();
        let mut pending: Vec<u64> = tx_map.keys().copied().collect();
        pending.sort();

        for attempt in 0..=self.config.max_retries {
            if pending.is_empty() {
                break;
            }

            // Phase 1: Parallel speculative execution
            let new_results = self.execute_parallel(&pending, &tx_map, block);

            // Store results
            for result in new_results {
                results.insert(result.tx_seq, result);
            }

            // Phase 2: Sequential validation
            let all_results: Vec<_> = results.values().cloned().collect();
            let validation = validate_batch(&all_results);

            // Mark committed transactions in MVCC
            for seq in &validation.committed {
                if let Some(result) = results.get(seq) {
                    self.state.commit_write_set(*seq, &result.write_set);
                }
            }

            // Mark aborted transactions
            for seq in &validation.aborted {
                if let Some(result) = results.get(seq) {
                    self.state.abort_write_set(*seq, &result.write_set);
                }
                results.remove(seq);
            }

            // Queue aborted for re-execution
            pending = validation.aborted;

            if pending.is_empty() {
                break;
            }

            tracing::debug!(
                attempt = attempt,
                aborted = pending.len(),
                "Re-executing conflicting transactions"
            );
        }

        // Return results in sequence order
        let mut final_results: Vec<_> = results.into_values().collect();
        final_results.sort_by_key(|r| r.tx_seq);
        final_results
    }

    /// Execute transactions in parallel (internal).
    fn execute_parallel(
        &self,
        sequences: &[u64],
        tx_map: &HashMap<u64, DecryptedTransaction>,
        block: &crate::evm::vm::BlockContext,
    ) -> Vec<ExecutionResult> {
        sequences
            .iter()
            .filter_map(|seq| tx_map.get(seq))
            .map(|tx| self.execute_single(tx, block))
            .collect()
    }

    /// Execute a single transaction speculatively.
    fn execute_single(
        &self,
        tx: &DecryptedTransaction,
        block: &crate::evm::vm::BlockContext,
    ) -> ExecutionResult {
        // Delegate to RytherVm
        self.vm.execute(tx, &self.state, block)
    }
}

/// Statistics from batch execution.
#[derive(Debug, Default)]
pub struct ExecutionStats {
    /// Total transactions processed
    pub total_txs: usize,

    /// Transactions that succeeded first try
    pub first_pass_success: usize,

    /// Total re-executions needed
    pub reexecutions: usize,

    /// Maximum re-executions for single tx
    pub max_retries_used: usize,

    /// Total gas consumed
    pub total_gas: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tx(seq: u64, from: u8, to: u8) -> DecryptedTransaction {
        DecryptedTransaction {
            from: Address([from; 20]),
            to: Some(Address([to; 20])),
            value: U256::from_u64(100),
            gas_limit: 21000,
            gas_price: U256::from_u64(1_000_000_000),
            nonce: 0,
            data: vec![],
            source_commitment: [0; 32],
            sequence_number: seq,
        }
    }

    #[test]
    fn test_execute_empty_batch() {
        let state = Arc::new(MultiVersionState::new());
        let vm = RytherVm::new(1);
        let executor = ParallelExecutor::new(state, vm, ExecutorConfig::default());
        let block = make_block_context();

        let results = executor.execute_batch(vec![], &block);

        assert!(results.is_empty());
    }

    // Helper for block context
    fn make_block_context() -> crate::evm::vm::BlockContext {
        crate::evm::vm::BlockContext {
            number: 1,
            timestamp: 1234567890,
            coinbase: crate::types::Address([0u8; 20]),
            gas_limit: 10_000_000,
            base_fee: crate::types::U256::ZERO,
            chain_id: 1,
        }
    }

    // Helper to create state with funded accounts
    fn make_funded_state(addresses: &[u8]) -> Arc<MultiVersionState> {
        let mut base = Vec::new();

        for &addr_byte in addresses {
            let addr = Address([addr_byte; 20]);

            // Fund balance
            let balance_key = StateKey {
                address: addr.0,
                slot: U256::from_u64(0), // SLOT_BALANCE
            };
            // 100 ETH
            let eth = U256::from_u64(1_000_000_000_000_000_000);
            let mut balance = U256::ZERO;
            // Simple multiplication loop since Mul not implemented
            for _ in 0..100 {
                balance = balance.wrapping_add(eth);
            }

            base.push((balance_key, balance));

            // Set nonce to 0 (default, but explicit)
            let nonce_key = StateKey {
                address: addr.0,
                slot: U256::from_u64(1), // SLOT_NONCE
            };
            base.push((nonce_key, U256::ZERO));
        }

        Arc::new(MultiVersionState::with_base_state(base))
    }

    #[test]
    fn test_execute_single_tx() {
        // Fund sender 0xAA
        let state = make_funded_state(&[0xAA]);
        let vm = RytherVm::new(1);
        let executor = ParallelExecutor::new(state, vm, ExecutorConfig::default());
        let block = make_block_context();

        let tx = make_tx(0, 0xAA, 0xBB);
        let results = executor.execute_batch(vec![tx], &block);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].tx_seq, 0);
        assert_eq!(results[0].status, ExecutionStatus::Success);
    }

    #[test]
    fn test_execute_independent_txs() {
        // Fund senders 0xAA, 0xBB, 0xEE
        let state = make_funded_state(&[0xAA, 0xBB, 0xEE]);
        let vm = RytherVm::new(1);
        let executor = ParallelExecutor::new(state, vm, ExecutorConfig::default());
        let block = make_block_context();

        // Independent transactions (different senders)
        let txs = vec![
            make_tx(0, 0xAA, 0xCC),
            make_tx(1, 0xBB, 0xDD),
            make_tx(2, 0xEE, 0xFF),
        ];

        let results = executor.execute_batch(txs, &block);

        assert_eq!(results.len(), 3);
        // Results should be in sequence order
        assert_eq!(results[0].tx_seq, 0);
        assert_eq!(results[1].tx_seq, 1);
        assert_eq!(results[2].tx_seq, 2);
    }

    #[test]
    fn test_results_ordered() {
        // Fund senders 0x33, 0x11, 0x22, 0x00
        let state = make_funded_state(&[0x33, 0x11, 0x22, 0x00]);
        let vm = RytherVm::new(1);
        let executor = ParallelExecutor::new(state, vm, ExecutorConfig::default());
        let block = make_block_context();

        // Submit out of order
        let txs = vec![
            make_tx(3, 0x33, 0xAA),
            make_tx(1, 0x11, 0xBB),
            make_tx(2, 0x22, 0xCC),
            make_tx(0, 0x00, 0xDD),
        ];

        let results = executor.execute_batch(txs, &block);

        // Should come back in sequence order
        for (i, result) in results.iter().enumerate() {
            assert_eq!(result.tx_seq, i as u64);
        }
    }
}
