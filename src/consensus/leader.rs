//! Deterministic post-facto leader election.
//!
//! Leaders are elected deterministically based on round number,
//! without any communication. All honest nodes compute the same leader.

use crate::types::{ValidatorId, sha256};
use crate::types::validator::ValidatorSet;

/// Elect leader for a given round.
///
/// This is a pure function - given the same inputs, all nodes
/// compute the same leader.
///
/// # Arguments
/// * `validator_set` - Current active validators with stakes
/// * `round` - The round number to elect leader for
/// * `seed` - Randomness seed (typically from previous committed round)
pub fn elect_leader(
    validator_set: &ValidatorSet,
    round: u64,
    seed: [u8; 32],
) -> Option<ValidatorId> {
    if validator_set.is_empty() {
        return None;
    }
    
    // Get validators in canonical (sorted) order
    let mut validators: Vec<_> = validator_set.iter()
        .map(|(id, stake)| (*id, *stake))
        .collect();
    validators.sort_by_key(|(id, _)| id.0);
    
    // Compute deterministic hash from round + seed
    let mut input = Vec::with_capacity(40);
    input.extend_from_slice(&round.to_le_bytes());
    input.extend_from_slice(&seed);
    let hash = sha256(&input);
    
    // Convert first 8 bytes to u64 for selection
    let random_value = u64::from_le_bytes(hash[0..8].try_into().unwrap());
    
    // Weighted selection by stake
    let target = random_value % validator_set.total_stake();
    
    let mut cumulative = 0u64;
    for (validator_id, stake) in validators {
        cumulative += stake;
        if cumulative > target {
            return Some(validator_id);
        }
    }
    
    // Should never reach here if validator set is non-empty
    None
}

/// Compute the seed for next round from current anchor.
pub fn derive_next_seed(anchor_id: [u8; 32]) -> [u8; 32] {
    sha256(&anchor_id)
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
    fn test_deterministic_leader() {
        let v1 = make_validator(1);
        let v2 = make_validator(2);
        let v3 = make_validator(3);
        
        let set = ValidatorSet::from_validators(vec![
            (v1, 100),
            (v2, 100),
            (v3, 100),
        ]);
        
        let seed = [0u8; 32];
        
        // Same inputs -> same leader
        let leader1 = elect_leader(&set, 5, seed);
        let leader2 = elect_leader(&set, 5, seed);
        
        assert_eq!(leader1, leader2);
    }
    
    #[test]
    fn test_different_rounds_different_leaders() {
        let v1 = make_validator(1);
        let v2 = make_validator(2);
        let v3 = make_validator(3);
        
        let set = ValidatorSet::from_validators(vec![
            (v1, 100),
            (v2, 100),
            (v3, 100),
        ]);
        
        let seed = [0u8; 32];
        
        // Different rounds may produce different leaders
        let mut leaders = std::collections::HashSet::new();
        for round in 0..100 {
            if let Some(leader) = elect_leader(&set, round, seed) {
                leaders.insert(leader);
            }
        }
        
        // With 100 rounds and 3 validators, we should see at least 2 different leaders
        assert!(leaders.len() >= 2);
    }
    
    #[test]
    fn test_weighted_selection() {
        let v1 = make_validator(1);
        let v2 = make_validator(2);
        
        // v1 has 90% stake, v2 has 10%
        let set = ValidatorSet::from_validators(vec![
            (v1, 900),
            (v2, 100),
        ]);
        
        let seed = [0u8; 32];
        
        // Run many rounds and count leader distribution
        let mut v1_count = 0;
        let mut v2_count = 0;
        
        for round in 0..1000 {
            match elect_leader(&set, round, seed) {
                Some(leader) if leader == v1 => v1_count += 1,
                Some(leader) if leader == v2 => v2_count += 1,
                _ => {}
            }
        }
        
        // v1 should be leader roughly 90% of the time
        // Allow some variance
        assert!(v1_count > 700, "v1 should be leader most of the time");
        assert!(v2_count > 30, "v2 should occasionally be leader");
    }
    
    #[test]
    fn test_empty_set() {
        let set = ValidatorSet::new();
        let leader = elect_leader(&set, 0, [0u8; 32]);
        assert!(leader.is_none());
    }
}
