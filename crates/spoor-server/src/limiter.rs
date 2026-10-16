use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RateLimitResult {
    Allowed,
    Exceeded { retry_after_secs: u64 },
}

impl RateLimitResult {
    pub fn is_allowed(&self) -> bool {
        matches!(self, RateLimitResult::Allowed)
    }

    pub fn retry_after(&self) -> Option<u64> {
        match self {
            RateLimitResult::Allowed => None,
            RateLimitResult::Exceeded { retry_after_secs } => Some(*retry_after_secs),
        }
    }
}

/// Ephemeral in-memory rate limiter per client IP address.
/// Client IPs are salted and hashed with a process-startup secret so no raw
/// IP addresses are stored or logged.
pub struct EphemeralIpRateLimiter {
    salt: u64,
    windows: HashMap<u64, Vec<Instant>>,
    max_requests_per_window: usize,
    window_duration: Duration,
}

impl EphemeralIpRateLimiter {
    pub fn new(max_requests_per_window: usize, window_secs: u64) -> Self {
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

    pub fn check(&mut self, client_ip: &str) -> RateLimitResult {
        let now = Instant::now();
        let key = self.hash_ip(client_ip);

        if self.windows.len() > 1000 {
            self.windows.retain(|_, timestamps| {
                timestamps.retain(|t| now.duration_since(*t) < self.window_duration);
                !timestamps.is_empty()
            });
        }

        let timestamps = self.windows.entry(key).or_default();
        timestamps.retain(|t| now.duration_since(*t) < self.window_duration);

        if timestamps.len() >= self.max_requests_per_window {
            let earliest = timestamps.first().copied().unwrap_or(now);
            let elapsed = now.duration_since(earliest);
            let retry_after_secs = if elapsed < self.window_duration {
                (self.window_duration - elapsed).as_secs().max(1)
            } else {
                1
            };
            RateLimitResult::Exceeded { retry_after_secs }
        } else {
            RateLimitResult::Allowed
        }
    }

    pub fn record(&mut self, client_ip: &str) {
        let now = Instant::now();
        let key = self.hash_ip(client_ip);
        let timestamps = self.windows.entry(key).or_default();
        timestamps.push(now);
    }

    pub fn check_and_record(&mut self, client_ip: &str) -> RateLimitResult {
        let res = self.check(client_ip);
        if res.is_allowed() {
            self.record(client_ip);
        }
        res
    }
}

/// Process-wide global rate limiter protecting the public Spoor instance from distributed abuse.
pub struct GlobalRateLimiter {
    timestamps: Vec<Instant>,
    max_requests_per_window: usize,
    window_duration: Duration,
}

impl GlobalRateLimiter {
    pub fn new(max_requests_per_window: usize, window_secs: u64) -> Self {
        Self {
            timestamps: Vec::new(),
            max_requests_per_window,
            window_duration: Duration::from_secs(window_secs),
        }
    }

    pub fn check(&mut self) -> RateLimitResult {
        let now = Instant::now();
        self.timestamps
            .retain(|t| now.duration_since(*t) < self.window_duration);

        if self.timestamps.len() >= self.max_requests_per_window {
            let earliest = self.timestamps.first().copied().unwrap_or(now);
            let elapsed = now.duration_since(earliest);
            let retry_after_secs = if elapsed < self.window_duration {
                (self.window_duration - elapsed).as_secs().max(1)
            } else {
                1
            };
            RateLimitResult::Exceeded { retry_after_secs }
        } else {
            RateLimitResult::Allowed
        }
    }

    pub fn record(&mut self) {
        self.timestamps.push(Instant::now());
    }

    pub fn check_and_record(&mut self) -> RateLimitResult {
        let res = self.check();
        if res.is_allowed() {
            self.record();
        }
        res
    }
}

/// Combined server rate limiter evaluating both per-IP limits and global limits.
pub struct ServerRateLimiter {
    ip_limiter: EphemeralIpRateLimiter,
    global_limiter: GlobalRateLimiter,
}

impl ServerRateLimiter {
    pub fn new(
        ip_limit: usize,
        ip_window_secs: u64,
        global_limit: usize,
        global_window_secs: u64,
    ) -> Self {
        Self {
            ip_limiter: EphemeralIpRateLimiter::new(ip_limit, ip_window_secs),
            global_limiter: GlobalRateLimiter::new(global_limit, global_window_secs),
        }
    }

    /// Read configuration from environment variables with production defaults:
    /// - `SPOOR_RATE_LIMIT_PER_IP`: 10 lookups / hour
    /// - `SPOOR_RATE_LIMIT_WINDOW_SECS`: 3600 seconds
    /// - `SPOOR_GLOBAL_RATE_LIMIT`: 100 lookups / hour
    /// - `SPOOR_GLOBAL_RATE_LIMIT_WINDOW_SECS`: 3600 seconds
    pub fn from_env() -> Self {
        let ip_limit = std::env::var("SPOOR_RATE_LIMIT_PER_IP")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(10);
        let ip_window = std::env::var("SPOOR_RATE_LIMIT_WINDOW_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(3600);
        let global_limit = std::env::var("SPOOR_GLOBAL_RATE_LIMIT")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(100);
        let global_window = std::env::var("SPOOR_GLOBAL_RATE_LIMIT_WINDOW_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(3600);

        Self::new(ip_limit, ip_window, global_limit, global_window)
    }

    pub fn check_and_record(&mut self, client_ip: &str) -> RateLimitResult {
        let global_res = self.global_limiter.check();
        if !global_res.is_allowed() {
            return global_res;
        }

        let ip_res = self.ip_limiter.check(client_ip);
        if !ip_res.is_allowed() {
            return ip_res;
        }

        self.global_limiter.record();
        self.ip_limiter.record(client_ip);

        RateLimitResult::Allowed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ip_rate_limiter_allows_up_to_limit() {
        let mut limiter = EphemeralIpRateLimiter::new(3, 60);
        assert_eq!(
            limiter.check_and_record("192.0.2.1"),
            RateLimitResult::Allowed
        );
        assert_eq!(
            limiter.check_and_record("192.0.2.1"),
            RateLimitResult::Allowed
        );
        assert_eq!(
            limiter.check_and_record("192.0.2.1"),
            RateLimitResult::Allowed
        );

        // 4th request rejected
        let res = limiter.check_and_record("192.0.2.1");
        assert!(!res.is_allowed());
        assert!(res.retry_after().unwrap() <= 60);

        // Different IP is allowed
        assert_eq!(
            limiter.check_and_record("198.51.100.2"),
            RateLimitResult::Allowed
        );
    }

    #[test]
    fn global_rate_limiter_enforces_capacity() {
        let mut limiter = GlobalRateLimiter::new(2, 60);
        assert_eq!(limiter.check_and_record(), RateLimitResult::Allowed);
        assert_eq!(limiter.check_and_record(), RateLimitResult::Allowed);

        let res = limiter.check_and_record();
        assert!(!res.is_allowed());
        assert!(res.retry_after().is_some());
    }

    #[test]
    fn server_rate_limiter_checks_both() {
        let mut limiter = ServerRateLimiter::new(2, 60, 3, 60);
        assert!(limiter.check_and_record("1.1.1.1").is_allowed());
        assert!(limiter.check_and_record("1.1.1.1").is_allowed());
        // IP 1.1.1.1 exceeded
        assert!(!limiter.check_and_record("1.1.1.1").is_allowed());

        // IP 2.2.2.2 still has per-IP quota, but uses 3rd global slot
        assert!(limiter.check_and_record("2.2.2.2").is_allowed());

        // Now global quota (3) is exhausted
        assert!(!limiter.check_and_record("3.3.3.3").is_allowed());
    }
}
