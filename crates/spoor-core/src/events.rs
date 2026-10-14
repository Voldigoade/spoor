use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", content = "data", rename_all = "snake_case")]
pub enum SpoorEvent {
    LookupStarted {
        target: String,
        mode: String,
    },
    TargetResolved {
        target: String,
        target_type: String,
    },
    ProviderStarted {
        provider: String,
        operation: String,
    },
    ProviderCompleted {
        provider: String,
        details: String,
    },
    ProviderWarning {
        provider: String,
        message: String,
    },
    ScanProgress {
        current: usize,
        total: usize,
        message: String,
    },
    AnalysisCompleted {
        duration_ms: u64,
    },
    LookupFailed {
        error: String,
    },
}

impl SpoorEvent {
    pub fn event_type(&self) -> &'static str {
        match self {
            SpoorEvent::LookupStarted { .. } => "lookup.started",
            SpoorEvent::TargetResolved { .. } => "target.resolved",
            SpoorEvent::ProviderStarted { .. } => "provider.started",
            SpoorEvent::ProviderCompleted { .. } => "provider.completed",
            SpoorEvent::ProviderWarning { .. } => "provider.warning",
            SpoorEvent::ScanProgress { .. } => "scan.progress",
            SpoorEvent::AnalysisCompleted { .. } => "analysis.completed",
            SpoorEvent::LookupFailed { .. } => "lookup.failed",
        }
    }
}
