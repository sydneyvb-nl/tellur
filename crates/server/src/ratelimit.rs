//! Minimal in-memory fixed-window rate limiter.
//!
//! Single-node only — sufficient for self-host. Distributed rate limiting
//! (shared across replicas) arrives with the Postgres backend in B5.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// A fixed-window limiter keyed by an arbitrary string (e.g. a member id).
pub struct RateLimiter {
    max: u32,
    window: Duration,
    state: Mutex<HashMap<String, (Instant, u32)>>,
}

impl RateLimiter {
    pub fn new(max: u32, window: Duration) -> Self {
        Self {
            max,
            window,
            state: Mutex::new(HashMap::new()),
        }
    }

    /// Record an attempt for `key`; returns `true` if it is within the limit.
    pub fn check(&self, key: &str) -> bool {
        let now = Instant::now();
        let mut map = self.state.lock().unwrap_or_else(|p| p.into_inner());
        let entry = map.entry(key.to_string()).or_insert((now, 0));
        if now.duration_since(entry.0) >= self.window {
            *entry = (now, 0);
        }
        if entry.1 >= self.max {
            return false;
        }
        entry.1 += 1;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_up_to_max_then_blocks() {
        let rl = RateLimiter::new(2, Duration::from_secs(60));
        assert!(rl.check("k"));
        assert!(rl.check("k"));
        assert!(!rl.check("k")); // third within window blocked
        assert!(rl.check("other")); // independent key
    }

    #[test]
    fn window_resets() {
        let rl = RateLimiter::new(1, Duration::from_millis(20));
        assert!(rl.check("k"));
        assert!(!rl.check("k"));
        std::thread::sleep(Duration::from_millis(30));
        assert!(rl.check("k"));
    }
}
