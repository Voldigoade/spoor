use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use spoor_core::{AnalysisMode, Report, SpoorEvent, Target};
use spoor_providers::AnalysisEngine;
use tokio::sync::{broadcast, Mutex, RwLock, Semaphore};
use uuid::Uuid;

use crate::limiter::ServerRateLimiter;
use crate::proxy::TrustedProxyConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LookupStatus {
    Pending,
    Running,
    Completed,
    Failed,
}

pub struct LookupEntry {
    pub id: Uuid,
    pub target: Target,
    pub mode: AnalysisMode,
    pub enrich: bool,
    pub status: LookupStatus,
    pub created_at: DateTime<Utc>,
    pub report: Option<Report>,
    pub error: Option<String>,
    pub event_tx: broadcast::Sender<SpoorEvent>,
}

/// Lookup state, SSE channels and rate limits are process-local.
/// Run one replica until shared job state is implemented; otherwise requests
/// can reach a replica that does not own the lookup or its event stream.
#[derive(Clone)]
pub struct AppState {
    pub engine: Arc<AnalysisEngine>,
    pub lookups: Arc<RwLock<HashMap<Uuid, LookupEntry>>>,
    pub limiter: Arc<Mutex<ServerRateLimiter>>,
    pub concurrency_semaphore: Arc<Semaphore>,
    pub proxy_config: Arc<TrustedProxyConfig>,
    pub http_deep_enabled: bool,
    pub retention_duration: Duration,
}

impl AppState {
    /// Initialize application state, reading environment configuration:
    /// - `SPOOR_TRUSTED_PROXIES`
    /// - `SPOOR_HTTP_DEEP_ENABLED` (default: false)
    /// - `SPOOR_MAX_CONCURRENT_LOOKUPS` (default: 4)
    /// - `SPOOR_LOOKUP_RETENTION_SECS` (default: 3600)
    /// - `SPOOR_RATE_LIMIT_PER_IP` (default: 10)
    /// - `SPOOR_RATE_LIMIT_WINDOW_SECS` (default: 3600)
    /// - `SPOOR_GLOBAL_RATE_LIMIT` (default: 100)
    /// - `SPOOR_GLOBAL_RATE_LIMIT_WINDOW_SECS` (default: 3600)
    pub fn new(engine: Arc<AnalysisEngine>) -> Self {
        let proxy_config = std::env::var("SPOOR_TRUSTED_PROXIES")
            .ok()
            .and_then(|raw| match TrustedProxyConfig::parse(&raw) {
                Ok(cfg) => Some(cfg),
                Err(e) => {
                    tracing::warn!("Failed to parse SPOOR_TRUSTED_PROXIES: {e}");
                    None
                }
            })
            .unwrap_or_default();

        let http_deep_enabled = std::env::var("SPOOR_HTTP_DEEP_ENABLED")
            .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
            .unwrap_or(false);

        let max_concurrency = std::env::var("SPOOR_MAX_CONCURRENT_LOOKUPS")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(4);

        let retention_secs = std::env::var("SPOOR_LOOKUP_RETENTION_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(3600);

        Self {
            engine,
            lookups: Arc::new(RwLock::new(HashMap::new())),
            limiter: Arc::new(Mutex::new(ServerRateLimiter::from_env())),
            concurrency_semaphore: Arc::new(Semaphore::new(max_concurrency)),
            proxy_config: Arc::new(proxy_config),
            http_deep_enabled,
            retention_duration: Duration::from_secs(retention_secs),
        }
    }

    /// Initialize application state with an explicit trusted proxy configuration.
    pub fn with_proxy_config(
        engine: Arc<AnalysisEngine>,
        proxy_config: TrustedProxyConfig,
    ) -> Self {
        let mut state = Self::new(engine);
        state.proxy_config = Arc::new(proxy_config);
        state
    }

    /// Initialize application state with explicit proxy configuration and rate limiting options.
    pub fn with_options(
        engine: Arc<AnalysisEngine>,
        proxy_config: TrustedProxyConfig,
        max_requests_per_window: usize,
        window_secs: u64,
    ) -> Self {
        Self {
            engine,
            lookups: Arc::new(RwLock::new(HashMap::new())),
            limiter: Arc::new(Mutex::new(ServerRateLimiter::new(
                max_requests_per_window,
                window_secs,
                max_requests_per_window * 10,
                window_secs,
            ))),
            concurrency_semaphore: Arc::new(Semaphore::new(4)),
            proxy_config: Arc::new(proxy_config),
            http_deep_enabled: false,
            retention_duration: Duration::from_secs(3600),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ServerOptions {
    pub proxy_config: TrustedProxyConfig,
    pub ip_limit: usize,
    pub ip_window_secs: u64,
    pub global_limit: usize,
    pub global_window_secs: u64,
    pub http_deep_enabled: bool,
    pub max_concurrency: usize,
    pub retention_secs: u64,
}

impl Default for ServerOptions {
    fn default() -> Self {
        Self {
            proxy_config: TrustedProxyConfig::default(),
            ip_limit: 10,
            ip_window_secs: 3600,
            global_limit: 100,
            global_window_secs: 3600,
            http_deep_enabled: false,
            max_concurrency: 4,
            retention_secs: 3600,
        }
    }
}

impl AppState {
    /// Initialize application state with explicit ServerOptions.
    pub fn with_server_options(engine: Arc<AnalysisEngine>, options: ServerOptions) -> Self {
        Self {
            engine,
            lookups: Arc::new(RwLock::new(HashMap::new())),
            limiter: Arc::new(Mutex::new(ServerRateLimiter::new(
                options.ip_limit,
                options.ip_window_secs,
                options.global_limit,
                options.global_window_secs,
            ))),
            concurrency_semaphore: Arc::new(Semaphore::new(options.max_concurrency)),
            proxy_config: Arc::new(options.proxy_config),
            http_deep_enabled: options.http_deep_enabled,
            retention_duration: Duration::from_secs(options.retention_secs),
        }
    }

    /// Prune completed and failed lookups that exceed `retention_duration`.
    /// Age is measured from creation; pending and running lookups are retained.
    pub async fn prune_expired_lookups(&self) -> usize {
        let now = Utc::now();
        let retention = chrono::Duration::from_std(self.retention_duration)
            .unwrap_or(chrono::Duration::hours(1));
        let mut map = self.lookups.write().await;
        let mut evicted = 0;
        map.retain(|_id, entry| {
            if entry.status == LookupStatus::Pending || entry.status == LookupStatus::Running {
                return true;
            }
            let age = now.signed_duration_since(entry.created_at);
            if age > retention {
                evicted += 1;
                false
            } else {
                true
            }
        });
        evicted
    }
}
