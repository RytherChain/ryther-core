//! LRU Cache for hot state access.
//!
//! Provides a bounded cache with least-recently-used eviction
//! for frequently accessed state values.

use parking_lot::Mutex;
use std::collections::HashMap;
use std::hash::Hash;

/// Entry in the LRU cache.
struct CacheEntry<V> {
    value: V,
    prev: Option<usize>,
    next: Option<usize>,
}

/// LRU cache with bounded capacity.
///
/// Thread-safe via internal mutex.
pub struct LruCache<K, V> {
    inner: Mutex<LruCacheInner<K, V>>,
}

struct LruCacheInner<K, V> {
    /// Key to index mapping
    map: HashMap<K, usize>,

    /// Entry storage (indices are stable)
    entries: Vec<Option<CacheEntry<V>>>,

    /// Free list of available indices
    free_list: Vec<usize>,

    /// Head of LRU list (most recently used)
    head: Option<usize>,

    /// Tail of LRU list (least recently used)
    tail: Option<usize>,

    /// Maximum capacity
    capacity: usize,

    /// Current size
    size: usize,

    /// Statistics
    hits: u64,
    misses: u64,
}

impl<K, V> LruCache<K, V>
where
    K: Hash + Eq + Clone,
    V: Clone,
{
    /// Create a new cache with given capacity.
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        Self {
            inner: Mutex::new(LruCacheInner {
                map: HashMap::with_capacity(capacity),
                entries: Vec::with_capacity(capacity),
                free_list: Vec::new(),
                head: None,
                tail: None,
                capacity,
                size: 0,
                hits: 0,
                misses: 0,
            }),
        }
    }

    /// Get a value from the cache.
    /// Moves the entry to front (most recently used).
    pub fn get(&self, key: &K) -> Option<V> {
        let mut inner = self.inner.lock();

        let idx = match inner.map.get(key).copied() {
            Some(idx) => idx,
            None => {
                inner.misses += 1;
                return None;
            }
        };

        inner.hits += 1;
        inner.move_to_front(idx);

        inner.entries[idx].as_ref().map(|e| e.value.clone())
    }

    /// Insert a value into the cache.
    /// Evicts least recently used if at capacity.
    pub fn put(&self, key: K, value: V) -> Option<V> {
        let mut inner = self.inner.lock();

        // Check if key already exists
        if let Some(&idx) = inner.map.get(&key) {
            let old_value = inner.entries[idx].as_ref().map(|e| e.value.clone());

            if let Some(entry) = inner.entries[idx].as_mut() {
                entry.value = value;
            }

            inner.move_to_front(idx);
            return old_value;
        }

        // Evict if at capacity
        let evicted = if inner.size >= inner.capacity {
            inner.evict_lru()
        } else {
            None
        };

        // Allocate new entry
        let idx = inner.allocate_entry(value);
        inner.map.insert(key, idx);
        inner.add_to_front(idx);
        inner.size += 1;

        evicted
    }

    /// Remove a value from the cache.
    pub fn remove(&self, key: &K) -> Option<V> {
        let mut inner = self.inner.lock();

        let idx = inner.map.remove(key)?;
        inner.remove_from_list(idx);

        let entry = inner.entries[idx].take();
        inner.free_list.push(idx);
        inner.size -= 1;

        entry.map(|e| e.value)
    }

    /// Check if key exists.
    pub fn contains(&self, key: &K) -> bool {
        self.inner.lock().map.contains_key(key)
    }

    /// Current number of entries.
    pub fn len(&self) -> usize {
        self.inner.lock().size
    }

    /// Check if cache is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Clear all entries.
    pub fn clear(&self) {
        let mut inner = self.inner.lock();
        inner.map.clear();
        inner.entries.clear();
        inner.free_list.clear();
        inner.head = None;
        inner.tail = None;
        inner.size = 0;
    }

    /// Get cache statistics.
    pub fn stats(&self) -> CacheStats {
        let inner = self.inner.lock();
        CacheStats {
            size: inner.size,
            capacity: inner.capacity,
            hits: inner.hits,
            misses: inner.misses,
        }
    }
}

impl<K, V> LruCacheInner<K, V>
where
    V: Clone,
{
    fn allocate_entry(&mut self, value: V) -> usize {
        if let Some(idx) = self.free_list.pop() {
            self.entries[idx] = Some(CacheEntry {
                value,
                prev: None,
                next: None,
            });
            idx
        } else {
            let idx = self.entries.len();
            self.entries.push(Some(CacheEntry {
                value,
                prev: None,
                next: None,
            }));
            idx
        }
    }

    fn add_to_front(&mut self, idx: usize) {
        if let Some(entry) = self.entries[idx].as_mut() {
            entry.prev = None;
            entry.next = self.head;
        }

        if let Some(old_head) = self.head {
            if let Some(entry) = self.entries[old_head].as_mut() {
                entry.prev = Some(idx);
            }
        }

        self.head = Some(idx);

        if self.tail.is_none() {
            self.tail = Some(idx);
        }
    }

    fn remove_from_list(&mut self, idx: usize) {
        let (prev, next) = match self.entries[idx].as_ref() {
            Some(entry) => (entry.prev, entry.next),
            None => return,
        };

        if let Some(prev_idx) = prev {
            if let Some(entry) = self.entries[prev_idx].as_mut() {
                entry.next = next;
            }
        } else {
            self.head = next;
        }

        if let Some(next_idx) = next {
            if let Some(entry) = self.entries[next_idx].as_mut() {
                entry.prev = prev;
            }
        } else {
            self.tail = prev;
        }
    }

    fn move_to_front(&mut self, idx: usize) {
        if self.head == Some(idx) {
            return;
        }

        self.remove_from_list(idx);
        self.add_to_front(idx);
    }

    fn evict_lru(&mut self) -> Option<V> {
        let tail_idx = self.tail?;

        self.remove_from_list(tail_idx);

        let entry = self.entries[tail_idx].take();
        self.free_list.push(tail_idx);
        self.size -= 1;

        // Remove from map (need to find key)
        // This is O(n) - in production, store key in entry
        self.map.retain(|_, &mut v| v != tail_idx);

        entry.map(|e| e.value)
    }
}

/// Cache statistics.
#[derive(Debug, Clone)]
pub struct CacheStats {
    pub size: usize,
    pub capacity: usize,
    pub hits: u64,
    pub misses: u64,
}

impl CacheStats {
    /// Hit rate as percentage.
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            self.hits as f64 / total as f64 * 100.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_put_get() {
        let cache = LruCache::new(10);

        cache.put("a", 1);
        cache.put("b", 2);
        cache.put("c", 3);

        assert_eq!(cache.get(&"a"), Some(1));
        assert_eq!(cache.get(&"b"), Some(2));
        assert_eq!(cache.get(&"c"), Some(3));
        assert_eq!(cache.get(&"d"), None);
    }

    #[test]
    fn test_lru_eviction() {
        let cache = LruCache::new(3);

        cache.put("a", 1);
        cache.put("b", 2);
        cache.put("c", 3);

        // Cache is full, inserting d should evict a (LRU)
        cache.put("d", 4);

        assert_eq!(cache.get(&"a"), None); // Evicted
        assert_eq!(cache.get(&"b"), Some(2));
        assert_eq!(cache.get(&"c"), Some(3));
        assert_eq!(cache.get(&"d"), Some(4));
    }

    #[test]
    fn test_access_updates_lru() {
        let cache = LruCache::new(3);

        cache.put("a", 1);
        cache.put("b", 2);
        cache.put("c", 3);

        // Access a to make it recently used
        cache.get(&"a");

        // Insert d - should evict b (now LRU)
        cache.put("d", 4);

        assert_eq!(cache.get(&"a"), Some(1)); // Still present
        assert_eq!(cache.get(&"b"), None); // Evicted
        assert_eq!(cache.get(&"c"), Some(3));
        assert_eq!(cache.get(&"d"), Some(4));
    }

    #[test]
    fn test_update_existing() {
        let cache = LruCache::new(10);

        cache.put("a", 1);
        let old = cache.put("a", 2);

        assert_eq!(old, Some(1));
        assert_eq!(cache.get(&"a"), Some(2));
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn test_remove() {
        let cache = LruCache::new(10);

        cache.put("a", 1);
        cache.put("b", 2);

        let removed = cache.remove(&"a");

        assert_eq!(removed, Some(1));
        assert_eq!(cache.get(&"a"), None);
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn test_stats() {
        let cache = LruCache::new(10);

        cache.put("a", 1);
        cache.get(&"a"); // Hit
        cache.get(&"a"); // Hit
        cache.get(&"b"); // Miss

        let stats = cache.stats();
        assert_eq!(stats.hits, 2);
        assert_eq!(stats.misses, 1);
        assert!(stats.hit_rate() > 66.0);
    }

    #[test]
    fn test_clear() {
        let cache = LruCache::new(10);

        cache.put("a", 1);
        cache.put("b", 2);
        cache.clear();

        assert!(cache.is_empty());
        assert_eq!(cache.get(&"a"), None);
    }
}
