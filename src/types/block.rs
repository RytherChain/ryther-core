//! Block and round types for RPC responses.

use super::{sha256, Address, EventId};
use serde::{Deserialize, Serialize};
use serde_big_array::BigArray;

/// A block/round in the Ryther DAG.
/// In Ryther, a "block" is actually a committed round containing multiple DAG events.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Block {
    /// Block number (round number)
    #[serde(with = "hex_u64")]
    pub number: u64,

    /// Block hash (derived from committed events)
    #[serde(with = "hex_bytes32")]
    pub hash: [u8; 32],

    /// Parent block hash
    #[serde(with = "hex_bytes32")]
    pub parent_hash: [u8; 32],

    /// State root after executing this block
    #[serde(with = "hex_bytes32")]
    pub state_root: [u8; 32],

    /// Transactions root (merkle root of all tx commitments)
    #[serde(with = "hex_bytes32")]
    pub transactions_root: [u8; 32],

    /// Receipts root (merkle root of all receipts)
    #[serde(with = "hex_bytes32")]
    pub receipts_root: [u8; 32],

    /// Logs bloom filter
    #[serde(with = "hex_bytes256")]
    pub logs_bloom: [u8; 256],

    /// Block producer (leader for this round)
    pub miner: Address,

    /// Difficulty (always 0 for PoS/DAG)
    #[serde(with = "hex_u64")]
    pub difficulty: u64,

    /// Total difficulty
    #[serde(with = "hex_u64")]
    pub total_difficulty: u64,

    /// Extra data (can include validator signatures)
    #[serde(with = "hex_bytes")]
    pub extra_data: Vec<u8>,

    /// Block size in bytes
    #[serde(with = "hex_u64")]
    pub size: u64,

    /// Gas limit
    #[serde(with = "hex_u64")]
    pub gas_limit: u64,

    /// Gas used
    #[serde(with = "hex_u64")]
    pub gas_used: u64,

    /// Timestamp
    #[serde(with = "hex_u64")]
    pub timestamp: u64,

    /// Transaction hashes in this block
    pub transactions: Vec<TxHash>,

    /// Uncles (empty for DAG consensus)
    pub uncles: Vec<TxHash>,

    /// Base fee per gas (EIP-1559)
    #[serde(with = "hex_u64")]
    pub base_fee_per_gas: u64,

    /// DAG events that were committed in this round
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub dag_events: Vec<EventId>,
}

/// Transaction hash wrapper for hex serialization.
#[derive(Clone, Copy, Debug)]
pub struct TxHash(pub [u8; 32]);

impl Serialize for TxHash {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&format!("0x{}", hex::encode(&self.0)))
    }
}

impl<'de> Deserialize<'de> for TxHash {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let s = s.trim_start_matches("0x");
        let bytes = hex::decode(s).map_err(serde::de::Error::custom)?;
        if bytes.len() != 32 {
            return Err(serde::de::Error::custom("hash must be 32 bytes"));
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&bytes);
        Ok(TxHash(arr))
    }
}

impl From<[u8; 32]> for TxHash {
    fn from(bytes: [u8; 32]) -> Self {
        TxHash(bytes)
    }
}

impl Block {
    /// Create a genesis block.
    pub fn genesis(chain_id: u64) -> Self {
        Self {
            number: 0,
            hash: [0; 32],
            parent_hash: [0; 32],
            state_root: [0; 32],
            transactions_root: [0; 32],
            receipts_root: [0; 32],
            logs_bloom: [0; 256],
            miner: Address::zero(),
            difficulty: 0,
            total_difficulty: 0,
            extra_data: format!("Ryther Genesis - Chain {}", chain_id).into_bytes(),
            size: 0,
            gas_limit: 30_000_000,
            gas_used: 0,
            timestamp: 0,
            transactions: vec![],
            uncles: vec![],
            base_fee_per_gas: 1_000_000_000, // 1 gwei
            dag_events: vec![],
        }
    }

    /// Create a new block from committed events.
    pub fn from_round(
        number: u64,
        parent_hash: [u8; 32],
        state_root: [u8; 32],
        transactions: Vec<[u8; 32]>,
        dag_events: Vec<EventId>,
        leader: Address,
        gas_used: u64,
        timestamp: u64,
    ) -> Self {
        // Compute block hash
        let mut hash_input = Vec::new();
        hash_input.extend_from_slice(&number.to_le_bytes());
        hash_input.extend_from_slice(&parent_hash);
        hash_input.extend_from_slice(&state_root);
        let hash = sha256(&hash_input);

        // Compute transactions root
        let tx_root = if transactions.is_empty() {
            [0; 32]
        } else {
            let mut tx_data = Vec::new();
            for tx in &transactions {
                tx_data.extend_from_slice(tx);
            }
            sha256(&tx_data)
        };

        Self {
            number,
            hash,
            parent_hash,
            state_root,
            transactions_root: tx_root,
            receipts_root: [0; 32],
            logs_bloom: [0; 256],
            miner: leader,
            difficulty: 0,
            total_difficulty: 0,
            extra_data: vec![],
            size: (transactions.len() * 32) as u64,
            gas_limit: 30_000_000,
            gas_used,
            timestamp,
            transactions: transactions.into_iter().map(TxHash::from).collect(),
            uncles: vec![],
            base_fee_per_gas: 1_000_000_000,
            dag_events,
        }
    }
}

/// Transaction receipt.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransactionReceipt {
    /// Transaction hash
    pub transaction_hash: TxHash,

    /// Transaction index in block
    #[serde(with = "hex_u64")]
    pub transaction_index: u64,

    /// Block hash
    pub block_hash: TxHash,

    /// Block number
    #[serde(with = "hex_u64")]
    pub block_number: u64,

    /// Sender address
    pub from: Address,

    /// Recipient address (None for contract creation)
    pub to: Option<Address>,

    /// Cumulative gas used
    #[serde(with = "hex_u64")]
    pub cumulative_gas_used: u64,

    /// Gas used by this transaction
    #[serde(with = "hex_u64")]
    pub gas_used: u64,

    /// Contract address if this was a deployment
    pub contract_address: Option<Address>,

    /// Logs emitted
    pub logs: Vec<Log>,

    /// Logs bloom filter
    #[serde(with = "hex_bytes256")]
    pub logs_bloom: [u8; 256],

    /// Status (1 = success, 0 = failure)
    #[serde(with = "hex_u64")]
    pub status: u64,

    /// Effective gas price
    #[serde(with = "hex_u64")]
    pub effective_gas_price: u64,

    /// Transaction type
    #[serde(with = "hex_u64", rename = "type")]
    pub tx_type: u64,
}

/// Event log.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Log {
    /// Contract address that emitted the log
    pub address: Address,

    /// Indexed topics
    pub topics: Vec<TxHash>,

    /// Non-indexed data
    #[serde(with = "hex_bytes")]
    pub data: Vec<u8>,

    /// Block number
    #[serde(with = "hex_u64")]
    pub block_number: u64,

    /// Transaction hash
    pub transaction_hash: TxHash,

    /// Transaction index
    #[serde(with = "hex_u64")]
    pub transaction_index: u64,

    /// Block hash
    pub block_hash: TxHash,

    /// Log index in block
    #[serde(with = "hex_u64")]
    pub log_index: u64,

    /// Whether this log was removed (reorg)
    pub removed: bool,
}

/// Log filter for querying logs.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogFilter {
    /// From block
    pub from_block: Option<BlockId>,

    /// To block
    pub to_block: Option<BlockId>,

    /// Contract addresses to filter
    pub address: Option<Vec<Address>>,

    /// Topics to filter (each position can have multiple options)
    pub topics: Option<Vec<Option<Vec<TxHash>>>>,
}

/// Block identifier (number, hash, or tag).
#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum BlockId {
    /// Block number
    Number(u64),

    /// Block hash
    Hash(TxHash),

    /// Block tag (latest, earliest, pending)
    Tag(String),
}

impl Default for BlockId {
    fn default() -> Self {
        BlockId::Tag("latest".to_string())
    }
}

/// Helper module for hex serialization of u64.
mod hex_u64 {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(value: &u64, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&format!("0x{:x}", value))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<u64, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let s = s.trim_start_matches("0x");
        u64::from_str_radix(s, 16).map_err(serde::de::Error::custom)
    }
}

/// Helper module for hex serialization of bytes.
mod hex_bytes {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(value: &[u8], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&format!("0x{}", hex::encode(value)))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let s = s.trim_start_matches("0x");
        hex::decode(s).map_err(serde::de::Error::custom)
    }
}

/// Helper module for hex serialization of [u8; 32].
mod hex_bytes32 {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(value: &[u8; 32], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&format!("0x{}", hex::encode(value)))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<[u8; 32], D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let s = s.trim_start_matches("0x");
        let bytes = hex::decode(s).map_err(serde::de::Error::custom)?;
        if bytes.len() != 32 {
            return Err(serde::de::Error::custom("expected 32 bytes"));
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&bytes);
        Ok(arr)
    }
}

/// Helper module for hex serialization of [u8; 256].
mod hex_bytes256 {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(value: &[u8; 256], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&format!("0x{}", hex::encode(value)))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<[u8; 256], D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let s = s.trim_start_matches("0x");
        let bytes = hex::decode(s).map_err(serde::de::Error::custom)?;
        if bytes.len() != 256 {
            return Err(serde::de::Error::custom("expected 256 bytes"));
        }
        let mut arr = [0u8; 256];
        arr.copy_from_slice(&bytes);
        Ok(arr)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_genesis_block() {
        let genesis = Block::genesis(1);

        assert_eq!(genesis.number, 0);
        assert_eq!(genesis.gas_limit, 30_000_000);
        assert!(!genesis.extra_data.is_empty());
    }

    #[test]
    fn test_block_creation() {
        let block = Block::from_round(
            1,
            [0; 32],
            [1; 32],
            vec![[2; 32]],
            vec![[3; 32]],
            Address::zero(),
            21000,
            1234567890,
        );

        assert_eq!(block.number, 1);
        assert_eq!(block.transactions.len(), 1);
        assert_eq!(block.dag_events.len(), 1);
        assert_eq!(block.gas_used, 21000);
    }

    #[test]
    fn test_block_serialization() {
        let block = Block::genesis(1);
        let json = serde_json::to_string(&block).unwrap();

        assert!(json.contains("\"number\":\"0x0\""));
        assert!(json.contains("\"gasLimit\":\"0x1c9c380\""));
    }

    #[test]
    fn test_log_filter() {
        let filter = LogFilter {
            from_block: Some(BlockId::Number(100)),
            to_block: Some(BlockId::Tag("latest".to_string())),
            address: Some(vec![Address::zero()]),
            topics: None,
        };

        assert!(filter.address.is_some());
    }
}
