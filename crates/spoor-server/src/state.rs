use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use spoor_core::{AnalysisMode, Report, SpoorEvent, Target};
use spoor_providers::AnalysisEngine;
use tokio::sync::{broadcast, Mutex, RwLock, Semaphore};
use uuid::Uuid;

use crate::limiter::EphemeralIpRateLimiter;
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
    pub status: LookupStatus,
    pub created_at: DateTime<Utc>,
    pub report: Option<Report>,
    pub error: Option<String>,
    pub event_tx: broadcast::Sender<SpoorEvent>,
}

/// Operational note on concurrency & clustering:
///
/// Spoor v0.1 server maintains active lookup states, SSE broadcast channels, and
/// ephemeral IP rate limits in process-local memory.
///
/// OPERATIONAL REQUIREMENT:
/// Spoor v0.1 server must run as a single application replica unless a shared lookup/job
/// state implementation is introduced. Horizontal scaling behind a multi-node round-robin
/// load balancer will cause partitioned SSE event delivery and disjoint rate-limiting.
#[derive(Clone)]
pub struct AppState {
    pub engine: Arc<AnalysisEngine>,
    pub lookups: Arc<RwLock<HashMap<Uuid, LookupEntry>>>,
    pub limiter: Arc<Mutex<EphemeralIpRateLimiter>>,
    pub concurrency_semaphore: Arc<Semaphore>,
    pub proxy_config: Arc<TrustedProxyConfig>,
}

impl AppState {
    /// Initialize application state, reading `SPOOR_TRUSTED_PROXIES` from the environment if set.
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

        Self::with_proxy_config(engine, proxy_config)
    }

    /// Initialize application state with an explicit trusted proxy configuration.
    pub fn with_proxy_config(
        engine: Arc<AnalysisEngine>,
        proxy_config: TrustedProxyConfig,
    ) -> Self {
        Self::with_options(engine, proxy_config, 30, 60)
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
            limiter: Arc::new(Mutex::new(EphemeralIpRateLimiter::new(
                max_requests_per_window,
                window_secs,
            ))),
            concurrency_semaphore: Arc::new(Semaphore::new(10)),
            proxy_config: Arc::new(proxy_config),
        }
    }
}
