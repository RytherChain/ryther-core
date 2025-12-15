//! DAG storage and traversal.

use crate::types::event::DagEvent;
use crate::types::{EventId, ValidatorId};
use dashmap::DashMap;
use std::collections::{HashMap, HashSet, VecDeque};

/// Thread-safe DAG storage.
pub struct DagStore {
    /// Events indexed by ID
    events: DashMap<EventId, DagEvent>,

    /// Events indexed by (creator, round)
    by_creator_round: DashMap<(ValidatorId, u64), EventId>,

    /// Events at each round
    by_round: DashMap<u64, Vec<EventId>>,

    /// Latest event from each validator
    latest_by_creator: DashMap<ValidatorId, EventId>,

    /// Maximum round seen
    max_round: std::sync::atomic::AtomicU64,
}

impl DagStore {
    pub fn new() -> Self {
        Self {
            events: DashMap::new(),
            by_creator_round: DashMap::new(),
            by_round: DashMap::new(),
            latest_by_creator: DashMap::new(),
            max_round: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// Insert an event into the DAG.
    pub fn insert(&self, event: DagEvent) {
        let id = event.id;
        let creator = event.creator;
        let round = event.round;

        // Update max round
        self.max_round
            .fetch_max(round, std::sync::atomic::Ordering::SeqCst);

        // Index by creator-round
        self.by_creator_round.insert((creator, round), id);

        // Index by round
        self.by_round.entry(round).or_default().push(id);

        // Update latest for creator
        if let Some(mut latest) = self.latest_by_creator.get_mut(&creator) {
            if let Some(existing) = self.events.get(&*latest) {
                if event.round > existing.round {
                    *latest = id;
                }
            }
        } else {
            self.latest_by_creator.insert(creator, id);
        }

        // Store event
        self.events.insert(id, event);
    }

    /// Get event by ID.
    pub fn get(&self, id: &EventId) -> Option<DagEvent> {
        self.events.get(id).map(|e| e.clone())
    }

    /// Check if event exists.
    pub fn contains(&self, id: &EventId) -> bool {
        self.events.contains_key(id)
    }

    /// Get latest event from a validator.
    pub fn latest_from(&self, creator: &ValidatorId) -> Option<EventId> {
        self.latest_by_creator.get(creator).map(|e| *e)
    }

    /// Get all events at a specific round.
    pub fn events_at_round(&self, round: u64) -> Vec<DagEvent> {
        self.by_round
            .get(&round)
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| self.events.get(id).map(|e| e.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Get event from specific creator at specific round.
    pub fn event_at(&self, creator: &ValidatorId, round: u64) -> Option<DagEvent> {
        self.by_creator_round
            .get(&(*creator, round))
            .and_then(|id| self.events.get(&*id).map(|e| e.clone()))
    }

    /// Maximum round in the DAG.
    pub fn max_round(&self) -> u64 {
        self.max_round.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Check if `from` transitively references `to`.
    /// Uses BFS through parent links.
    pub fn can_reach(&self, from: EventId, to: EventId) -> bool {
        if from == to {
            return true;
        }

        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        queue.push_back(from);

        while let Some(current) = queue.pop_front() {
            if current == to {
                return true;
            }

            if !visited.insert(current) {
                continue;
            }

            if let Some(event) = self.events.get(&current) {
                for parent in &event.strong_parents {
                    if !visited.contains(parent) {
                        queue.push_back(*parent);
                    }
                }
            }
        }

        false
    }

    /// Number of events in the DAG.
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// Check if DAG is empty.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Get all events (for iteration).
    pub fn all_events(&self) -> Vec<DagEvent> {
        self.events.iter().map(|e| e.clone()).collect()
    }
}

impl Default for DagStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::event::EncryptedBatch;
    use crate::types::BlsSignature;

    fn make_event(creator: u8, round: u64, parents: Vec<EventId>) -> DagEvent {
        let mut creator_id = [0u8; 48];
        creator_id[0] = creator;

        let id = crate::types::sha256(&[creator, round as u8]);

        DagEvent {
            id,
            creator: ValidatorId(creator_id),
            lamport_time: round,
            round,
            strong_parents: parents,
            weak_parents: vec![],
            payload: EncryptedBatch::empty(),
            signature: BlsSignature::default(),
        }
    }

    #[test]
    fn test_insert_and_get() {
        let store = DagStore::new();
        let event = make_event(1, 1, vec![]);
        let id = event.id;

        store.insert(event.clone());

        assert!(store.contains(&id));
        let retrieved = store.get(&id).unwrap();
        assert_eq!(retrieved.round, 1);
    }

    #[test]
    fn test_round_indexing() {
        let store = DagStore::new();

        store.insert(make_event(1, 1, vec![]));
        store.insert(make_event(2, 1, vec![]));
        store.insert(make_event(3, 2, vec![]));

        let round1 = store.events_at_round(1);
        assert_eq!(round1.len(), 2);

        let round2 = store.events_at_round(2);
        assert_eq!(round2.len(), 1);
    }

    #[test]
    fn test_can_reach() {
        let store = DagStore::new();

        let e1 = make_event(1, 1, vec![]);
        let e1_id = e1.id;
        store.insert(e1);

        let e2 = make_event(1, 2, vec![e1_id]);
        let e2_id = e2.id;
        store.insert(e2);

        let e3 = make_event(1, 3, vec![e2_id]);
        let e3_id = e3.id;
        store.insert(e3);

        // e3 can reach e1 through e2
        assert!(store.can_reach(e3_id, e1_id));

        // e1 cannot reach e3
        assert!(!store.can_reach(e1_id, e3_id));
    }
}
