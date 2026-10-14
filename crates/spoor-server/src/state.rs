use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use spoor_core::{AnalysisMode, Report, SpoorEvent, Target};
use spoor_providers::AnalysisEngine;
use tokio::sync::{broadcast, Mutex, RwLock, Semaphore};
use uuid::Uuid;

use crate::limiter::EphemeralIpRateLimiter;

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

#[derive(Clone)]
pub struct AppState {
    pub engine: Arc<AnalysisEngine>,
    pub lookups: Arc<RwLock<HashMap<Uuid, LookupEntry>>>,
    pub limiter: Arc<Mutex<EphemeralIpRateLimiter>>,
    pub concurrency_semaphore: Arc<Semaphore>,
}

impl AppState {
    pub fn new(engine: Arc<AnalysisEngine>) -> Self {
        Self {
            engine,
            lookups: Arc::new(RwLock::new(HashMap::new())),
            limiter: Arc::new(Mutex::new(EphemeralIpRateLimiter::new(30, 60))),
            concurrency_semaphore: Arc::new(Semaphore::new(10)),
        }
    }
}
