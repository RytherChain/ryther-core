//! RytherDB Storage Layer.
//!
//! Implements persistent state storage using:
//! - Jellyfish Merkle Tree for authenticated state
//! - RocksDB-style LSM backend for persistence
//! - Async I/O primitives for non-blocking access

pub mod jmt;
pub mod page;
pub mod cache;

pub use jmt::{JellyfishMerkleTree, JmtProof};
pub use page::{PageId, Page, PageStore};
pub use cache::LruCache;
