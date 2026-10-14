pub mod client;
pub mod engine;
pub mod github;
pub mod gitleak;
pub mod hygiene;

pub use client::{HttpClient, RateLimitState, DEFAULT_USER_AGENT, MAX_RESPONSE_BYTES};
pub use engine::AnalysisEngine;
pub use github::models as gh_models;
pub use github::GitHubProvider;
pub use gitleak::models as gl_models;
pub use gitleak::{GitLeakBudget, GitLeakClient, GitLeakOutcome};
pub use hygiene::detect_hygiene_from_paths;

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
