pub mod cache;
pub mod classification;
pub mod email;
pub mod error;
pub mod events;
pub mod provenance;
pub mod report;
pub mod target;

pub use cache::{CacheStats, CacheStore, NoopCacheStore, SqliteCacheStore};
pub use classification::Classification;
pub use email::{classify_email, EmailType};
pub use error::SpoorError;
pub use events::SpoorEvent;
pub use provenance::Provenance;
pub use report::{
    ActivityEventSummary, ActivityReport, CoverageRecord, GeneratorInfo, IdentityRecord,
    InferenceRecord, KeyRecord, KeyType, ProfileReport, ProviderHealth, ProviderStatus,
    RepoHygiene, RepoReport, Report, ReportSummary, SCHEMA_VERSION,
};
pub use target::{AnalysisMode, AnalysisOptions, Target};

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
