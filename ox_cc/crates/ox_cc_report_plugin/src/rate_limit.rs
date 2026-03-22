/// Per-client rate limiter using a sliding window backed by an LRU cache.
///
/// Each entry tracks the start of the current 60-second window and the count
/// of reports received within it. Once the window expires (60 s from start),
/// the counter resets. The LRU capacity bounds memory: when full, the least
/// recently used client entry is evicted, effectively resetting their counter.
///
/// The struct is `Send + Sync` (interior mutability via `Mutex`) so it can be
/// stored in shared plugin state.
use std::num::NonZeroUsize;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use lru::LruCache;

const WINDOW: Duration = Duration::from_secs(60);
/// Maximum number of distinct clients tracked before LRU eviction.
const DEFAULT_CAPACITY: usize = 10_000;

struct Entry {
    window_start: Instant,
    count: u32,
}

pub struct RateLimiter {
    cache: Mutex<LruCache<String, Entry>>,
    limit: u32,
}

impl RateLimiter {
    pub fn new(limit: u32) -> Self {
        let cap = NonZeroUsize::new(DEFAULT_CAPACITY).unwrap();
        Self {
            cache: Mutex::new(LruCache::new(cap)),
            limit,
        }
    }

    /// Returns `true` if the request is within the allowed rate, `false` if it
    /// should be rejected (429). Increments the counter on allow.
    pub fn check(&self, client_id: &str) -> bool {
        let now = Instant::now();
        let mut cache = self.cache.lock().unwrap();
        match cache.get_mut(client_id) {
            Some(entry) if now.duration_since(entry.window_start) < WINDOW => {
                if entry.count >= self.limit {
                    return false;
                }
                entry.count += 1;
                true
            }
            _ => {
                // Window expired or first request — start a fresh window
                cache.put(client_id.to_string(), Entry { window_start: now, count: 1 });
                true
            }
        }
    }
}
