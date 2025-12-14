//! Parallel Executor for RytherVM.
//!
//! Dispatches transactions for parallel speculative execution,
//! then validates in sequence order and re-executes conflicts.

use std::sync::Arc;
use std::collections::HashMap;
use crossbeam::channel;

use crate::types::transaction::DecryptedTransaction;
use crate::types::state::{ExecutionResult, ExecutionStatus, ReadSet, WriteSet, Log};
use crate::types::{StateKey, U256};
use super::mvcc::MultiVersionState;
use super::conflict::{ConflictDetector, validate_batch};

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
}

impl ParallelExecutor {
    /// Create a new executor with given state.
    pub fn new(state: Arc<MultiVersionState>, config: ExecutorConfig) -> Self {
        Self { config, state }
    }
    
    /// Execute a batch of transactions in parallel.
    /// 
    /// # Algorithm
    /// 1. Dispatch all transactions for parallel speculative execution
    /// 2. Collect results
    /// 3. Validate in sequence order, detecting conflicts
    /// 4. Re-execute conflicting transactions
    /// 5. Repeat until no conflicts or max retries
    /// 6. Return final ordered results
    pub fn execute_batch(
        &self,
        transactions: Vec<DecryptedTransaction>,
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
            let new_results = self.execute_parallel(&pending, &tx_map);
            
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
    ) -> Vec<ExecutionResult> {
        // For simplicity, use rayon-style parallel iteration
        // In production, would use dedicated worker pool
        
        sequences.iter()
            .filter_map(|seq| tx_map.get(seq))
            .map(|tx| self.execute_single(tx))
            .collect()
    }
    
    /// Execute a single transaction speculatively.
    fn execute_single(&self, tx: &DecryptedTransaction) -> ExecutionResult {
        let mut read_set = ReadSet::new();
        let mut write_set = WriteSet::new();
        
        // In a real implementation, this would:
        // 1. Create an EVM instance
        // 2. Execute bytecode step by step
        // 3. Track SLOAD -> read_set
        // 4. Track SSTORE -> write_set
        
        // For now, simulate basic execution
        let gas_used = self.simulate_execution(tx, &mut read_set, &mut write_set);
        
        // Apply writes to MVCC
        for (key, value) in &write_set.writes {
            self.state.write(key.clone(), tx.sequence_number, *value);
        }
        
        ExecutionResult {
            tx_seq: tx.sequence_number,
            status: ExecutionStatus::Success,
            read_set,
            write_set,
            gas_used,
            logs: vec![],
            output: vec![],
        }
    }
    
    /// Simulate transaction execution (placeholder for real EVM).
    fn simulate_execution(
        &self,
        tx: &DecryptedTransaction,
        read_set: &mut ReadSet,
        write_set: &mut WriteSet,
    ) -> u64 {
        // Simple transfer simulation
        let base_gas = 21000u64;
        
        if tx.to.is_some() {
            // Transfer: read sender balance, write to both
            let sender_balance_key = StateKey {
                address: tx.from,
                slot: U256::ZERO, // Balance slot
            };
            
            // Read sender balance
            let balance = self.state.read_tracked(
                &sender_balance_key,
                tx.sequence_number,
                read_set,
            ).unwrap_or(U256::ZERO);
            
            // Deduct value (simplified)
            let new_balance = balance; // Would subtract tx.value
            write_set.record(sender_balance_key, Some(new_balance));
        }
        
        base_gas + (tx.data.len() as u64 * 16) // 16 gas per calldata byte
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
            from: [from; 20],
            to: Some([to; 20]),
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
        let executor = ParallelExecutor::new(state, ExecutorConfig::default());
        
        let results = executor.execute_batch(vec![]);
        assert!(results.is_empty());
    }
    
    #[test]
    fn test_execute_single_tx() {
        let state = Arc::new(MultiVersionState::new());
        let executor = ParallelExecutor::new(state, ExecutorConfig::default());
        
        let tx = make_tx(0, 0xAA, 0xBB);
        let results = executor.execute_batch(vec![tx]);
        
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].tx_seq, 0);
        assert_eq!(results[0].status, ExecutionStatus::Success);
    }
    
    #[test]
    fn test_execute_independent_txs() {
        let state = Arc::new(MultiVersionState::new());
        let executor = ParallelExecutor::new(state, ExecutorConfig::default());
        
        // Independent transactions (different senders)
        let txs = vec![
            make_tx(0, 0xAA, 0xCC),
            make_tx(1, 0xBB, 0xDD),
            make_tx(2, 0xEE, 0xFF),
        ];
        
        let results = executor.execute_batch(txs);
        
        assert_eq!(results.len(), 3);
        // Results should be in sequence order
        assert_eq!(results[0].tx_seq, 0);
        assert_eq!(results[1].tx_seq, 1);
        assert_eq!(results[2].tx_seq, 2);
    }
    
    #[test]
    fn test_results_ordered() {
        let state = Arc::new(MultiVersionState::new());
        let executor = ParallelExecutor::new(state, ExecutorConfig::default());
        
        // Submit out of order
        let txs = vec![
            make_tx(3, 0x33, 0xAA),
            make_tx(1, 0x11, 0xBB),
            make_tx(2, 0x22, 0xCC),
            make_tx(0, 0x00, 0xDD),
        ];
        
        let results = executor.execute_batch(txs);
        
        // Should come back in sequence order
        for (i, result) in results.iter().enumerate() {
            assert_eq!(result.tx_seq, i as u64);
        }
    }
}
