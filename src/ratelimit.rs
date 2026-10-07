use std::{
    collections::HashMap,
    net::IpAddr,
    sync::Mutex,
    time::{Duration, Instant},
};

/// A tiny in-memory sliding-window rate limiter, keyed by `ip:action`.
///
/// ponytail: single process, single HashMap. Fine for one box. Swap for Redis
/// or a DB table if we ever run more than one replica.
pub struct RateLimiter {
    hits: Mutex<HashMap<String, Vec<Instant>>>,
}

impl RateLimiter {
    pub fn new() -> Self {
        Self {
            hits: Mutex::new(HashMap::new()),
        }
    }

    /// `true` if the call is allowed, `false` if the window is full.
    pub fn check(&self, key: &str, max: u32, window: Duration) -> bool {
        let now = Instant::now();
        let mut hits = self.hits.lock().unwrap();
        let entry = hits.entry(key.to_string()).or_default();
        entry.retain(|t| now.duration_since(*t) < window);
        if entry.len() as u32 >= max {
            return false;
        }
        entry.push(now);
        // Occasionally prune empty keys so the map can't grow unbounded.
        if hits.len() > 10_000 {
            hits.retain(|_, v| !v.is_empty());
        }
        true
    }
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

pub fn key(ip: IpAddr, action: &str) -> String {
    format!("{ip}:{action}")
}

/// Same idea, for limits keyed by something other than IP (e.g. a user id).
pub fn key_str(id: &str, action: &str) -> String {
    format!("{id}:{action}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_after_max() {
        let rl = RateLimiter::new();
        let w = Duration::from_secs(60);
        for _ in 0..5 {
            assert!(rl.check("k", 5, w));
        }
        assert!(!rl.check("k", 5, w));
        assert!(rl.check("other", 5, w));
    }
}
