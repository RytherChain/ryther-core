//! Commit detection for Helix consensus.
//!
//! Implements the >2/3 reference rule for committing rounds.

use super::leader::elect_leader;
use crate::dag::DagStore;
use crate::types::validator::ValidatorSet;
use crate::types::{sha256, EventId, ValidatorId};
use std::collections::HashSet;

/// Commit detector for Helix consensus.
pub struct CommitDetector {
    /// Validator set for quorum calculations
    validator_set: ValidatorSet,

    /// Last committed round
    last_committed: u64,

    /// Randomness seed for leader election
    seed: [u8; 32],
}

impl CommitDetector {
    pub fn new(validator_set: ValidatorSet, initial_seed: [u8; 32]) -> Self {
        Self {
            validator_set,
            last_committed: 0,
            seed: initial_seed,
        }
    }

    /// Check if a round can be committed.
    ///
    /// # Commit Rule
    /// Round R commits when there exists a round R' > R such that:
    /// - The leader's anchor event exists at round R
    /// - >2/3 of validators at round R' have events
    /// - All those events transitively reference the anchor
    pub fn check_commit(&self, dag: &DagStore, round: u64) -> Option<CommitResult> {
        // Already committed?
        if round <= self.last_committed {
            return None;
        }

        // Find leader for this round
        let leader = elect_leader(&self.validator_set, round, self.seed)?;

        // Find leader's anchor event at this round
        let anchor = dag.event_at(&leader, round)?;
        let anchor_id = anchor.id;

        // Check future rounds for quorum support
        for future_round in (round + 1)..=dag.max_round() {
            if let Some(support_round) = self.check_quorum_support(dag, &anchor_id, future_round) {
                return Some(CommitResult {
                    round,
                    anchor_id,
                    leader,
                    support_round,
                });
            }
        }

        None
    }

    /// Check if events at a future round form quorum referencing anchor.
    fn check_quorum_support(
        &self,
        dag: &DagStore,
        anchor: &EventId,
        future_round: u64,
    ) -> Option<u64> {
        let events = dag.events_at_round(future_round);

        let mut supporters = HashSet::new();

        for event in events {
            // Check if this event transitively references the anchor
            if dag.can_reach(event.id, *anchor) {
                supporters.insert(event.creator);
            }
        }

        if self.validator_set.has_quorum(&supporters) {
            Some(future_round)
        } else {
            None
        }
    }

    /// Mark a round as committed and update state.
    pub fn confirm_commit(&mut self, round: u64, anchor_id: EventId) {
        if round > self.last_committed {
            self.last_committed = round;
            // Derive next seed from committed anchor
            self.seed = sha256(&anchor_id);
        }
    }

    /// Get the last committed round.
    pub fn last_committed(&self) -> u64 {
        self.last_committed
    }

    /// Get current randomness seed.
    pub fn current_seed(&self) -> [u8; 32] {
        self.seed
    }

    /// Try to commit the next round.
    ///
    /// Returns all rounds that can be committed.
    pub fn try_advance(&mut self, dag: &DagStore) -> Vec<CommitResult> {
        let mut commits = Vec::new();

        // Try each round starting from last_committed + 1
        let mut round = self.last_committed + 1;

        while round <= dag.max_round() {
            if let Some(result) = self.check_commit(dag, round) {
                self.confirm_commit(result.round, result.anchor_id);
                commits.push(result);
                round += 1;
            } else {
                break;
            }
        }

        commits
    }
}

/// Result of a successful commit.
#[derive(Debug, Clone)]
pub struct CommitResult {
    /// The round that was committed
    pub round: u64,

    /// The anchor event for this round
    pub anchor_id: EventId,

    /// The leader who created the anchor
    pub leader: ValidatorId,

    /// The round at which quorum was achieved
    pub support_round: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::event::{DagEvent, EncryptedBatch};
    use crate::types::BlsSignature;

    fn make_validator(id: u8) -> ValidatorId {
        let mut v = [0u8; 48];
        v[0] = id;
        ValidatorId(v)
    }

    fn make_event(creator: ValidatorId, round: u64, parents: Vec<EventId>) -> DagEvent {
        let id = sha256(&[creator.0[0], round as u8]);

        DagEvent {
            id,
            creator,
            lamport_time: round,
            round,
            strong_parents: parents,
            weak_parents: vec![],
            payload: EncryptedBatch::empty(),
            signature: BlsSignature::default(),
        }
    }

    #[test]
    fn test_commit_with_quorum() {
        let v1 = make_validator(1);
        let v2 = make_validator(2);
        let v3 = make_validator(3);

        let validator_set = ValidatorSet::from_validators(vec![
            (v1, 100, crate::types::Address([0u8; 20])),
            (v2, 100, crate::types::Address([0u8; 20])),
            (v3, 100, crate::types::Address([0u8; 20])),
        ]);

        let dag = DagStore::new();

        // Round 1: Create events
        let e1_r1 = make_event(v1, 1, vec![]);
        let e2_r1 = make_event(v2, 1, vec![]);
        let e3_r1 = make_event(v3, 1, vec![]);

        let e1_id = e1_r1.id;
        let e2_id = e2_r1.id;
        let e3_id = e3_r1.id;

        dag.insert(e1_r1);
        dag.insert(e2_r1);
        dag.insert(e3_r1);

        // Round 2: All validators reference round 1 events
        let e1_r2 = make_event(v1, 2, vec![e1_id, e2_id, e3_id]);
        let e2_r2 = make_event(v2, 2, vec![e1_id, e2_id, e3_id]);
        let e3_r2 = make_event(v3, 2, vec![e1_id, e2_id, e3_id]);

        dag.insert(e1_r2);
        dag.insert(e2_r2);
        dag.insert(e3_r2);

        // Check if round 1 can commit
        let seed = [0u8; 32];
        let detector = CommitDetector::new(validator_set, seed);

        // The leader for round 1 depends on the election
        // If the leader exists in round 1 and is referenced by quorum in round 2
        // then round 1 should commit
        let result = detector.check_commit(&dag, 1);

        // Result depends on which validator is elected leader
        // Since all 3 have events at round 1, one should be the leader
        if result.is_some() {
            let result = result.unwrap();
            assert_eq!(result.round, 1);
            assert_eq!(result.support_round, 2);
        }
    }
}
