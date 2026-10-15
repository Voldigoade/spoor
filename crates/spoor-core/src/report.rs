use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::classification::Classification;
use crate::email::EmailType;
use crate::provenance::Provenance;
use crate::target::{AnalysisMode, Target};

pub const SCHEMA_VERSION: &str = "1.2";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeneratorInfo {
    pub name: String,
    pub version: String,
    pub author: String,
    pub repository: String,
}

impl Default for GeneratorInfo {
    fn default() -> Self {
        Self {
            name: "Spoor".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            author: "Voldigoade".to_string(),
            repository: "https://github.com/Voldigoade/spoor".to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyType {
    Gpg,
    Ssh,
}

impl std::fmt::Display for KeyType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KeyType::Gpg => write!(f, "GPG"),
            KeyType::Ssh => write!(f, "SSH"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyRecord {
    pub key_type: KeyType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
    pub raw_preview: String,
    pub emails: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub can_sign: Option<bool>,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdentityRecord {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub email: String,
    pub email_type: EmailType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_seen: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_seen: Option<DateTime<Utc>>,
    pub commits_observed: usize,
    pub repositories: Vec<String>,
    pub sources: Vec<String>,
    pub is_verified_signer: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InferenceRecord {
    pub topic: String,
    pub inference: String,
    pub confidence: String,
    pub basis: String,
    pub classification: Classification,
    pub limitation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RepoHygiene {
    pub has_readme: bool,
    pub has_license: bool,
    pub has_security_policy: bool,
    pub has_contributing: bool,
    pub has_codeowners: bool,
    pub has_actions_ci: bool,
    pub has_dependabot: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RepoReport {
    pub name: String,
    pub full_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub is_fork: bool,
    pub is_archived: bool,
    pub default_branch: String,
    pub stars: u64,
    pub forks: u64,
    pub open_issues: u64,
    pub size_kb: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_language: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub languages: Option<HashMap<String, u64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pushed_at: Option<DateTime<Utc>>,
    pub topics: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub license_spdx: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hygiene: Option<RepoHygiene>,
    pub html_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileReport {
    pub login: String,
    pub id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bio: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub company: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blog_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub twitter_username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub public_email: Option<String>,
    pub avatar_url: String,
    pub html_url: String,
    pub public_repos: u64,
    pub public_gists: u64,
    pub followers: u64,
    pub following: u64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub account_age_days: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityEventSummary {
    pub event_type: String,
    pub repository: String,
    pub created_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ActivityReport {
    pub window_note: String,
    pub total_events_observed: usize,
    pub push_events: usize,
    pub pull_request_events: usize,
    pub issue_events: usize,
    pub release_events: usize,
    pub fork_events: usize,
    pub other_events: usize,
    pub active_repositories: Vec<String>,
    pub recent_actions: Vec<ActivityEventSummary>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderHealth {
    Ok,
    Cached,
    Partial,
    RateLimited,
    Unavailable,
    Skipped,
}

impl std::fmt::Display for ProviderHealth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProviderHealth::Ok => write!(f, "OK"),
            ProviderHealth::Cached => write!(f, "CACHED"),
            ProviderHealth::Partial => write!(f, "PARTIAL"),
            ProviderHealth::RateLimited => write!(f, "RATE_LIMITED"),
            ProviderHealth::Unavailable => write!(f, "UNAVAILABLE"),
            ProviderHealth::Skipped => write!(f, "SKIPPED"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderStatus {
    pub status: ProviderHealth,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub items_retrieved: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate_limit_remaining: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate_limit_reset: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after_seconds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommitMatchRole {
    Author,
    Committer,
    Both,
}

impl std::fmt::Display for CommitMatchRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CommitMatchRole::Author => write!(f, "Author"),
            CommitMatchRole::Committer => write!(f, "Committer"),
            CommitMatchRole::Both => write!(f, "Author & Committer"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct DiscoveryCoverage {
    pub owned_repositories_discovered: usize,
    pub external_repositories_discovered: usize,
    pub candidates_selected: usize,
    pub owned_selected: usize,
    pub external_selected: usize,
    pub candidates_skipped: usize,
    pub repository_scans_attempted: usize,
    pub repository_scans_succeeded: usize,
    pub repository_scans_failed: usize,
    pub global_author_commits_found: usize,
    pub global_committer_commits_found: usize,
    pub commit_search_windows_complete: usize,
    pub commit_search_windows_partial: usize,
    pub git_history_attempted: usize,
    pub git_history_succeeded: usize,
    pub git_history_failed: usize,
    pub git_history_timed_out: usize,
    pub graphql_years_fully_traversed: bool,
    pub stopped_due_to_budget: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CoverageRecord {
    pub commits_scanned: usize,
    pub repositories_scanned: usize,
    pub repositories_total: usize,
    #[serde(default)]
    pub candidate_repositories_found: usize,
    #[serde(default)]
    pub global_commits_found: usize,
    #[serde(default)]
    pub git_history_inspected: bool,
    pub is_partial: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discovery: Option<DiscoveryCoverage>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ReportSummary {
    pub display_name: String,
    pub target_type: String,
    pub account_created: Option<DateTime<Utc>>,
    pub total_public_repositories: usize,
    pub total_stars_accumulated: u64,
    pub total_forks_accumulated: u64,
    pub primary_languages: Vec<(String, u64)>,
    pub distinct_emails_observed: usize,
    pub public_keys_found: usize,
    pub fork_ratio: f64,
    pub archived_ratio: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub schema_version: String,
    pub generator: GeneratorInfo,
    pub target: Target,
    pub mode: AnalysisMode,
    pub generated_at: DateTime<Utc>,
    pub summary: ReportSummary,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<ProfileReport>,
    pub repositories: Vec<RepoReport>,
    pub activity: ActivityReport,
    pub identities: Vec<IdentityRecord>,
    pub keys: Vec<KeyRecord>,
    pub inferences: Vec<InferenceRecord>,
    pub provider_status: HashMap<String, ProviderStatus>,
    pub coverage: CoverageRecord,
    pub provenance: Vec<Provenance>,
    pub warnings: Vec<String>,
    pub limitations: Vec<String>,
}

impl Report {
    pub fn new(target: Target, mode: AnalysisMode) -> Self {
        Self {
            schema_version: SCHEMA_VERSION.to_string(),
            generator: GeneratorInfo::default(),
            target,
            mode,
            generated_at: Utc::now(),
            summary: ReportSummary::default(),
            profile: None,
            repositories: Vec::new(),
            activity: ActivityReport {
                window_note: "GitHub public events cover up to 300 events from the previous 30 days. Zero returned events do not establish inactivity."
                    .to_string(),
                ..Default::default()
            },
            identities: Vec::new(),
            keys: Vec::new(),
            inferences: Vec::new(),
            provider_status: HashMap::new(),
            coverage: CoverageRecord::default(),
            provenance: Vec::new(),
            warnings: Vec::new(),
            limitations: vec![
                "Findings strictly reflect publicly accessible GitHub metadata and public API endpoints.".to_string(),
                "Commit identities represent public author/committer headers in git metadata and do not assert current private contact information.".to_string(),
                "Timezone analysis represents algorithmic inferences from commit timestamps and must never be interpreted as physical residency or nationality.".to_string(),
                "Historical activity may be truncated by upstream provider retention policies and request budgets.".to_string(),
            ],
        }
    }

    /// Recomputes and verifies summary metrics from the canonical report data.
    pub fn recompute_summary(&mut self) {
        let total_stars: u64 = self.repositories.iter().map(|r| r.stars).sum();
        let total_forks: u64 = self.repositories.iter().map(|r| r.forks).sum();

        let total_repos = if let Some(p) = &self.profile {
            p.public_repos as usize
        } else {
            self.repositories.len()
        };

        let repos_count = self.repositories.len();
        let fork_count = self.repositories.iter().filter(|r| r.is_fork).count();
        let archived_count = self.repositories.iter().filter(|r| r.is_archived).count();

        let fork_ratio = if repos_count > 0 {
            fork_count as f64 / repos_count as f64
        } else {
            0.0
        };

        let archived_ratio = if repos_count > 0 {
            archived_count as f64 / repos_count as f64
        } else {
            0.0
        };

        let mut lang_counts: std::collections::BTreeMap<String, u64> =
            std::collections::BTreeMap::new();
        for r in &self.repositories {
            if let Some(lang) = &r.primary_language {
                *lang_counts.entry(lang.clone()).or_insert(0) += 1;
            }
        }
        let mut sorted_langs: Vec<(String, u64)> = lang_counts.into_iter().collect();
        sorted_langs.sort_by_key(|b| std::cmp::Reverse(b.1));

        // Distinct human / target emails (excluding automated platform bots)
        let distinct_emails = self
            .identities
            .iter()
            .filter(|id| id.email_type != EmailType::GitHubSystem)
            .count();

        self.summary.total_public_repositories = total_repos;
        self.summary.total_stars_accumulated = total_stars;
        self.summary.total_forks_accumulated = total_forks;
        self.summary.primary_languages = sorted_langs;
        self.summary.distinct_emails_observed = distinct_emails;
        self.summary.public_keys_found = self.keys.len();
        self.summary.fork_ratio = fork_ratio;
        self.summary.archived_ratio = archived_ratio;
    }
}
