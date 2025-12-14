//! Validator set and quorum tracking.
//!
//! Implements Byzantine quorum logic for the Helix consensus.

use std::collections::{HashMap, HashSet};
use serde::{Deserialize, Serialize};
use crate::types::ValidatorId;

/// Active validator set with stake weights.
/// 
/// # Invariants
/// - `total_stake == sum(validators.values())`
/// - All stake values are > 0
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ValidatorSet {
    /// Map: validator_id -> stake weight
    validators: HashMap<ValidatorId, u64>,
    
    /// Total stake (cached for quorum calculations)
    total_stake: u64,
}

impl ValidatorSet {
    /// Create an empty validator set.
    pub fn new() -> Self {
        Self {
            validators: HashMap::new(),
            total_stake: 0,
        }
    }
    
    /// Create from a list of (validator, stake) pairs.
    pub fn from_validators(validators: impl IntoIterator<Item = (ValidatorId, u64)>) -> Self {
        let mut set = Self::new();
        for (id, stake) in validators {
            set.add(id, stake);
        }
        set
    }
    
    /// Add a validator with given stake.
    pub fn add(&mut self, id: ValidatorId, stake: u64) {
        if stake == 0 {
            return;
        }
        
        if let Some(existing) = self.validators.insert(id, stake) {
            // Replacing existing validator
            self.total_stake = self.total_stake - existing + stake;
        } else {
            self.total_stake += stake;
        }
    }
    
    /// Remove a validator.
    pub fn remove(&mut self, id: &ValidatorId) -> Option<u64> {
        if let Some(stake) = self.validators.remove(id) {
            self.total_stake -= stake;
            Some(stake)
        } else {
            None
        }
    }
    
    /// Check if validator is in the set.
    pub fn contains(&self, id: &ValidatorId) -> bool {
        self.validators.contains_key(id)
    }
    
    /// Get stake for a validator.
    pub fn stake(&self, id: &ValidatorId) -> Option<u64> {
        self.validators.get(id).copied()
    }
    
    /// Total stake in the system.
    pub fn total_stake(&self) -> u64 {
        self.total_stake
    }
    
    /// Number of validators.
    pub fn len(&self) -> usize {
        self.validators.len()
    }
    
    /// Check if set is empty.
    pub fn is_empty(&self) -> bool {
        self.validators.is_empty()
    }
    
    /// Iterator over all validators.
    pub fn iter(&self) -> impl Iterator<Item = (&ValidatorId, &u64)> {
        self.validators.iter()
    }
    
    /// Get all validator IDs.
    pub fn validator_ids(&self) -> impl Iterator<Item = &ValidatorId> {
        self.validators.keys()
    }
    
    // ========================================================================
    // QUORUM LOGIC
    // ========================================================================
    
    /// Byzantine quorum threshold: >2/3 of total stake.
    /// 
    /// This is the minimum stake required for a quorum.
    /// INVARIANT: quorum_threshold > (2 * total_stake) / 3
    pub fn quorum_threshold(&self) -> u64 {
        // Integer division: (2n/3) + 1 gives us >2/3
        (self.total_stake * 2 / 3) + 1
    }
    
    /// Maximum Byzantine stake tolerable: <1/3 of total.
    pub fn fault_threshold(&self) -> u64 {
        self.total_stake / 3
    }
    
    /// Check if given set of validators forms a quorum.
    pub fn has_quorum(&self, voters: &HashSet<ValidatorId>) -> bool {
        let vote_stake: u64 = voters.iter()
            .filter_map(|v| self.validators.get(v))
            .sum();
        vote_stake >= self.quorum_threshold()
    }
    
    /// Calculate stake held by given validators.
    pub fn stake_for(&self, validators: &HashSet<ValidatorId>) -> u64 {
        validators.iter()
            .filter_map(|v| self.validators.get(v))
            .sum()
    }
    
    /// Check if two quorums must have an honest intersection.
    /// 
    /// With n=total and f<n/3 Byzantine:
    /// Two quorums Q1, Q2 with |Q1|, |Q2| > 2n/3 must share >n/3 stake,
    /// which means at least one honest validator.
    pub fn quorum_intersection_guaranteed(&self) -> bool {
        // This is always true if we're using >2/3 quorums
        // Just here for documentation
        true
    }
}

impl Default for ValidatorSet {
    fn default() -> Self {
        Self::new()
    }
}

/// Tracks progress toward quorum for a specific anchor/decision.
#[derive(Clone, Debug)]
pub struct QuorumTracker {
    /// The item being tracked (e.g., anchor event ID)
    pub target: [u8; 32],
    
    /// Validators who have supported this target
    supporters: HashSet<ValidatorId>,
    
    /// Reference to validator set for stake lookups
    validator_set: ValidatorSet,
}

impl QuorumTracker {
    /// Create a new tracker for a target.
    pub fn new(target: [u8; 32], validator_set: ValidatorSet) -> Self {
        Self {
            target,
            supporters: HashSet::new(),
            validator_set,
        }
    }
    
    /// Record a validator's support.
    /// Returns true if this caused quorum to be reached.
    pub fn add_support(&mut self, validator: ValidatorId) -> bool {
        let was_quorum = self.has_quorum();
        self.supporters.insert(validator);
        !was_quorum && self.has_quorum()
    }
    
    /// Check if quorum has been reached.
    pub fn has_quorum(&self) -> bool {
        self.validator_set.has_quorum(&self.supporters)
    }
    
    /// Current stake supporting.
    pub fn current_stake(&self) -> u64 {
        self.validator_set.stake_for(&self.supporters)
    }
    
    /// Stake needed for quorum.
    pub fn stake_needed(&self) -> u64 {
        let threshold = self.validator_set.quorum_threshold();
        let current = self.current_stake();
        threshold.saturating_sub(current)
    }
    
    /// Number of supporting validators.
    pub fn supporter_count(&self) -> usize {
        self.supporters.len()
    }
    
    /// Get all supporters.
    pub fn supporters(&self) -> &HashSet<ValidatorId> {
        &self.supporters
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    fn make_validator(id: u8) -> ValidatorId {
        let mut v = [0u8; 48];
        v[0] = id;
        ValidatorId(v)
    }
    
    #[test]
    fn test_quorum_threshold() {
        // 100 total stake -> need 67 for quorum
        let set = ValidatorSet::from_validators(vec![
            (make_validator(1), 50),
            (make_validator(2), 50),
        ]);
        
        assert_eq!(set.total_stake(), 100);
        assert_eq!(set.quorum_threshold(), 67); // 100 * 2/3 + 1
        assert_eq!(set.fault_threshold(), 33);  // 100 / 3
    }
    
    #[test]
    fn test_has_quorum() {
        let v1 = make_validator(1);
        let v2 = make_validator(2);
        let v3 = make_validator(3);
        
        let set = ValidatorSet::from_validators(vec![
            (v1, 34),
            (v2, 33),
            (v3, 33),
        ]);
        
        // Need 67 for quorum
        
        // Single validator: not enough
        let voters: HashSet<_> = [v1].into_iter().collect();
        assert!(!set.has_quorum(&voters));
        
        // Two validators: 34 + 33 = 67, enough!
        let voters: HashSet<_> = [v1, v2].into_iter().collect();
        assert!(set.has_quorum(&voters));
        
        // Two smallest: 33 + 33 = 66, not enough
        let voters: HashSet<_> = [v2, v3].into_iter().collect();
        assert!(!set.has_quorum(&voters));
        
        // All three: definitely enough
        let voters: HashSet<_> = [v1, v2, v3].into_iter().collect();
        assert!(set.has_quorum(&voters));
    }
    
    #[test]
    fn test_quorum_tracker() {
        let v1 = make_validator(1);
        let v2 = make_validator(2);
        let v3 = make_validator(3);
        
        let set = ValidatorSet::from_validators(vec![
            (v1, 34),
            (v2, 33),
            (v3, 33),
        ]);
        
        let mut tracker = QuorumTracker::new([0xAB; 32], set);
        
        // Add first supporter
        assert!(!tracker.add_support(v1));
        assert!(!tracker.has_quorum());
        assert_eq!(tracker.current_stake(), 34);
        
        // Add second supporter -> quorum!
        assert!(tracker.add_support(v2));
        assert!(tracker.has_quorum());
        assert_eq!(tracker.current_stake(), 67);
    }
    
    #[test]
    fn test_add_remove_validator() {
        let mut set = ValidatorSet::new();
        let v1 = make_validator(1);
        
        set.add(v1, 100);
        assert_eq!(set.total_stake(), 100);
        assert!(set.contains(&v1));
        
        set.remove(&v1);
        assert_eq!(set.total_stake(), 0);
        assert!(!set.contains(&v1));
    }
    
    #[test]
    fn test_stake_update() {
        let mut set = ValidatorSet::new();
        let v1 = make_validator(1);
        
        set.add(v1, 100);
        assert_eq!(set.total_stake(), 100);
        
        // Update stake
        set.add(v1, 150);
        assert_eq!(set.total_stake(), 150);
        assert_eq!(set.stake(&v1), Some(150));
    }
}
