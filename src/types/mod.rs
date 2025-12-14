//! Core type definitions for the Ryther Protocol.
//!
//! All fundamental types are defined here with explicit byte layouts
//! and invariant documentation.

pub mod lamport;
pub mod event;
pub mod transaction;
pub mod validator;
pub mod state;

use serde::{Deserialize, Serialize};
use std::fmt;

// ============================================================================
// IDENTITY TYPES
// ============================================================================

/// Unique identifier for a DAG event.
/// Computed as: sha256(creator || round || payload_hash || parent_hashes)
pub type EventId = [u8; 32];

/// BLS12-381 public key identifying a validator.
/// 48 bytes compressed G1 point.
/// Using a newtype wrapper for serde support.
#[derive(Clone, Copy, Hash, Eq, PartialEq, Ord, PartialOrd)]
pub struct ValidatorId(pub [u8; 48]);

impl ValidatorId {
    pub fn as_bytes(&self) -> &[u8; 48] {
        &self.0
    }
}

impl Default for ValidatorId {
    fn default() -> Self {
        Self([0u8; 48])
    }
}

impl From<[u8; 48]> for ValidatorId {
    fn from(bytes: [u8; 48]) -> Self {
        Self(bytes)
    }
}

impl AsRef<[u8]> for ValidatorId {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl fmt::Debug for ValidatorId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ValidatorId({}...)", hex::encode(&self.0[..4]))
    }
}

impl Serialize for ValidatorId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where S: serde::Serializer {
        serializer.serialize_bytes(&self.0)
    }
}

impl<'de> Deserialize<'de> for ValidatorId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where D: serde::Deserializer<'de> {
        let bytes: Vec<u8> = Vec::deserialize(deserializer)?;
        if bytes.len() != 48 {
            return Err(serde::de::Error::custom("ValidatorId must be 48 bytes"));
        }
        let mut arr = [0u8; 48];
        arr.copy_from_slice(&bytes);
        Ok(Self(arr))
    }
}

/// Ethereum-compatible 20-byte address.
pub type Address = [u8; 20];

/// BLS12-381 signature (96 bytes, compressed G2 point).
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct BlsSignature(pub [u8; 96]);

impl BlsSignature {
    pub fn as_bytes(&self) -> &[u8; 96] {
        &self.0
    }
}

impl Default for BlsSignature {
    fn default() -> Self {
        Self([0u8; 96])
    }
}

impl From<[u8; 96]> for BlsSignature {
    fn from(bytes: [u8; 96]) -> Self {
        Self(bytes)
    }
}

impl fmt::Debug for BlsSignature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "BlsSignature({}...)", hex::encode(&self.0[..8]))
    }
}

impl Serialize for BlsSignature {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where S: serde::Serializer {
        serializer.serialize_bytes(&self.0)
    }
}

impl<'de> Deserialize<'de> for BlsSignature {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where D: serde::Deserializer<'de> {
        let bytes: Vec<u8> = Vec::deserialize(deserializer)?;
        if bytes.len() != 96 {
            return Err(serde::de::Error::custom("BlsSignature must be 96 bytes"));
        }
        let mut arr = [0u8; 96];
        arr.copy_from_slice(&bytes);
        Ok(Self(arr))
    }
}

// ============================================================================
// STATE TYPES  
// ============================================================================

/// A key in the global state trie.
/// Uniquely identifies a storage slot.
#[derive(Clone, Copy, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub struct StateKey {
    /// Contract address
    pub address: Address,
    /// Storage slot (256-bit)
    pub slot: U256,
}

impl StateKey {
    pub fn new(address: Address, slot: U256) -> Self {
        Self { address, slot }
    }
    
    /// Convert to bytes for hashing/storage
    pub fn to_bytes(&self) -> [u8; 52] {
        let mut out = [0u8; 52];
        out[0..20].copy_from_slice(&self.address);
        out[20..52].copy_from_slice(&self.slot.to_be_bytes());
        out
    }
}

impl fmt::Debug for StateKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "StateKey({:?}:{})", 
            hex::encode(&self.address[..4]),
            self.slot)
    }
}

// ============================================================================
// NUMERIC TYPES
// ============================================================================

/// 256-bit unsigned integer for EVM compatibility.
/// Simple implementation - production would use `primitive-types` or `ruint`.
#[derive(Clone, Copy, Default, Hash, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub struct U256(pub [u64; 4]);

impl U256 {
    pub const ZERO: U256 = U256([0, 0, 0, 0]);
    pub const ONE: U256 = U256([1, 0, 0, 0]);
    
    pub fn from_u64(val: u64) -> Self {
        U256([val, 0, 0, 0])
    }
    
    pub fn to_be_bytes(&self) -> [u8; 32] {
        let mut out = [0u8; 32];
        for (i, limb) in self.0.iter().rev().enumerate() {
            out[i*8..(i+1)*8].copy_from_slice(&limb.to_be_bytes());
        }
        out
    }
    
    pub fn from_be_bytes(bytes: [u8; 32]) -> Self {
        let mut limbs = [0u64; 4];
        for (i, limb) in limbs.iter_mut().rev().enumerate() {
            let start = i * 8;
            *limb = u64::from_be_bytes(bytes[start..start+8].try_into().unwrap());
        }
        U256(limbs)
    }
    
    pub fn wrapping_add(self, other: Self) -> Self {
        let mut result = [0u64; 4];
        let mut carry = 0u64;
        
        for i in 0..4 {
            let (sum, c1) = self.0[i].overflowing_add(other.0[i]);
            let (sum, c2) = sum.overflowing_add(carry);
            result[i] = sum;
            carry = (c1 as u64) + (c2 as u64);
        }
        
        U256(result)
    }
}

impl fmt::Debug for U256 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "U256(0x{})", hex::encode(self.to_be_bytes()))
    }
}

impl fmt::Display for U256 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Simple decimal for small values
        if self.0[1] == 0 && self.0[2] == 0 && self.0[3] == 0 {
            write!(f, "{}", self.0[0])
        } else {
            write!(f, "0x{}", hex::encode(self.to_be_bytes()))
        }
    }
}

// ============================================================================
// UTILITY FUNCTIONS
// ============================================================================

/// Compute SHA-256 hash of input bytes.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    use sha2::{Sha256, Digest};
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize().into()
}

/// Compute Keccak-256 hash (Ethereum standard).
pub fn keccak256(data: &[u8]) -> [u8; 32] {
    use sha3::{Keccak256, Digest};
    let mut hasher = Keccak256::new();
    hasher.update(data);
    hasher.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_u256_roundtrip() {
        let original = U256([0xDEADBEEF, 0xCAFEBABE, 0x12345678, 0xABCDEF00]);
        let bytes = original.to_be_bytes();
        let recovered = U256::from_be_bytes(bytes);
        assert_eq!(original, recovered);
    }
    
    #[test]
    fn test_state_key_bytes() {
        let key = StateKey {
            address: [0x42; 20],
            slot: U256::from_u64(100),
        };
        let bytes = key.to_bytes();
        assert_eq!(bytes.len(), 52);
        assert_eq!(&bytes[0..20], &[0x42; 20]);
    }
    
    #[test]
    fn test_validator_id_serde() {
        let id = ValidatorId([0xAB; 48]);
        let serialized = bincode::serialize(&id).unwrap();
        let deserialized: ValidatorId = bincode::deserialize(&serialized).unwrap();
        assert_eq!(id, deserialized);
    }
}
