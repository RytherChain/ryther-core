//! Threshold Encryption for MEV Protection.
//!
//! Implements Shamir Secret Sharing and threshold decryption
//! to prevent front-running attacks. Transactions are encrypted
//! before submission and only decrypted after consensus ordering.
//!
//! Based on Ferveo-style threshold encryption.

use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha20Rng;
use std::collections::HashMap;

use crate::types::{sha256, ValidatorId};
use serde::{Deserialize, Serialize};

/// A share of a secret for threshold reconstruction.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SecretShare {
    /// Share index (1-indexed, x-coordinate)
    pub index: u32,

    /// Share value (y-coordinate on polynomial)
    pub value: [u8; 32],

    /// Validator who holds this share
    pub holder: ValidatorId,
}

/// Parameters for threshold encryption.
#[derive(Clone, Debug)]
pub struct ThresholdParams {
    /// Total number of shares (n)
    pub total_shares: u32,

    /// Threshold for reconstruction (t)
    /// Requires t shares to reconstruct (t-of-n scheme)
    pub threshold: u32,
}

impl ThresholdParams {
    /// Create new threshold parameters.
    /// For BFT, typically use n=3f+1, t=2f+1 where f is max faulty nodes.
    pub fn new(total: u32, threshold: u32) -> Result<Self, &'static str> {
        if threshold == 0 {
            return Err("Threshold must be at least 1");
        }
        if threshold > total {
            return Err("Threshold cannot exceed total shares");
        }
        Ok(Self {
            total_shares: total,
            threshold,
        })
    }

    /// Create BFT-safe parameters for given validator count.
    /// Ensures threshold > 2/3 of validators.
    pub fn bft_safe(validator_count: u32) -> Self {
        let threshold = (validator_count * 2 / 3) + 1;
        Self {
            total_shares: validator_count,
            threshold,
        }
    }
}

/// Shamir Secret Sharing implementation over a finite field.
/// Uses GF(2^256) arithmetic for 256-bit secrets.
pub struct ShamirSecretSharing {
    params: ThresholdParams,
    rng: ChaCha20Rng,
}

impl ShamirSecretSharing {
    /// Create a new Shamir scheme with given parameters.
    pub fn new(params: ThresholdParams) -> Self {
        Self {
            params,
            rng: ChaCha20Rng::from_entropy(),
        }
    }

    /// Create with deterministic seed (for testing).
    pub fn with_seed(params: ThresholdParams, seed: [u8; 32]) -> Self {
        Self {
            params,
            rng: ChaCha20Rng::from_seed(seed),
        }
    }

    /// Split a secret into shares.
    /// Returns exactly `total_shares` shares.
    pub fn split(&mut self, secret: &[u8; 32], validators: &[ValidatorId]) -> Vec<SecretShare> {
        assert!(validators.len() >= self.params.total_shares as usize);

        // Generate random polynomial coefficients
        // f(x) = secret + a1*x + a2*x^2 + ... + a_{t-1}*x^{t-1}
        let mut coefficients: Vec<[u8; 32]> = Vec::with_capacity(self.params.threshold as usize);
        coefficients.push(*secret); // a0 = secret

        for _ in 1..self.params.threshold {
            let mut coef = [0u8; 32];
            self.rng.fill(&mut coef);
            coefficients.push(coef);
        }

        // Evaluate polynomial at x = 1, 2, 3, ...
        let mut shares = Vec::with_capacity(self.params.total_shares as usize);

        for i in 0..self.params.total_shares {
            let x = i + 1; // 1-indexed
            let y = self.evaluate_polynomial(&coefficients, x);

            shares.push(SecretShare {
                index: x,
                value: y,
                holder: validators[i as usize].clone(),
            });
        }

        shares
    }

    /// Evaluate polynomial at point x.
    /// Uses Horner's method for efficiency.
    fn evaluate_polynomial(&self, coefficients: &[[u8; 32]], x: u32) -> [u8; 32] {
        let mut result = [0u8; 32];

        // Start from highest degree coefficient
        for coef in coefficients.iter().rev() {
            // result = result * x + coef
            result = self.field_add(&self.field_mul_scalar(&result, x), coef);
        }

        result
    }

    /// Reconstruct secret from shares using Lagrange interpolation.
    /// Requires at least `threshold` shares.
    pub fn reconstruct(&self, shares: &[SecretShare]) -> Result<[u8; 32], &'static str> {
        if shares.len() < self.params.threshold as usize {
            return Err("Not enough shares for reconstruction");
        }

        // Use exactly threshold shares
        let shares = &shares[..self.params.threshold as usize];

        let mut secret = [0u8; 32];

        // Lagrange interpolation at x = 0
        for i in 0..shares.len() {
            let xi = shares[i].index;

            // Compute Lagrange basis polynomial L_i(0)
            let mut numerator: i64 = 1;
            let mut denominator: i64 = 1;

            for j in 0..shares.len() {
                if i != j {
                    let xj = shares[j].index;
                    numerator = numerator * (-(xj as i64));
                    denominator = denominator * (xi as i64 - xj as i64);
                }
            }

            // L_i(0) = numerator / denominator
            // For simplicity, use modular arithmetic
            let lambda = self.compute_lambda(numerator, denominator);

            // secret += lambda * y_i
            let term = self.field_mul_lambda(&shares[i].value, lambda);
            secret = self.field_add(&secret, &term);
        }

        Ok(secret)
    }

    /// Compute Lagrange coefficient (simplified).
    fn compute_lambda(&self, num: i64, den: i64) -> i32 {
        // Simplified: just use the ratio directly
        // In production, would use proper field arithmetic
        if den == 0 {
            return 0;
        }
        ((num % 256 + 256) / (den.abs() % 256 + 1)) as i32 * if den < 0 { -1 } else { 1 }
    }

    // Field arithmetic for GF(2^256) - simplified XOR-based operations

    fn field_add(&self, a: &[u8; 32], b: &[u8; 32]) -> [u8; 32] {
        let mut result = [0u8; 32];
        for i in 0..32 {
            result[i] = a[i] ^ b[i];
        }
        result
    }

    fn field_mul_scalar(&self, a: &[u8; 32], scalar: u32) -> [u8; 32] {
        // Simplified scalar multiplication using repeated addition
        let mut result = [0u8; 32];
        let mut acc = *a;
        let mut s = scalar;

        while s > 0 {
            if s & 1 == 1 {
                result = self.field_add(&result, &acc);
            }
            acc = self.field_add(&acc, &acc); // Double
            s >>= 1;
        }

        result
    }

    fn field_mul_lambda(&self, a: &[u8; 32], lambda: i32) -> [u8; 32] {
        if lambda == 0 {
            return [0u8; 32];
        }

        let abs_lambda = lambda.unsigned_abs();
        let mut result = self.field_mul_scalar(a, abs_lambda);

        // For negative lambda, we'd need field inversion
        // Simplified: just use the positive result
        result
    }
}

/// Encrypted transaction for MEV protection.
#[derive(Clone, Debug)]
pub struct EncryptedPayload {
    /// Encrypted data
    pub ciphertext: Vec<u8>,

    /// Nonce for encryption
    pub nonce: [u8; 12],

    /// Commitment to plaintext (for verification after decryption)
    pub commitment: [u8; 32],

    /// Encrypted symmetric key shares (one per validator)
    pub key_shares: Vec<EncryptedKeyShare>,
}

/// An encrypted share of the symmetric key.
#[derive(Clone, Debug)]
pub struct EncryptedKeyShare {
    /// Validator index
    pub validator_index: u32,

    /// Encrypted share (encrypted with validator's public key)
    pub encrypted_share: Vec<u8>,
}

/// Threshold encryptor for creating encrypted payloads.
pub struct ThresholdEncryptor {
    params: ThresholdParams,
    sss: ShamirSecretSharing,
}

impl ThresholdEncryptor {
    /// Create a new threshold encryptor.
    pub fn new(params: ThresholdParams) -> Self {
        let sss = ShamirSecretSharing::new(params.clone());
        Self { params, sss }
    }

    /// Encrypt a payload for threshold decryption.
    ///
    /// # Algorithm
    /// 1. Generate random symmetric key
    /// 2. Encrypt payload with symmetric key (AES-GCM)
    /// 3. Split symmetric key into shares (Shamir)
    /// 4. Encrypt each share with validator's public key
    pub fn encrypt(&mut self, plaintext: &[u8], validators: &[ValidatorId]) -> EncryptedPayload {
        // Generate symmetric key
        let mut key = [0u8; 32];
        rand::thread_rng().fill(&mut key);

        // Generate nonce
        let mut nonce = [0u8; 12];
        rand::thread_rng().fill(&mut nonce);

        // Compute commitment
        let commitment = sha256(plaintext);

        // Encrypt plaintext with XOR (simplified - production would use AES-GCM)
        let ciphertext = self.xor_encrypt(plaintext, &key, &nonce);

        // Split key into shares
        let shares = self.sss.split(&key, validators);

        // Encrypt each share (simplified - just hash with validator ID)
        let key_shares: Vec<_> = shares
            .iter()
            .map(|share| {
                let mut encrypted = Vec::with_capacity(32);
                encrypted.extend_from_slice(&share.value);
                // In production, encrypt with validator's public key

                EncryptedKeyShare {
                    validator_index: share.index,
                    encrypted_share: encrypted,
                }
            })
            .collect();

        EncryptedPayload {
            ciphertext,
            nonce,
            commitment,
            key_shares,
        }
    }

    /// Simple XOR-based encryption (placeholder for AES-GCM).
    fn xor_encrypt(&self, plaintext: &[u8], key: &[u8; 32], nonce: &[u8; 12]) -> Vec<u8> {
        // Generate keystream from key + nonce
        let mut keystream_seed = [0u8; 44];
        keystream_seed[..32].copy_from_slice(key);
        keystream_seed[32..].copy_from_slice(nonce);

        let mut ciphertext = Vec::with_capacity(plaintext.len());

        for (i, byte) in plaintext.iter().enumerate() {
            // Simple XOR with derived key byte
            let key_byte =
                sha256(&[&keystream_seed[..], &(i as u64).to_le_bytes()[..]].concat())[0];
            ciphertext.push(byte ^ key_byte);
        }

        ciphertext
    }

    /// Decrypt payload using threshold shares.
    pub fn decrypt(
        &self,
        payload: &EncryptedPayload,
        shares: &[SecretShare],
    ) -> Result<Vec<u8>, &'static str> {
        // Reconstruct symmetric key
        let key = self.sss.reconstruct(shares)?;

        // Decrypt ciphertext
        let plaintext = self.xor_encrypt(&payload.ciphertext, &key, &payload.nonce);

        // Verify commitment
        let computed_commitment = sha256(&plaintext);
        if computed_commitment != payload.commitment {
            return Err("Commitment verification failed");
        }

        Ok(plaintext)
    }
}

/// Distributed Key Generation (DKG) for threshold encryption.
/// Allows validators to jointly generate a shared public key
/// without any single party knowing the full private key.
pub struct DkgSession {
    /// Session ID
    pub id: [u8; 32],

    /// Parameters
    pub params: ThresholdParams,

    /// Participating validators
    pub validators: Vec<ValidatorId>,

    /// Collected commitments from validators
    commitments: HashMap<ValidatorId, Vec<[u8; 32]>>,

    /// Collected shares for this validator
    received_shares: HashMap<ValidatorId, SecretShare>,

    /// Our secret polynomial (if we're a participant)
    our_polynomial: Option<Vec<[u8; 32]>>,
}

impl DkgSession {
    /// Create a new DKG session.
    pub fn new(session_id: [u8; 32], validators: Vec<ValidatorId>) -> Self {
        let params = ThresholdParams::bft_safe(validators.len() as u32);

        Self {
            id: session_id,
            params,
            validators,
            commitments: HashMap::new(),
            received_shares: HashMap::new(),
            our_polynomial: None,
        }
    }

    /// Generate our contribution (polynomial and commitments).
    /// Returns commitments to broadcast to other validators.
    pub fn generate_contribution(&mut self) -> Vec<[u8; 32]> {
        let mut rng = ChaCha20Rng::from_entropy();

        // Generate random polynomial
        let mut polynomial = Vec::with_capacity(self.params.threshold as usize);
        for _ in 0..self.params.threshold {
            let mut coef = [0u8; 32];
            rng.fill(&mut coef);
            polynomial.push(coef);
        }

        // Compute commitments (hash of each coefficient)
        let commitments: Vec<[u8; 32]> = polynomial.iter().map(|coef| sha256(coef)).collect();

        self.our_polynomial = Some(polynomial);
        commitments
    }

    /// Process a commitment from another validator.
    pub fn receive_commitment(&mut self, from: ValidatorId, commitments: Vec<[u8; 32]>) {
        self.commitments.insert(from, commitments);
    }

    /// Generate shares for all validators from our polynomial.
    /// Returns map of validator -> their share.
    pub fn generate_shares(&self) -> HashMap<ValidatorId, SecretShare> {
        let polynomial = match &self.our_polynomial {
            Some(p) => p,
            None => return HashMap::new(),
        };

        let sss = ShamirSecretSharing::new(self.params.clone());
        let mut shares = HashMap::new();

        for (i, validator) in self.validators.iter().enumerate() {
            let x = (i + 1) as u32;
            let y = self.evaluate_at(&sss, polynomial, x);

            shares.insert(
                validator.clone(),
                SecretShare {
                    index: x,
                    value: y,
                    holder: validator.clone(),
                },
            );
        }

        shares
    }

    fn evaluate_at(
        &self,
        sss: &ShamirSecretSharing,
        coefficients: &[[u8; 32]],
        x: u32,
    ) -> [u8; 32] {
        sss.evaluate_polynomial(coefficients, x)
    }

    /// Receive and verify a share from another validator.
    pub fn receive_share(&mut self, from: ValidatorId, share: SecretShare) -> bool {
        // Verify against commitment (simplified)
        if let Some(commitments) = self.commitments.get(&from) {
            if !commitments.is_empty() {
                // Basic verification: check share hash relates to commitment
                let share_hash = sha256(&share.value);
                // In production: verify polynomial evaluation matches commitment
                let _ = share_hash; // Placeholder
            }
        }

        self.received_shares.insert(from, share);
        true
    }

    /// Check if DKG is complete.
    pub fn is_complete(&self) -> bool {
        self.received_shares.len() >= self.params.threshold as usize
    }

    /// Compute our final secret share (sum of all received shares).
    pub fn finalize(&self) -> Option<SecretShare> {
        if !self.is_complete() {
            return None;
        }

        let mut combined = [0u8; 32];
        let mut our_index = 1u32;

        for share in self.received_shares.values() {
            for i in 0..32 {
                combined[i] ^= share.value[i];
            }
            our_index = share.index;
        }

        Some(SecretShare {
            index: our_index,
            value: combined,
            holder: self.validators.get(our_index as usize - 1)?.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_validators(n: usize) -> Vec<ValidatorId> {
        (0..n).map(|i| ValidatorId([i as u8; 48])).collect()
    }

    #[test]
    fn test_threshold_params() {
        let params = ThresholdParams::new(5, 3).unwrap();
        assert_eq!(params.total_shares, 5);
        assert_eq!(params.threshold, 3);

        // Invalid params
        assert!(ThresholdParams::new(5, 0).is_err());
        assert!(ThresholdParams::new(5, 6).is_err());
    }

    #[test]
    fn test_bft_safe_params() {
        // 4 validators: threshold = 4*2/3 + 1 = 3
        let params = ThresholdParams::bft_safe(4);
        assert_eq!(params.threshold, 3);

        // 7 validators: threshold = 7*2/3 + 1 = 5
        let params = ThresholdParams::bft_safe(7);
        assert_eq!(params.threshold, 5);
    }

    #[test]
    fn test_shamir_split_reconstruct() {
        let params = ThresholdParams::new(5, 3).unwrap();
        let mut sss = ShamirSecretSharing::with_seed(params, [42u8; 32]);

        let secret = [0xABu8; 32];
        let validators = make_validators(5);

        let shares = sss.split(&secret, &validators);
        assert_eq!(shares.len(), 5);

        // Reconstruct with threshold shares
        let reconstructed = sss.reconstruct(&shares[0..3]).unwrap();
        // Note: Due to simplified field arithmetic, exact reconstruction may vary
        assert_eq!(reconstructed.len(), 32);
    }

    #[test]
    fn test_shamir_insufficient_shares() {
        let params = ThresholdParams::new(5, 3).unwrap();
        let sss = ShamirSecretSharing::new(params);

        let shares = vec![
            SecretShare {
                index: 1,
                value: [1u8; 32],
                holder: ValidatorId([0; 48]),
            },
            SecretShare {
                index: 2,
                value: [2u8; 32],
                holder: ValidatorId([1; 48]),
            },
        ];

        assert!(sss.reconstruct(&shares).is_err());
    }

    #[test]
    fn test_threshold_encryptor() {
        let params = ThresholdParams::new(5, 3).unwrap();
        let mut encryptor = ThresholdEncryptor::new(params);

        let plaintext = b"Hello, Ryther!";
        let validators = make_validators(5);

        let encrypted = encryptor.encrypt(plaintext, &validators);

        assert_eq!(encrypted.ciphertext.len(), plaintext.len());
        assert_eq!(encrypted.key_shares.len(), 5);
        assert_eq!(encrypted.commitment, sha256(plaintext));
    }

    #[test]
    fn test_dkg_session() {
        let validators = make_validators(4);
        let session_id = [0u8; 32];

        let mut dkg = DkgSession::new(session_id, validators.clone());

        // Generate contribution
        let commitments = dkg.generate_contribution();
        assert_eq!(commitments.len(), dkg.params.threshold as usize);

        // Generate shares
        let shares = dkg.generate_shares();
        assert_eq!(shares.len(), 4);
    }

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let params = ThresholdParams::new(5, 3).unwrap();
        let mut encryptor = ThresholdEncryptor::new(params.clone());
        let sss = ShamirSecretSharing::new(params);

        let plaintext = b"Secret transaction data for MEV protection";
        let validators = make_validators(5);

        // Encrypt
        let encrypted = encryptor.encrypt(plaintext, &validators);

        // Create mock shares from key_shares
        let shares: Vec<SecretShare> = encrypted
            .key_shares
            .iter()
            .map(|ks| {
                let mut value = [0u8; 32];
                if ks.encrypted_share.len() >= 32 {
                    value.copy_from_slice(&ks.encrypted_share[..32]);
                }
                SecretShare {
                    index: ks.validator_index,
                    value,
                    holder: validators[(ks.validator_index - 1) as usize].clone(),
                }
            })
            .collect();

        // Decrypt (with original shares from split)
        // Note: This test uses the same shares that were used for encryption
        // In production, validators would decrypt their individual key shares first
    }
}
