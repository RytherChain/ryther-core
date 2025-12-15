//! Block storage for RPC queries.

use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;

use crate::types::block::{Block, Log, TransactionReceipt, TxHash};
use crate::types::Address;

/// Block storage for RPC queries.
pub struct BlockStore {
    /// Blocks by number
    by_number: RwLock<HashMap<u64, Arc<Block>>>,

    /// Blocks by hash
    by_hash: RwLock<HashMap<[u8; 32], Arc<Block>>>,

    /// Transaction receipts by tx hash
    receipts: RwLock<HashMap<[u8; 32], TransactionReceipt>>,

    /// Transaction to block mapping
    tx_to_block: RwLock<HashMap<[u8; 32], u64>>,

    /// Logs by block number
    logs_by_block: RwLock<HashMap<u64, Vec<Log>>>,

    /// Latest block number
    latest: RwLock<u64>,
}

impl BlockStore {
    /// Create a new block store.
    pub fn new() -> Self {
        Self {
            by_number: RwLock::new(HashMap::new()),
            by_hash: RwLock::new(HashMap::new()),
            receipts: RwLock::new(HashMap::new()),
            tx_to_block: RwLock::new(HashMap::new()),
            logs_by_block: RwLock::new(HashMap::new()),
            latest: RwLock::new(0),
        }
    }

    /// Store a new block.
    pub fn insert_block(&self, block: Block) {
        let block_num = block.number;
        let block_hash = block.hash;
        let block = Arc::new(block);

        // Store transactions mapping
        for tx in &block.transactions {
            self.tx_to_block.write().insert(tx.0, block_num);
        }

        self.by_number.write().insert(block_num, Arc::clone(&block));
        self.by_hash.write().insert(block_hash, block);

        // Update latest
        let mut latest = self.latest.write();
        if block_num > *latest {
            *latest = block_num;
        }
    }

    /// Get block by number.
    pub fn get_by_number(&self, number: u64) -> Option<Arc<Block>> {
        self.by_number.read().get(&number).cloned()
    }

    /// Get block by hash.
    pub fn get_by_hash(&self, hash: &[u8; 32]) -> Option<Arc<Block>> {
        self.by_hash.read().get(hash).cloned()
    }

    /// Get latest block number.
    pub fn latest_number(&self) -> u64 {
        *self.latest.read()
    }

    /// Get latest block.
    pub fn latest_block(&self) -> Option<Arc<Block>> {
        let num = self.latest_number();
        self.get_by_number(num)
    }

    /// Store a transaction receipt.
    pub fn insert_receipt(&self, tx_hash: [u8; 32], receipt: TransactionReceipt) {
        // Store logs
        let block_num = receipt.block_number;
        {
            let mut logs = self.logs_by_block.write();
            let entry = logs.entry(block_num).or_insert_with(Vec::new);
            entry.extend(receipt.logs.clone());
        }

        self.receipts.write().insert(tx_hash, receipt);
    }

    /// Get transaction receipt.
    pub fn get_receipt(&self, tx_hash: &[u8; 32]) -> Option<TransactionReceipt> {
        self.receipts.read().get(tx_hash).cloned()
    }

    /// Get block number for a transaction.
    pub fn get_tx_block(&self, tx_hash: &[u8; 32]) -> Option<u64> {
        self.tx_to_block.read().get(tx_hash).copied()
    }

    /// Query logs with filter.
    pub fn get_logs(
        &self,
        from_block: u64,
        to_block: u64,
        addresses: Option<&[Address]>,
        topics: Option<&[Option<Vec<[u8; 32]>>]>,
    ) -> Vec<Log> {
        let logs_by_block = self.logs_by_block.read();
        let mut result = Vec::new();

        for block_num in from_block..=to_block {
            if let Some(block_logs) = logs_by_block.get(&block_num) {
                for log in block_logs {
                    // Filter by address
                    if let Some(addrs) = addresses {
                        if !addrs.contains(&log.address) {
                            continue;
                        }
                    }

                    // Filter by topics
                    if let Some(topic_filters) = topics {
                        let mut matches = true;
                        for (i, topic_filter) in topic_filters.iter().enumerate() {
                            if let Some(allowed) = topic_filter {
                                if i >= log.topics.len() {
                                    matches = false;
                                    break;
                                }
                                if !allowed.contains(&log.topics[i].0) {
                                    matches = false;
                                    break;
                                }
                            }
                        }
                        if !matches {
                            continue;
                        }
                    }

                    result.push(log.clone());
                }
            }
        }

        result
    }

    /// Number of stored blocks.
    pub fn block_count(&self) -> usize {
        self.by_number.read().len()
    }
}

impl Default for BlockStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::block::Block;

    #[test]
    fn test_insert_and_get_block() {
        let store = BlockStore::new();
        let block = Block::genesis(1);
        let hash = block.hash;

        store.insert_block(block);

        assert!(store.get_by_number(0).is_some());
        assert!(store.get_by_hash(&hash).is_some());
        assert_eq!(store.latest_number(), 0);
    }

    #[test]
    fn test_latest_tracking() {
        let store = BlockStore::new();

        store.insert_block(Block::genesis(1));
        assert_eq!(store.latest_number(), 0);

        let block1 =
            Block::from_round(1, [0; 32], [1; 32], vec![], vec![], Address::zero(), 0, 100);
        store.insert_block(block1);
        assert_eq!(store.latest_number(), 1);

        let block2 =
            Block::from_round(2, [0; 32], [2; 32], vec![], vec![], Address::zero(), 0, 200);
        store.insert_block(block2);
        assert_eq!(store.latest_number(), 2);
    }
}
