use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::time::{Duration, Instant};

pub struct EphemeralIpRateLimiter {
    salt: u64,
    windows: HashMap<u64, Vec<Instant>>,
    max_requests_per_window: usize,
    window_duration: Duration,
}

impl EphemeralIpRateLimiter {
    pub fn new(max_requests_per_window: usize, window_secs: u64) -> Self {
        // Ephemeral in-memory salt generated at process startup
        let salt = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0xCAFE_BABE_DEAD_BEEF);

        Self {
            salt,
            windows: HashMap::new(),
            max_requests_per_window,
            window_duration: Duration::from_secs(window_secs),
        }
    }

    fn hash_ip(&self, client_ip: &str) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.salt.hash(&mut hasher);
        client_ip.hash(&mut hasher);
        hasher.finish()
    }

    pub fn check_and_record(&mut self, client_ip: &str) -> bool {
        let now = Instant::now();
        let key = self.hash_ip(client_ip);

        // Periodically purge old entries
        if self.windows.len() > 1000 {
            self.windows.retain(|_, timestamps| {
                timestamps.retain(|t| now.duration_since(*t) < self.window_duration);
                !timestamps.is_empty()
            });
        }

        let timestamps = self.windows.entry(key).or_default();
        timestamps.retain(|t| now.duration_since(*t) < self.window_duration);

        if timestamps.len() >= self.max_requests_per_window {
            false
        } else {
            timestamps.push(now);
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_limiter_allows_up_to_limit() {
        let mut limiter = EphemeralIpRateLimiter::new(3, 60);
        assert!(limiter.check_and_record("192.0.2.1"));
        assert!(limiter.check_and_record("192.0.2.1"));
        assert!(limiter.check_and_record("192.0.2.1"));
        assert!(!limiter.check_and_record("192.0.2.1")); // 4th request rejected

        // Different IP is allowed
        assert!(limiter.check_and_record("198.51.100.2"));
    }
}
