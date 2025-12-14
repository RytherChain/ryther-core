//! BLS12-381 signature implementation.
//!
//! Uses the `blst` crate for cryptographic operations.
//! BLS signatures enable efficient aggregation - multiple signatures
//! can be combined into a single signature that verifies against
//! multiple public keys.

use crate::types::ValidatorId;
use thiserror::Error;

/// BLS secret key (32 bytes scalar).
#[derive(Clone)]
pub struct SecretKey(blst::min_pk::SecretKey);

/// BLS public key (48 bytes compressed G1).
#[derive(Clone, Debug)]
pub struct PublicKey(blst::min_pk::PublicKey);

/// BLS signature (96 bytes compressed G2).
#[derive(Clone, Debug)]
pub struct Signature(blst::min_pk::Signature);

/// Aggregated BLS signature.
#[derive(Clone, Debug)]
pub struct AggregateSignature(blst::min_pk::AggregateSignature);

/// Domain separation tag for signing.
const DST: &[u8] = b"RYTHER-PROTOCOL-V1";

#[derive(Debug, Error)]
pub enum BlsError {
    #[error("Invalid secret key bytes")]
    InvalidSecretKey,
    
    #[error("Invalid public key bytes")]
    InvalidPublicKey,
    
    #[error("Invalid signature bytes")]
    InvalidSignature,
    
    #[error("Signature verification failed")]
    VerificationFailed,
    
    #[error("No signatures to aggregate")]
    EmptyAggregate,
}

impl SecretKey {
    /// Generate a new random secret key.
    pub fn generate() -> Self {
        use rand::RngCore;
        let mut ikm = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut ikm);
        Self(blst::min_pk::SecretKey::key_gen(&ikm, &[]).unwrap())
    }
    
    /// Create from raw bytes.
    pub fn from_bytes(bytes: &[u8; 32]) -> Result<Self, BlsError> {
        blst::min_pk::SecretKey::from_bytes(bytes)
            .map(Self)
            .map_err(|_| BlsError::InvalidSecretKey)
    }
    
    /// Export to raw bytes.
    pub fn to_bytes(&self) -> [u8; 32] {
        self.0.to_bytes()
    }
    
    /// Derive the public key.
    pub fn public_key(&self) -> PublicKey {
        PublicKey(self.0.sk_to_pk())
    }
    
    /// Sign a message.
    pub fn sign(&self, message: &[u8]) -> Signature {
        Signature(self.0.sign(message, DST, &[]))
    }
}

impl PublicKey {
    /// Create from compressed bytes.
    pub fn from_bytes(bytes: &[u8; 48]) -> Result<Self, BlsError> {
        blst::min_pk::PublicKey::from_bytes(bytes)
            .map(Self)
            .map_err(|_| BlsError::InvalidPublicKey)
    }
    
    /// Export to compressed bytes.
    pub fn to_bytes(&self) -> [u8; 48] {
        self.0.to_bytes()
    }
    
    /// Convert to ValidatorId format.
    pub fn to_validator_id(&self) -> ValidatorId {
        ValidatorId(self.to_bytes())
    }
    
    /// Verify a signature against this public key.
    pub fn verify(&self, signature: &Signature, message: &[u8]) -> Result<(), BlsError> {
        let result = signature.0.verify(true, message, DST, &[], &self.0, true);
        if result == blst::BLST_ERROR::BLST_SUCCESS {
            Ok(())
        } else {
            Err(BlsError::VerificationFailed)
        }
    }
}

impl Signature {
    /// Create from compressed bytes.
    pub fn from_bytes(bytes: &[u8; 96]) -> Result<Self, BlsError> {
        blst::min_pk::Signature::from_bytes(bytes)
            .map(Self)
            .map_err(|_| BlsError::InvalidSignature)
    }
    
    /// Export to compressed bytes.
    pub fn to_bytes(&self) -> [u8; 96] {
        self.0.to_bytes()
    }
    
    /// Convert to BlsSignature format.
    pub fn to_bls_signature(&self) -> crate::types::BlsSignature {
        crate::types::BlsSignature(self.to_bytes())
    }
}

impl AggregateSignature {
    /// Create from a single signature.
    pub fn from_signature(sig: &Signature) -> Self {
        let mut agg = blst::min_pk::AggregateSignature::from_signature(&sig.0);
        Self(agg)
    }
    
    /// Add a signature to the aggregate.
    pub fn add(&mut self, signature: &Signature) {
        self.0.add_signature(&signature.0, true).unwrap();
    }
    
    /// Finalize to a single signature.
    pub fn finalize(self) -> Signature {
        Signature(self.0.to_signature())
    }
    
    /// Aggregate multiple signatures at once.
    pub fn aggregate(signatures: &[Signature]) -> Result<Signature, BlsError> {
        if signatures.is_empty() {
            return Err(BlsError::EmptyAggregate);
        }
        
        let refs: Vec<_> = signatures.iter().map(|s| &s.0).collect();
        let agg = blst::min_pk::AggregateSignature::aggregate(&refs, true)
            .map_err(|_| BlsError::InvalidSignature)?;
        
        Ok(Signature(agg.to_signature()))
    }
}

/// Verify an aggregated signature against multiple public keys and messages.
/// 
/// Each public key must have signed its corresponding message.
pub fn verify_aggregate(
    signature: &Signature,
    public_keys: &[PublicKey],
    messages: &[&[u8]],
) -> Result<(), BlsError> {
    if public_keys.len() != messages.len() {
        return Err(BlsError::VerificationFailed);
    }
    
    let pk_refs: Vec<_> = public_keys.iter().map(|pk| &pk.0).collect();
    let result = signature.0.aggregate_verify(
        true,
        messages,
        DST,
        &pk_refs,
        true,
    );
    
    if result == blst::BLST_ERROR::BLST_SUCCESS {
        Ok(())
    } else {
        Err(BlsError::VerificationFailed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_sign_verify() {
        let sk = SecretKey::generate();
        let pk = sk.public_key();
        
        let message = b"Hello, Ryther!";
        let signature = sk.sign(message);
        
        assert!(pk.verify(&signature, message).is_ok());
    }
    
    #[test]
    fn test_invalid_signature() {
        let sk = SecretKey::generate();
        let pk = sk.public_key();
        
        let message = b"Hello, Ryther!";
        let signature = sk.sign(message);
        
        // Wrong message
        assert!(pk.verify(&signature, b"Wrong message").is_err());
    }
    
    #[test]
    fn test_aggregate_signatures() {
        let sk1 = SecretKey::generate();
        let sk2 = SecretKey::generate();
        let sk3 = SecretKey::generate();
        
        let pk1 = sk1.public_key();
        let pk2 = sk2.public_key();
        let pk3 = sk3.public_key();
        
        let msg1 = b"Message 1";
        let msg2 = b"Message 2";
        let msg3 = b"Message 3";
        
        let sig1 = sk1.sign(msg1);
        let sig2 = sk2.sign(msg2);
        let sig3 = sk3.sign(msg3);
        
        let agg = AggregateSignature::aggregate(&[sig1, sig2, sig3]).unwrap();
        
        let public_keys = vec![pk1, pk2, pk3];
        let messages: Vec<&[u8]> = vec![msg1, msg2, msg3];
        
        assert!(verify_aggregate(&agg, &public_keys, &messages).is_ok());
    }
    
    #[test]
    fn test_key_serialization() {
        let sk = SecretKey::generate();
        let pk = sk.public_key();
        
        let sk_bytes = sk.to_bytes();
        let pk_bytes = pk.to_bytes();
        
        let sk2 = SecretKey::from_bytes(&sk_bytes).unwrap();
        let pk2 = PublicKey::from_bytes(&pk_bytes).unwrap();
        
        // Verify recovered keys work
        let message = b"Test message";
        let sig = sk2.sign(message);
        assert!(pk2.verify(&sig, message).is_ok());
    }
}
