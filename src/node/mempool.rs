//! Transaction mempool.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use parking_lot::RwLock;

use crate::types::{Address, U256};
use crate::types::transaction::EncryptedTransaction;

/// Transaction in the mempool.
#[derive(Clone, Debug)]
pub struct PendingTransaction {
    /// The encrypted transaction
    pub transaction: EncryptedTransaction,
    
    /// Gas price (for ordering)
    pub gas_price: U256,
    
    /// When it was received
    pub received_at: std::time::Instant,
    
    /// Number of times broadcasted
    pub broadcast_count: u32,
}

/// Transaction pool for pending transactions.
pub struct TransactionPool {
    /// Transactions by commitment hash
    transactions: RwLock<HashMap<[u8; 32], PendingTransaction>>,
    
    /// Transactions ordered by gas price (descending)
    by_price: RwLock<BTreeMap<(U256, [u8; 32]), [u8; 32]>>,
    
    /// Known transaction hashes (for deduplication)
    known: RwLock<HashSet<[u8; 32]>>,
    
    /// Maximum pool size
    max_size: usize,
    
    /// Maximum transaction age (seconds)
    max_age_secs: u64,
}

impl TransactionPool {
    /// Create a new transaction pool.
    pub fn new(max_size: usize) -> Self {
        Self {
            transactions: RwLock::new(HashMap::new()),
            by_price: RwLock::new(BTreeMap::new()),
            known: RwLock::new(HashSet::new()),
            max_size,
            max_age_secs: 3600, // 1 hour
        }
    }
    
    /// Add a transaction to the pool.
    pub fn add(&self, tx: EncryptedTransaction, gas_price: U256) -> Result<(), PoolError> {
        let commitment = tx.commitment;
        
        // Check for duplicates
        if self.known.read().contains(&commitment) {
            return Err(PoolError::AlreadyKnown);
        }
        
        // Check pool capacity
        if self.transactions.read().len() >= self.max_size {
            // Try to evict lowest gas price transaction
            if !self.evict_lowest(gas_price) {
                return Err(PoolError::PoolFull);
            }
        }
        
        let pending = PendingTransaction {
            transaction: tx,
            gas_price,
            received_at: std::time::Instant::now(),
            broadcast_count: 0,
        };
        
        // Add to indices
        self.transactions.write().insert(commitment, pending);
        self.by_price.write().insert((gas_price, commitment), commitment);
        self.known.write().insert(commitment);
        
        Ok(())
    }
    
    /// Remove a transaction from the pool.
    pub fn remove(&self, commitment: &[u8; 32]) -> Option<PendingTransaction> {
        let pending = self.transactions.write().remove(commitment)?;
        self.by_price.write().remove(&(pending.gas_price, *commitment));
        // Keep in known set to prevent re-adding
        Some(pending)
    }
    
    /// Get a transaction by commitment.
    pub fn get(&self, commitment: &[u8; 32]) -> Option<PendingTransaction> {
        self.transactions.read().get(commitment).cloned()
    }
    
    /// Check if transaction is known.
    pub fn contains(&self, commitment: &[u8; 32]) -> bool {
        self.known.read().contains(commitment)
    }
    
    /// Get best transactions by gas price.
    pub fn best(&self, limit: usize) -> Vec<PendingTransaction> {
        let by_price = self.by_price.read();
        let transactions = self.transactions.read();
        
        by_price.values()
            .rev() // Highest gas price first
            .take(limit)
            .filter_map(|commitment| transactions.get(commitment).cloned())
            .collect()
    }
    
    /// Get all pending transactions.
    pub fn all(&self) -> Vec<PendingTransaction> {
        self.transactions.read().values().cloned().collect()
    }
    
    /// Number of transactions in pool.
    pub fn len(&self) -> usize {
        self.transactions.read().len()
    }
    
    /// Check if pool is empty.
    pub fn is_empty(&self) -> bool {
        self.transactions.read().is_empty()
    }
    
    /// Evict lowest gas price transaction if new tx has higher price.
    fn evict_lowest(&self, new_price: U256) -> bool {
        let lowest = {
            let by_price = self.by_price.read();
            by_price.keys().next().cloned()
        };
        
        if let Some((price, commitment)) = lowest {
            if new_price > price {
                self.remove(&commitment);
                return true;
            }
        }
        
        false
    }
    
    /// Remove expired transactions.
    pub fn prune_expired(&self) -> usize {
        let now = std::time::Instant::now();
        let max_age = std::time::Duration::from_secs(self.max_age_secs);
        
        let expired: Vec<_> = self.transactions.read()
            .iter()
            .filter(|(_, tx)| now.duration_since(tx.received_at) > max_age)
            .map(|(k, _)| *k)
            .collect();
        
        let count = expired.len();
        for commitment in expired {
            self.remove(&commitment);
        }
        
        count
    }
    
    /// Mark transactions as included (remove from pool).
    pub fn mark_included(&self, commitments: &[[u8; 32]]) -> usize {
        let mut count = 0;
        for commitment in commitments {
            if self.remove(commitment).is_some() {
                count += 1;
            }
        }
        count
    }
    
    /// Clear the known set (for testing).
    pub fn clear_known(&self) {
        self.known.write().clear();
    }
}

impl Default for TransactionPool {
    fn default() -> Self {
        Self::new(10000)
    }
}

/// Mempool errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PoolError {
    /// Transaction already known
    AlreadyKnown,
    
    /// Pool is full
    PoolFull,
    
    /// Gas price too low
    GasPriceTooLow,
    
    /// Invalid transaction
    Invalid(String),
}

impl std::fmt::Display for PoolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PoolError::AlreadyKnown => write!(f, "Transaction already known"),
            PoolError::PoolFull => write!(f, "Transaction pool is full"),
            PoolError::GasPriceTooLow => write!(f, "Gas price too low"),
            PoolError::Invalid(msg) => write!(f, "Invalid transaction: {}", msg),
        }
    }
}

impl std::error::Error for PoolError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::transaction::ThresholdCapsule;
    
    fn make_tx(id: u8, gas_price: u64) -> (EncryptedTransaction, U256) {
        let tx = EncryptedTransaction {
            ciphertext: vec![id],
            capsule: ThresholdCapsule::default(),
            commitment: [id; 32],
        };
        (tx, U256::from_u64(gas_price))
    }
    
    #[test]
    fn test_add_transaction() {
        let pool = TransactionPool::new(100);
        let (tx, price) = make_tx(1, 1000);
        
        assert!(pool.add(tx, price).is_ok());
        assert_eq!(pool.len(), 1);
    }
    
    #[test]
    fn test_duplicate_rejection() {
        let pool = TransactionPool::new(100);
        let (tx, price) = make_tx(1, 1000);
        
        assert!(pool.add(tx.clone(), price).is_ok());
        assert_eq!(pool.add(tx, price), Err(PoolError::AlreadyKnown));
    }
    
    #[test]
    fn test_best_by_price() {
        let pool = TransactionPool::new(100);
        
        pool.add(make_tx(1, 100).0, make_tx(1, 100).1).unwrap();
        pool.add(make_tx(2, 300).0, make_tx(2, 300).1).unwrap();
        pool.add(make_tx(3, 200).0, make_tx(3, 200).1).unwrap();
        
        let best = pool.best(2);
        
        assert_eq!(best.len(), 2);
        // First should have highest gas price
        assert_eq!(best[0].gas_price, U256::from_u64(300));
        assert_eq!(best[1].gas_price, U256::from_u64(200));
    }
    
    #[test]
    fn test_pool_capacity() {
        let pool = TransactionPool::new(2);
        
        pool.add(make_tx(1, 100).0, make_tx(1, 100).1).unwrap();
        pool.add(make_tx(2, 200).0, make_tx(2, 200).1).unwrap();
        
        // Pool full, but higher price should evict lowest
        assert!(pool.add(make_tx(3, 300).0, make_tx(3, 300).1).is_ok());
        assert_eq!(pool.len(), 2);
        
        // Lower price than all existing should fail
        assert_eq!(pool.add(make_tx(4, 50).0, make_tx(4, 50).1), Err(PoolError::PoolFull));
    }
    
    #[test]
    fn test_remove() {
        let pool = TransactionPool::new(100);
        let (tx, price) = make_tx(1, 1000);
        let commitment = tx.commitment;
        
        pool.add(tx, price).unwrap();
        assert!(pool.contains(&commitment));
        
        pool.remove(&commitment);
        assert!(pool.get(&commitment).is_none());
    }
}
