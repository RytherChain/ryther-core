//! Transaction types for the Ryther Protocol.
//!
//! Supports both encrypted (pre-ordering) and decrypted (post-ordering) forms
//! to implement the commit-then-reveal MEV protection scheme.

use crate::types::{sha256, Address, U256};
use serde::{Deserialize, Serialize};

// ============================================================================
// ENCRYPTED FORM (Pre-Consensus)
// ============================================================================

/// A transaction encrypted by the user before submission.
///
/// Validators can only see the ciphertext until after ordering is finalized,
/// preventing front-running and sandwich attacks.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EncryptedTransaction {
    /// Encrypted transaction payload.
    /// Enc(tx_rlp, threshold_pubkey)
    pub ciphertext: Vec<u8>,

    /// Threshold decryption capsule/hint.
    /// Used by validators to collaboratively decrypt after commit.
    pub capsule: ThresholdCapsule,

    /// Hash commitment for deduplication.
    /// H(plaintext_tx) - allows detecting duplicates without decryption.
    pub commitment: [u8; 32],
}

/// Cryptographic capsule for threshold decryption.
/// Placeholder - actual implementation depends on chosen threshold scheme.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ThresholdCapsule {
    /// Ephemeral public key from encryptor
    pub ephemeral_pubkey: Vec<u8>,

    /// Encrypted symmetric key shares
    pub encrypted_shares: Vec<u8>,
}

/// Batch of encrypted transactions within a DAG event.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EncryptedBatch {
    /// Individual encrypted transactions
    pub transactions: Vec<EncryptedTransaction>,

    /// Merkle root of transaction commitments
    /// Allows proving inclusion without decryption
    pub batch_root: [u8; 32],
}

impl EncryptedBatch {
    /// Create an empty batch (for events without transactions).
    pub fn empty() -> Self {
        Self {
            transactions: vec![],
            batch_root: [0u8; 32], // Null root
        }
    }

    /// Create batch from encrypted transactions.
    pub fn from_transactions(txs: Vec<EncryptedTransaction>) -> Self {
        let batch_root = Self::compute_merkle_root(&txs);
        Self {
            transactions: txs,
            batch_root,
        }
    }

    /// Compute Merkle root of transaction commitments.
    fn compute_merkle_root(txs: &[EncryptedTransaction]) -> [u8; 32] {
        if txs.is_empty() {
            return [0u8; 32];
        }

        if txs.len() == 1 {
            // Single element: root is just the commitment
            return txs[0].commitment;
        }

        // Simple Merkle tree construction
        let mut layer: Vec<[u8; 32]> = txs.iter().map(|tx| tx.commitment).collect();

        while layer.len() > 1 {
            let mut next_layer = Vec::new();

            for pair in layer.chunks(2) {
                let hash = if pair.len() == 2 {
                    let mut combined = [0u8; 64];
                    combined[..32].copy_from_slice(&pair[0]);
                    combined[32..].copy_from_slice(&pair[1]);
                    sha256(&combined)
                } else {
                    // Odd element: hash with itself
                    let mut combined = [0u8; 64];
                    combined[..32].copy_from_slice(&pair[0]);
                    combined[32..].copy_from_slice(&pair[0]);
                    sha256(&combined)
                };
                next_layer.push(hash);
            }

            layer = next_layer;
        }

        layer[0]
    }

    /// Number of transactions in batch.
    pub fn len(&self) -> usize {
        self.transactions.len()
    }

    /// Check if batch is empty.
    pub fn is_empty(&self) -> bool {
        self.transactions.is_empty()
    }
}

// ============================================================================
// DECRYPTED FORM (Post-Consensus)
// ============================================================================

/// A decrypted transaction ready for EVM execution.
///
/// Created after consensus finalizes ordering and validators
/// perform threshold decryption.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DecryptedTransaction {
    // === Standard EVM fields ===
    /// Sender address (recovered from signature)
    pub from: Address,

    /// Recipient address (None = contract creation)
    pub to: Option<Address>,

    /// Value in wei
    pub value: U256,

    /// Gas limit
    pub gas_limit: u64,

    /// Gas price (or max fee for EIP-1559)
    pub gas_price: U256,

    /// Sender nonce
    pub nonce: u64,

    /// Calldata
    pub data: Vec<u8>,

    // === Ryther-specific ===
    /// Hash commitment from encrypted form (for verification)
    pub source_commitment: [u8; 32],

    /// Global sequence number (assigned after consensus)
    /// INVARIANT: Unique across all transactions in a finalized block
    pub sequence_number: u64,
}

impl DecryptedTransaction {
    /// Compute hash of this transaction for verification.
    pub fn compute_hash(&self) -> [u8; 32] {
        use sha3::{Digest, Keccak256};
        let mut hasher = Keccak256::new();

        hasher.update(&self.from);
        if let Some(to) = &self.to {
            hasher.update([1u8]); // Has 'to'
            hasher.update(to);
        } else {
            hasher.update([0u8]); // No 'to'
        }
        hasher.update(self.value.to_be_bytes());
        hasher.update(self.gas_limit.to_le_bytes());
        hasher.update(self.gas_price.to_be_bytes());
        hasher.update(self.nonce.to_le_bytes());
        hasher.update(&self.data);

        hasher.finalize().into()
    }

    /// Verify this transaction matches the encrypted commitment.
    pub fn verify_commitment(&self) -> bool {
        self.compute_hash() == self.source_commitment
    }

    /// Check if this is a contract creation transaction.
    pub fn is_create(&self) -> bool {
        self.to.is_none()
    }
}

/// A batch of decrypted transactions with assigned sequence numbers.
#[derive(Clone, Debug)]
pub struct OrderedBatch {
    /// Transactions in final execution order
    pub transactions: Vec<DecryptedTransaction>,

    /// First sequence number in this batch
    pub start_sequence: u64,

    /// Block/round this batch belongs to
    pub round: u64,
}

impl OrderedBatch {
    /// Create an ordered batch from decrypted transactions.
    pub fn new(txs: Vec<DecryptedTransaction>, start_seq: u64, round: u64) -> Self {
        let mut transactions = txs;

        // Assign sequential sequence numbers
        for (i, tx) in transactions.iter_mut().enumerate() {
            tx.sequence_number = start_seq + i as u64;
        }

        Self {
            transactions,
            start_sequence: start_seq,
            round,
        }
    }

    /// Get the next sequence number after this batch.
    pub fn end_sequence(&self) -> u64 {
        self.start_sequence + self.transactions.len() as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_batch() {
        let batch = EncryptedBatch::empty();
        assert!(batch.is_empty());
        assert_eq!(batch.batch_root, [0u8; 32]);
    }

    #[test]
    fn test_merkle_root_single() {
        let tx = EncryptedTransaction {
            ciphertext: vec![1, 2, 3],
            capsule: ThresholdCapsule::default(),
            commitment: [0xAB; 32],
        };

        let batch = EncryptedBatch::from_transactions(vec![tx.clone()]);

        // Single element: root = commitment directly
        assert_eq!(batch.batch_root, tx.commitment);
    }

    #[test]
    fn test_merkle_root_determinism() {
        let txs: Vec<_> = (0..5)
            .map(|i| EncryptedTransaction {
                ciphertext: vec![i],
                capsule: ThresholdCapsule::default(),
                commitment: [i; 32],
            })
            .collect();

        let batch1 = EncryptedBatch::from_transactions(txs.clone());
        let batch2 = EncryptedBatch::from_transactions(txs);

        assert_eq!(batch1.batch_root, batch2.batch_root);
    }

    #[test]
    fn test_ordered_batch_sequence_assignment() {
        let txs: Vec<_> = (0..3)
            .map(|i| DecryptedTransaction {
                from: Address([i as u8; 20]),
                to: Some(Address([0xFF; 20])),
                value: U256::ZERO,
                gas_limit: 21000,
                gas_price: U256::from_u64(1_000_000_000),
                nonce: i,
                data: vec![],
                source_commitment: [0; 32],
                sequence_number: 0, // Will be assigned
            })
            .collect();

        let batch = OrderedBatch::new(txs, 100, 5);

        assert_eq!(batch.transactions[0].sequence_number, 100);
        assert_eq!(batch.transactions[1].sequence_number, 101);
        assert_eq!(batch.transactions[2].sequence_number, 102);
        assert_eq!(batch.end_sequence(), 103);
    }
}
