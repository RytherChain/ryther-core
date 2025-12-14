//! Jellyfish Merkle Tree implementation.
//!
//! A sparse Merkle tree optimized for blockchain state storage.
//! Key features:
//! - 256-bit key space (for state keys)
//! - Only stores non-empty nodes
//! - Efficient proof generation
//! - Batched updates

use std::collections::HashMap;
use crate::types::{sha256, StateKey, U256};

/// Node hash (32 bytes).
pub type NodeHash = [u8; 32];

/// Empty hash constant (hash of nothing).
pub const EMPTY_HASH: NodeHash = [0u8; 32];

/// A node in the Jellyfish Merkle Tree.
#[derive(Clone, Debug)]
pub enum JmtNode {
    /// Internal node with two children
    Internal {
        left: NodeHash,
        right: NodeHash,
    },
    
    /// Leaf node containing actual value
    Leaf {
        /// Full key path to this leaf
        key_hash: [u8; 32],
        /// Value hash (hash of the actual value)
        value_hash: [u8; 32],
    },
    
    /// Empty placeholder (not stored)
    Empty,
}

impl JmtNode {
    /// Compute the hash of this node.
    pub fn hash(&self) -> NodeHash {
        match self {
            JmtNode::Empty => EMPTY_HASH,
            
            JmtNode::Leaf { key_hash, value_hash } => {
                let mut data = [0u8; 65];
                data[0] = 0x00; // Leaf marker
                data[1..33].copy_from_slice(key_hash);
                data[33..65].copy_from_slice(value_hash);
                sha256(&data)
            }
            
            JmtNode::Internal { left, right } => {
                let mut data = [0u8; 65];
                data[0] = 0x01; // Internal marker
                data[1..33].copy_from_slice(left);
                data[33..65].copy_from_slice(right);
                sha256(&data)
            }
        }
    }
    
    /// Check if node is empty.
    pub fn is_empty(&self) -> bool {
        matches!(self, JmtNode::Empty)
    }
}

/// Jellyfish Merkle Tree for authenticated state storage.
/// 
/// # Design
/// - 256 levels (one per bit of key hash)
/// - Internal nodes only created when paths diverge
/// - Leaves store (key_hash, value_hash) pairs
pub struct JellyfishMerkleTree {
    /// Node storage: hash -> node
    nodes: HashMap<NodeHash, JmtNode>,
    
    /// Current root hash
    root: NodeHash,
    
    /// Tree version (for snapshots)
    version: u64,
}

impl JellyfishMerkleTree {
    /// Create a new empty tree.
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            root: EMPTY_HASH,
            version: 0,
        }
    }
    
    /// Get the current root hash.
    pub fn root_hash(&self) -> NodeHash {
        self.root
    }
    
    /// Get current version.
    pub fn version(&self) -> u64 {
        self.version
    }
    
    /// Hash a state key to get the tree path.
    fn hash_key(key: &StateKey) -> [u8; 32] {
        sha256(&key.to_bytes())
    }
    
    /// Get bit at position in key hash (0 = leftmost).
    fn get_bit(key_hash: &[u8; 32], pos: usize) -> bool {
        let byte_idx = pos / 8;
        let bit_idx = 7 - (pos % 8);
        (key_hash[byte_idx] >> bit_idx) & 1 == 1
    }
    
    /// Insert or update a value.
    pub fn put(&mut self, key: &StateKey, value: &U256) -> NodeHash {
        let key_hash = Self::hash_key(key);
        let value_hash = sha256(&value.to_be_bytes());
        
        self.root = self.put_recursive(&self.root.clone(), &key_hash, &value_hash, 0);
        self.version += 1;
        self.root
    }
    
    /// Recursive insert implementation.
    fn put_recursive(
        &mut self,
        node_hash: &NodeHash,
        key_hash: &[u8; 32],
        value_hash: &[u8; 32],
        depth: usize,
    ) -> NodeHash {
        if depth >= 256 {
            // Maximum depth - create/update leaf
            let leaf = JmtNode::Leaf {
                key_hash: *key_hash,
                value_hash: *value_hash,
            };
            let hash = leaf.hash();
            self.nodes.insert(hash, leaf);
            return hash;
        }
        
        if *node_hash == EMPTY_HASH {
            // Empty slot - create leaf directly
            let leaf = JmtNode::Leaf {
                key_hash: *key_hash,
                value_hash: *value_hash,
            };
            let hash = leaf.hash();
            self.nodes.insert(hash, leaf);
            return hash;
        }
        
        let node = self.nodes.get(node_hash).cloned();
        
        match node {
            Some(JmtNode::Leaf { key_hash: existing_key, value_hash: existing_value }) => {
                if existing_key == *key_hash {
                    // Same key - update value
                    let leaf = JmtNode::Leaf {
                        key_hash: *key_hash,
                        value_hash: *value_hash,
                    };
                    let hash = leaf.hash();
                    self.nodes.insert(hash, leaf);
                    hash
                } else {
                    // Different key - split into internal node
                    let existing_bit = Self::get_bit(&existing_key, depth);
                    let new_bit = Self::get_bit(key_hash, depth);
                    
                    if existing_bit == new_bit {
                        // Same direction - recurse
                        let existing_leaf = JmtNode::Leaf {
                            key_hash: existing_key,
                            value_hash: existing_value,
                        };
                        let existing_hash = existing_leaf.hash();
                        self.nodes.insert(existing_hash, existing_leaf);
                        
                        let child_hash = self.put_recursive(&existing_hash, key_hash, value_hash, depth + 1);
                        
                        let internal = if new_bit {
                            JmtNode::Internal { left: EMPTY_HASH, right: child_hash }
                        } else {
                            JmtNode::Internal { left: child_hash, right: EMPTY_HASH }
                        };
                        let hash = internal.hash();
                        self.nodes.insert(hash, internal);
                        hash
                    } else {
                        // Different directions - create internal with both leaves
                        let existing_leaf = JmtNode::Leaf {
                            key_hash: existing_key,
                            value_hash: existing_value,
                        };
                        let existing_hash = existing_leaf.hash();
                        self.nodes.insert(existing_hash, existing_leaf.clone());
                        
                        let new_leaf = JmtNode::Leaf {
                            key_hash: *key_hash,
                            value_hash: *value_hash,
                        };
                        let new_hash = new_leaf.hash();
                        self.nodes.insert(new_hash, new_leaf);
                        
                        let internal = if new_bit {
                            JmtNode::Internal { left: existing_hash, right: new_hash }
                        } else {
                            JmtNode::Internal { left: new_hash, right: existing_hash }
                        };
                        let hash = internal.hash();
                        self.nodes.insert(hash, internal);
                        hash
                    }
                }
            }
            
            Some(JmtNode::Internal { left, right }) => {
                let bit = Self::get_bit(key_hash, depth);
                
                let (new_left, new_right) = if bit {
                    (left, self.put_recursive(&right, key_hash, value_hash, depth + 1))
                } else {
                    (self.put_recursive(&left, key_hash, value_hash, depth + 1), right)
                };
                
                let internal = JmtNode::Internal { left: new_left, right: new_right };
                let hash = internal.hash();
                self.nodes.insert(hash, internal);
                hash
            }
            
            _ => {
                // Unknown node - treat as empty
                let leaf = JmtNode::Leaf {
                    key_hash: *key_hash,
                    value_hash: *value_hash,
                };
                let hash = leaf.hash();
                self.nodes.insert(hash, leaf);
                hash
            }
        }
    }
    
    /// Get a value from the tree.
    pub fn get(&self, key: &StateKey) -> Option<[u8; 32]> {
        let key_hash = Self::hash_key(key);
        self.get_recursive(&self.root, &key_hash, 0)
    }
    
    /// Recursive get implementation.
    fn get_recursive(&self, node_hash: &NodeHash, key_hash: &[u8; 32], depth: usize) -> Option<[u8; 32]> {
        if *node_hash == EMPTY_HASH {
            return None;
        }
        
        let node = self.nodes.get(node_hash)?;
        
        match node {
            JmtNode::Leaf { key_hash: stored_key, value_hash } => {
                if stored_key == key_hash {
                    Some(*value_hash)
                } else {
                    None
                }
            }
            
            JmtNode::Internal { left, right } => {
                let bit = Self::get_bit(key_hash, depth);
                if bit {
                    self.get_recursive(right, key_hash, depth + 1)
                } else {
                    self.get_recursive(left, key_hash, depth + 1)
                }
            }
            
            JmtNode::Empty => None,
        }
    }
    
    /// Generate an inclusion/exclusion proof.
    pub fn get_proof(&self, key: &StateKey) -> JmtProof {
        let key_hash = Self::hash_key(key);
        let mut siblings = Vec::new();
        
        self.collect_proof_siblings(&self.root, &key_hash, 0, &mut siblings);
        
        JmtProof {
            key_hash,
            siblings,
            root: self.root,
        }
    }
    
    /// Collect sibling hashes for proof.
    fn collect_proof_siblings(
        &self,
        node_hash: &NodeHash,
        key_hash: &[u8; 32],
        depth: usize,
        siblings: &mut Vec<(usize, NodeHash)>,
    ) {
        if *node_hash == EMPTY_HASH || depth >= 256 {
            return;
        }
        
        let Some(node) = self.nodes.get(node_hash) else { return };
        
        match node {
            JmtNode::Internal { left, right } => {
                let bit = Self::get_bit(key_hash, depth);
                if bit {
                    siblings.push((depth, *left));
                    self.collect_proof_siblings(right, key_hash, depth + 1, siblings);
                } else {
                    siblings.push((depth, *right));
                    self.collect_proof_siblings(left, key_hash, depth + 1, siblings);
                }
            }
            JmtNode::Leaf { .. } => {
                // Reached leaf, no more siblings
            }
            JmtNode::Empty => {}
        }
    }
    
    /// Number of nodes stored.
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }
}

impl Default for JellyfishMerkleTree {
    fn default() -> Self {
        Self::new()
    }
}

/// Merkle proof for inclusion/exclusion.
#[derive(Clone, Debug)]
pub struct JmtProof {
    /// Hash of the key being proved
    pub key_hash: [u8; 32],
    
    /// Sibling hashes along the path: (depth, hash)
    pub siblings: Vec<(usize, NodeHash)>,
    
    /// Root hash at time of proof generation
    pub root: NodeHash,
}

impl JmtProof {
    /// Verify proof of inclusion with given value.
    pub fn verify_inclusion(&self, value_hash: &[u8; 32]) -> bool {
        let leaf = JmtNode::Leaf {
            key_hash: self.key_hash,
            value_hash: *value_hash,
        };
        
        let mut current = leaf.hash();
        
        for &(depth, sibling) in self.siblings.iter().rev() {
            let bit = JellyfishMerkleTree::get_bit(&self.key_hash, depth);
            
            let internal = if bit {
                JmtNode::Internal { left: sibling, right: current }
            } else {
                JmtNode::Internal { left: current, right: sibling }
            };
            
            current = internal.hash();
        }
        
        current == self.root
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
    fn test_empty_tree() {
        let tree = JellyfishMerkleTree::new();
        assert_eq!(tree.root_hash(), EMPTY_HASH);
        assert_eq!(tree.node_count(), 0);
    }
    
    #[test]
    fn test_single_insert() {
        let mut tree = JellyfishMerkleTree::new();
        let key = make_key(1, 100);
        let value = U256::from_u64(42);
        
        tree.put(&key, &value);
        
        assert_ne!(tree.root_hash(), EMPTY_HASH);
        assert!(tree.get(&key).is_some());
    }
    
    #[test]
    fn test_get_nonexistent() {
        let mut tree = JellyfishMerkleTree::new();
        let key1 = make_key(1, 100);
        let key2 = make_key(2, 200);
        let value = U256::from_u64(42);
        
        tree.put(&key1, &value);
        
        // key2 was never inserted
        assert!(tree.get(&key2).is_none());
    }
    
    #[test]
    fn test_update_value() {
        let mut tree = JellyfishMerkleTree::new();
        let key = make_key(1, 100);
        
        tree.put(&key, &U256::from_u64(10));
        let hash1 = tree.get(&key);
        
        tree.put(&key, &U256::from_u64(20));
        let hash2 = tree.get(&key);
        
        // Value changed
        assert_ne!(hash1, hash2);
    }
    
    #[test]
    fn test_multiple_keys() {
        let mut tree = JellyfishMerkleTree::new();
        
        for i in 0..10 {
            let key = make_key(i, i as u64 * 100);
            tree.put(&key, &U256::from_u64(i as u64));
        }
        
        // All keys should be retrievable
        for i in 0..10 {
            let key = make_key(i, i as u64 * 100);
            assert!(tree.get(&key).is_some());
        }
    }
    
    #[test]
    fn test_proof_verification() {
        let mut tree = JellyfishMerkleTree::new();
        let key = make_key(1, 100);
        let value = U256::from_u64(42);
        
        tree.put(&key, &value);
        
        let proof = tree.get_proof(&key);
        let value_hash = sha256(&value.to_be_bytes());
        
        assert!(proof.verify_inclusion(&value_hash));
    }
    
    #[test]
    fn test_root_changes_with_updates() {
        let mut tree = JellyfishMerkleTree::new();
        
        let root0 = tree.root_hash();
        
        tree.put(&make_key(1, 1), &U256::from_u64(1));
        let root1 = tree.root_hash();
        
        tree.put(&make_key(2, 2), &U256::from_u64(2));
        let root2 = tree.root_hash();
        
        // All roots should be different
        assert_ne!(root0, root1);
        assert_ne!(root1, root2);
        assert_ne!(root0, root2);
    }
}
