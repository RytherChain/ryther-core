//! Lamport Logical Clock implementation.
//!
//! Provides causal ordering in a distributed system without synchronized clocks.
//! 
//! # Invariants
//! - Clock value is monotonically non-decreasing
//! - For any event e: L(e) > L(p) for all parents p of e
//! - Thread-safe via atomic operations

use std::sync::atomic::{AtomicU64, Ordering};

/// Thread-safe Lamport logical clock.
/// 
/// Used to establish partial ordering of events in the DAG.
/// Each validator maintains its own clock instance.
#[derive(Debug)]
pub struct LamportClock {
    /// Current local time. Never decreases.
    value: AtomicU64,
}

impl LamportClock {
    /// Create a new clock starting at 0.
    pub const fn new() -> Self {
        Self {
            value: AtomicU64::new(0),
        }
    }
    
    /// Create a clock starting at a specific value.
    /// Used when recovering from persistent state.
    pub fn with_initial(initial: u64) -> Self {
        Self {
            value: AtomicU64::new(initial),
        }
    }
    
    /// Get current clock value without advancing.
    pub fn current(&self) -> u64 {
        self.value.load(Ordering::SeqCst)
    }
    
    /// Increment clock for local event creation.
    /// 
    /// Returns the NEW timestamp to use for the event.
    /// 
    /// # Example
    /// ```
    /// use ryther_core::types::lamport::LamportClock;
    /// let clock = LamportClock::new();
    /// assert_eq!(clock.tick(), 1);
    /// assert_eq!(clock.tick(), 2);
    /// ```
    pub fn tick(&self) -> u64 {
        self.value.fetch_add(1, Ordering::SeqCst) + 1
    }
    
    /// Update clock upon receiving an external event.
    /// 
    /// Implements: new_time = max(local_time, received_time) + 1
    /// 
    /// This ensures that:
    /// 1. Our clock is always ahead of any event we've seen
    /// 2. Causal ordering is preserved
    /// 
    /// # Arguments
    /// * `received` - Lamport timestamp from the received event
    /// 
    /// # Returns
    /// The new local clock value
    pub fn witness(&self, received: u64) -> u64 {
        loop {
            let current = self.value.load(Ordering::SeqCst);
            let new_time = std::cmp::max(current, received) + 1;
            
            // CAS loop to handle concurrent updates
            match self.value.compare_exchange(
                current,
                new_time,
                Ordering::SeqCst,
                Ordering::Relaxed,
            ) {
                Ok(_) => return new_time,
                Err(_) => continue, // Retry with new current value
            }
        }
    }
    
    /// Witness multiple timestamps at once (e.g., for multiple parents).
    /// 
    /// Computes: max(local, max(received...)) + 1
    pub fn witness_many<I: IntoIterator<Item = u64>>(&self, timestamps: I) -> u64 {
        let max_received = timestamps.into_iter().max().unwrap_or(0);
        self.witness(max_received)
    }
}

impl Default for LamportClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for LamportClock {
    /// Clone creates a new clock with the same current value.
    /// Note: The cloned clock is independent - updates don't propagate.
    fn clone(&self) -> Self {
        Self::with_initial(self.current())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    
    #[test]
    fn test_tick_increments() {
        let clock = LamportClock::new();
        assert_eq!(clock.current(), 0);
        assert_eq!(clock.tick(), 1);
        assert_eq!(clock.tick(), 2);
        assert_eq!(clock.tick(), 3);
        assert_eq!(clock.current(), 3);
    }
    
    #[test]
    fn test_witness_advances_past_received() {
        let clock = LamportClock::new();
        
        // Witness an event from the future
        let new_time = clock.witness(100);
        assert_eq!(new_time, 101);
        assert_eq!(clock.current(), 101);
        
        // Witness an event from the past - should still advance
        let new_time = clock.witness(50);
        assert_eq!(new_time, 102);
    }
    
    #[test]
    fn test_witness_many() {
        let clock = LamportClock::new();
        
        // Witness multiple parent timestamps
        let timestamps = vec![10, 50, 30, 25];
        let new_time = clock.witness_many(timestamps);
        
        // Should be max(50) + 1 = 51
        assert_eq!(new_time, 51);
    }
    
    #[test]
    fn test_monotonicity_invariant() {
        let clock = LamportClock::new();
        
        let mut last = 0u64;
        for _ in 0..100 {
            let current = clock.tick();
            assert!(current > last, "Clock must be monotonically increasing");
            last = current;
        }
    }
    
    #[test]
    fn test_concurrent_ticks() {
        use std::sync::Arc;
        
        let clock = Arc::new(LamportClock::new());
        let mut handles = vec![];
        
        // Spawn 10 threads, each doing 100 ticks
        for _ in 0..10 {
            let clock = Arc::clone(&clock);
            handles.push(thread::spawn(move || {
                for _ in 0..100 {
                    clock.tick();
                }
            }));
        }
        
        for handle in handles {
            handle.join().unwrap();
        }
        
        // Should have exactly 1000 ticks
        assert_eq!(clock.current(), 1000);
    }
    
    #[test]
    fn test_causal_ordering_property() {
        // If event A happens before event B (A -> B),
        // then L(A) < L(B)
        
        let clock_a = LamportClock::new();
        let clock_b = LamportClock::new();
        
        // A creates an event
        let timestamp_a = clock_a.tick();
        
        // B receives A's event and creates its own
        let timestamp_b = clock_b.witness(timestamp_a);
        let timestamp_b_event = clock_b.tick();
        
        // B's event must have higher timestamp than A's
        assert!(timestamp_b_event > timestamp_a);
        assert!(timestamp_b > timestamp_a);
    }
}
