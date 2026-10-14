use axum::http::HeaderMap;
use ipnet::IpNet;
use std::net::IpAddr;

/// Trusted reverse proxy configuration for secure client IP extraction.
///
/// Under the default configuration, no proxies are trusted (`trusted_subnets` is empty).
/// In this mode, the server strictly uses the real socket peer IP and ignores all
/// forwarded IP headers (`X-Forwarded-For`).
///
/// When explicitly configured with trusted proxy IPs or CIDR subnets (e.g. `127.0.0.1`,
/// `10.0.0.0/8`, Cloudflare IP ranges), the server verifies that the direct socket peer
/// is in the trusted set before reading `X-Forwarded-For`. If trusted, it walks the
/// forwarded chain from right to left (reverse order) and selects the first untrusted
/// upstream IP. This guarantees that direct untrusted clients cannot spoof their identity
/// by injecting arbitrary `X-Forwarded-For` headers.
#[derive(Debug, Clone, Default)]
pub struct TrustedProxyConfig {
    trusted_subnets: Vec<IpNet>,
}

impl TrustedProxyConfig {
    /// Create a new configuration with the provided subnets.
    pub fn new(trusted_subnets: Vec<IpNet>) -> Self {
        Self { trusted_subnets }
    }

    /// Parse a comma-separated list of IP addresses and CIDR subnets.
    ///
    /// Accepts IPv4/IPv6 addresses (e.g. `127.0.0.1`, `::1`) and
    /// CIDR notations (e.g. `10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`).
    pub fn parse(s: &str) -> Result<Self, String> {
        let mut subnets = Vec::new();
        for raw in s.split(',') {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                continue;
            }

            if let Ok(net) = trimmed.parse::<IpNet>() {
                subnets.push(net);
            } else if let Ok(addr) = trimmed.parse::<IpAddr>() {
                subnets.push(IpNet::from(addr));
            } else {
                return Err(format!("Invalid trusted proxy IP or CIDR: '{trimmed}'"));
            }
        }
        Ok(Self {
            trusted_subnets: subnets,
        })
    }

    /// Returns true if trusted proxies are configured.
    pub fn is_enabled(&self) -> bool {
        !self.trusted_subnets.is_empty()
    }

    /// Returns true if the given IP address falls within any configured trusted subnet.
    pub fn is_trusted(&self, ip: &IpAddr) -> bool {
        self.trusted_subnets.iter().any(|net| net.contains(ip))
    }

    /// Resolves the authentic client IP address for rate limiting and auditing.
    ///
    /// 1. If trusted proxy mode is disabled, strictly returns `peer_ip`.
    /// 2. If the connecting `peer_ip` is NOT in the trusted proxy set, strictly returns `peer_ip`.
    ///    Any forwarded headers provided by the untrusted peer are discarded.
    /// 3. If `peer_ip` IS trusted, parses `X-Forwarded-For` from right to left (nearest proxy backwards).
    ///    The first untrusted IP encountered is returned as the client IP.
    pub fn resolve_client_ip(&self, peer_ip: IpAddr, headers: &HeaderMap) -> IpAddr {
        // If not configured, or if the direct socket connection is NOT from a trusted proxy,
        // use socket peer IP exclusively.
        if !self.is_enabled() || !self.is_trusted(&peer_ip) {
            return peer_ip;
        }

        // Socket peer is trusted. Traverse X-Forwarded-For backwards.
        if let Some(xff) = headers.get("x-forwarded-for").and_then(|h| h.to_str().ok()) {
            let entries: Vec<&str> = xff
                .split(',')
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .collect();

            for entry in entries.into_iter().rev() {
                if let Ok(ip) = entry.parse::<IpAddr>() {
                    // If this IP in the chain is another trusted proxy intermediary,
                    // continue walking backward. The first non-trusted IP is the client.
                    if !self.is_trusted(&ip) {
                        return ip;
                    }
                }
            }
        }

        // Fallback to peer IP if header is absent or contains only trusted proxy addresses
        peer_ip
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;
    use std::str::FromStr;

    #[test]
    fn default_config_is_disabled() {
        let config = TrustedProxyConfig::default();
        assert!(!config.is_enabled());

        let peer: IpAddr = "198.51.100.1".parse().unwrap();
        assert!(!config.is_trusted(&peer));

        // Direct client sends spoofed X-Forwarded-For
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", HeaderValue::from_static("203.0.113.195"));

        // Must ignore spoofed header and return real socket peer
        let resolved = config.resolve_client_ip(peer, &headers);
        assert_eq!(resolved, peer);
    }

    #[test]
    fn parse_valid_ips_and_cidrs() {
        let config =
            TrustedProxyConfig::parse("127.0.0.1, 10.0.0.0/8, ::1, 192.168.1.0/24").unwrap();
        assert!(config.is_enabled());

        assert!(config.is_trusted(&"127.0.0.1".parse().unwrap()));
        assert!(config.is_trusted(&"10.254.1.2".parse().unwrap()));
        assert!(config.is_trusted(&"::1".parse().unwrap()));
        assert!(config.is_trusted(&"192.168.1.50".parse().unwrap()));

        assert!(!config.is_trusted(&"192.168.2.1".parse().unwrap()));
        assert!(!config.is_trusted(&"203.0.113.5".parse().unwrap()));
    }

    #[test]
    fn parse_invalid_fails_cleanly() {
        assert!(TrustedProxyConfig::parse("127.0.0.1, not-an-ip").is_err());
        assert!(TrustedProxyConfig::parse("10.0.0.0/999").is_err());
    }

    #[test]
    fn untrusted_peer_cannot_spoof_with_trusted_proxies_configured() {
        // Trusted proxy configured at 10.0.0.1
        let config = TrustedProxyConfig::parse("10.0.0.1").unwrap();

        // Direct attacker connects from 198.51.100.5 (not 10.0.0.1)
        let attacker_peer: IpAddr = "198.51.100.5".parse().unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", HeaderValue::from_static("1.1.1.1"));

        // Header MUST be ignored because the peer socket is untrusted
        let resolved = config.resolve_client_ip(attacker_peer, &headers);
        assert_eq!(resolved, attacker_peer);
    }

    #[test]
    fn trusted_peer_resolves_client_ip() {
        let config = TrustedProxyConfig::parse("10.0.0.1").unwrap();
        let proxy_peer: IpAddr = "10.0.0.1".parse().unwrap();

        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", HeaderValue::from_static("203.0.113.42"));

        let resolved = config.resolve_client_ip(proxy_peer, &headers);
        assert_eq!(resolved, IpAddr::from_str("203.0.113.42").unwrap());
    }

    #[test]
    fn trusted_peer_ignores_client_injected_spoofed_headers() {
        // Proxy is at 10.0.0.1. Attacker at 198.51.100.7 sent spoofed "8.8.8.8" to proxy.
        // Proxy appended 198.51.100.7 to X-Forwarded-For: "8.8.8.8, 198.51.100.7".
        let config = TrustedProxyConfig::parse("10.0.0.1").unwrap();
        let proxy_peer: IpAddr = "10.0.0.1".parse().unwrap();

        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("8.8.8.8, 198.51.100.7"),
        );

        // Reverse walk stops at the first untrusted IP from the right (198.51.100.7)
        let resolved = config.resolve_client_ip(proxy_peer, &headers);
        assert_eq!(resolved, IpAddr::from_str("198.51.100.7").unwrap());
    }

    #[test]
    fn multi_tier_trusted_proxies_walks_backward() {
        // Multi-tier: Cloudflare (173.245.48.0/20) -> Local Nginx (10.0.0.1) -> Spoor
        let config = TrustedProxyConfig::parse("10.0.0.1, 173.245.48.0/20").unwrap();
        let nginx_peer: IpAddr = "10.0.0.1".parse().unwrap();

        let mut headers = HeaderMap::new();
        // Client (203.0.113.99) -> Cloudflare (173.245.48.5) -> Nginx (10.0.0.1)
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("203.0.113.99, 173.245.48.5"),
        );

        let resolved = config.resolve_client_ip(nginx_peer, &headers);
        assert_eq!(resolved, IpAddr::from_str("203.0.113.99").unwrap());
    }
}
