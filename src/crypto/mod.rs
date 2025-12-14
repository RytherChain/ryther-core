//! Cryptographic primitives for the Ryther Protocol.
//!
//! Provides BLS12-381 signatures and signature aggregation.

pub mod bls;

pub use bls::{SecretKey, PublicKey, Signature, AggregateSignature};
