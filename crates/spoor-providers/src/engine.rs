use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::Instant;

use chrono::{NaiveDate, Utc};
use spoor_core::{
    classify_email, is_github_platform_system, ActivityEventSummary, ActivityReport, AnalysisMode,
    AnalysisOptions, CacheStore, Classification, CommitMatchRole, CoverageRecord,
    DiscoveryCoverage, IdentityRecord, InferenceRecord, KeyRecord, KeyType, ProfileReport,
    Provenance, ProviderHealth, ProviderStatus, RelatedActorRecord, RepoHygiene, RepoReport,
    Report, ReportSummary, SpoorError, SpoorEvent, Target,
};
use tokio::sync::mpsc::Sender;

use crate::git::GitInspector;
use crate::github::models::{
    GhCommitActor, GhCommitDetail, GhCommitItem, GhSimpleUser, GhVerification,
};
use crate::github::GitHubProvider;
use crate::gitleak::{GitLeakBudget, GitLeakClient, GitLeakOutcome, GitLeakResponse};
use crate::hygiene::detect_hygiene_from_paths;

fn extract_repo_from_commit_url(url: &str) -> Option<String> {
    let stripped = url
        .trim()
        .strip_prefix("https://github.com/")
        .or_else(|| url.trim().strip_prefix("http://github.com/"))
        .or_else(|| url.trim().strip_prefix("github.com/"))?;
    let mut parts = stripped.split('/');
    let owner = parts.next()?;
    let repo = parts.next()?;
    if !owner.is_empty() && !repo.is_empty() {
        Some(format!("{owner}/{repo}"))
    } else {
        None
    }
}

fn is_target_noreply(email: &str, username: &str, user_id: Option<u64>) -> bool {
    let email = email.trim().to_lowercase();
    let username = username.trim().to_lowercase();
    if let Some(rest) = email.strip_suffix("@users.noreply.github.com") {
        if rest == username {
            return true;
        }
        if let Some((prefix, suffix)) = rest.split_once('+') {
            if suffix == username {
                if let Some(uid) = user_id {
                    return prefix == uid.to_string();
                }
            }
        }
    }
    false
}

fn is_target_actor(
    actor_email: &str,
    actor_user: Option<&GhSimpleUser>,
    target_username: &str,
    target_user_id: Option<u64>,
    target_anchors: &HashSet<String>,
) -> bool {
    let email_norm = actor_email.trim().to_lowercase();
    if email_norm.is_empty() || is_github_platform_system(&email_norm) {
        return false;
    }

    if target_anchors.contains(&email_norm) {
        return true;
    }

    if let Some(user) = actor_user {
        if user.login.eq_ignore_ascii_case(target_username) {
            if let Some(target_id) = target_user_id {
                if user.id == target_id {
                    return true;
                }
            } else {
                return true;
            }
        }
    }

    if is_target_noreply(&email_norm, target_username, target_user_id) {
        return true;
    }

    false
}

type ScannedRepoMeta = (Option<HashMap<String, u64>>, Option<RepoHygiene>);

#[derive(Debug, Clone)]
struct CandidateRepoEntry {
    owner: String,
    name: String,
    full_name: String,
    html_url: String,
    is_fork: bool,
    description: Option<String>,
    default_branch: Option<String>,
    sources: Vec<String>,
    commit_matches: usize,
}

#[derive(Debug, Clone)]
struct DiscoveredCommit {
    repo_full_name: String,
    item: GhCommitItem,
    role: CommitMatchRole,
}

#[derive(Debug, Clone)]
struct CanonicalCommitRecord {
    repo_full_name: String,
    sha: String,
    html_url: String,
    detail: GhCommitDetail,
    author_user: Option<GhSimpleUser>,
    committer_user: Option<GhSimpleUser>,
    search_role: Option<CommitMatchRole>,
    sources: HashSet<String>,
}

struct SearchSessionOutcome {
    commits: Vec<DiscoveredCommit>,
    author_commits_count: usize,
    committer_commits_count: usize,
    windows_complete: usize,
    windows_partial: usize,
    stopped_due_to_budget: bool,
    health: ProviderHealth,
    error_message: Option<String>,
}

pub struct AnalysisEngine {
    github: GitHubProvider,
    gitleak: GitLeakClient,
    gitleak_budget: GitLeakBudget,
    cache: Arc<dyn CacheStore>,
}

impl AnalysisEngine {
    pub fn new(github: GitHubProvider, gitleak: GitLeakClient, cache: Arc<dyn CacheStore>) -> Self {
        Self {
            github,
            gitleak,
            gitleak_budget: GitLeakBudget::from_env(),
            cache,
        }
    }

    pub fn with_budget(
        github: GitHubProvider,
        gitleak: GitLeakClient,
        gitleak_budget: GitLeakBudget,
        cache: Arc<dyn CacheStore>,
    ) -> Self {
        Self {
            github,
            gitleak,
            gitleak_budget,
            cache,
        }
    }

    async fn emit(tx: &Option<Sender<SpoorEvent>>, event: SpoorEvent) {
        if let Some(sender) = tx {
            let _ = sender.send(event).await;
        }
    }

    pub async fn run_analysis(
        &self,
        target: &Target,
        options: impl Into<AnalysisOptions>,
        event_tx: Option<Sender<SpoorEvent>>,
    ) -> Result<Report, SpoorError> {
        let options = options.into();
        let start_time = Instant::now();

        Self::emit(
            &event_tx,
            SpoorEvent::LookupStarted {
                target: target.display_name(),
                mode: options.mode.to_string(),
            },
        )
        .await;

        Self::emit(
            &event_tx,
            SpoorEvent::TargetResolved {
                target: target.display_name(),
                target_type: if target.is_user() {
                    "user"
                } else {
                    "repository"
                }
                .to_string(),
            },
        )
        .await;

        let cache_key = format!(
            "report:{}:{}:enrich={}",
            target.display_name(),
            options.mode,
            options.enrich
        );
        if let Ok(Some(cached_json)) = self.cache.get("spoor", &cache_key, "v2").await {
            if let Ok(mut cached_report) = serde_json::from_str::<Report>(&cached_json) {
                Self::emit(
                    &event_tx,
                    SpoorEvent::ProviderCompleted {
                        provider: "Cache".to_string(),
                        details: "Loaded fresh cached report".to_string(),
                    },
                )
                .await;
                Self::emit(
                    &event_tx,
                    SpoorEvent::AnalysisCompleted {
                        duration_ms: start_time.elapsed().as_millis() as u64,
                    },
                )
                .await;
                for status in cached_report.provider_status.values_mut() {
                    if status.status == ProviderHealth::Ok {
                        status.status = ProviderHealth::Cached;
                    }
                }
                return Ok(cached_report);
            }
        }

        let report = match target {
            Target::User { username } => self.analyze_user(username, options, &event_tx).await?,
            Target::Repo { owner, repo } => {
                self.analyze_repo(owner, repo, options.mode, &event_tx)
                    .await?
            }
        };

        let ttl_secs = match options.mode {
            AnalysisMode::Fast => 3600,
            AnalysisMode::Deep => 21600,
        };
        if let Ok(serialized) = serde_json::to_string(&report) {
            let _ = self
                .cache
                .set("spoor", &cache_key, "v2", &serialized, ttl_secs)
                .await;
        }

        Self::emit(
            &event_tx,
            SpoorEvent::AnalysisCompleted {
                duration_ms: start_time.elapsed().as_millis() as u64,
            },
        )
        .await;

        Ok(report)
    }

    async fn run_global_commit_search(
        &self,
        username: &str,
        created_at: chrono::DateTime<Utc>,
        mode: AnalysisMode,
    ) -> SearchSessionOutcome {
        let mut map: HashMap<(String, String), (GhCommitItem, CommitMatchRole)> = HashMap::new();
        let mut author_commits_count = 0usize;
        let mut committer_commits_count = 0usize;
        let mut windows_complete = 0usize;
        let mut windows_partial = 0usize;
        let mut stopped_due_to_budget = false;
        let mut first_error: Option<SpoorError> = None;

        let (author_max_requests, committer_max_requests) = match mode {
            AnalysisMode::Fast => (1, 1),
            AnalysisMode::Deep => (7, 7),
        };
        let mut author_requests_used = 0usize;
        let mut committer_requests_used = 0usize;

        let start_date = created_at.date_naive();
        let end_date = Utc::now().date_naive();

        if mode == AnalysisMode::Fast {
            if author_requests_used < author_max_requests {
                author_requests_used += 1;
                let query = format!("author:{username}");
                match self
                    .github
                    .search_commits(&query, 1, 100, Some("author-date"))
                    .await
                {
                    Ok(res) => {
                        let count = res.items.len();
                        author_commits_count += count;
                        if res.total_count <= 100 {
                            windows_complete += 1;
                        } else {
                            windows_partial += 1;
                        }
                        for item in res.items {
                            let repo_full = item.repository.full_name.clone();
                            let sha = item.sha.clone();
                            let key = (repo_full, sha.clone());
                            let commit_item = GhCommitItem {
                                sha,
                                html_url: item.html_url,
                                commit: item.commit,
                                author: item.author,
                                committer: item.committer,
                            };
                            map.insert(key, (commit_item, CommitMatchRole::Author));
                        }
                    }
                    Err(e) => {
                        windows_partial += 1;
                        if first_error.is_none() {
                            first_error = Some(e);
                        }
                    }
                }
            }
        } else {
            let mut queue: VecDeque<(NaiveDate, NaiveDate)> = VecDeque::new();
            queue.push_back((start_date, end_date));

            while let Some((w_start, w_end)) = queue.pop_front() {
                if author_requests_used >= author_max_requests {
                    windows_partial += 1 + queue.len();
                    stopped_due_to_budget = true;
                    queue.clear();
                    break;
                }

                let is_full_span = w_start == start_date && w_end == end_date;
                let query = if is_full_span {
                    format!("author:{username}")
                } else {
                    format!("author:{username} author-date:{w_start}..{w_end}")
                };

                author_requests_used += 1;
                match self
                    .github
                    .search_commits(&query, 1, 100, Some("author-date"))
                    .await
                {
                    Ok(res) => {
                        author_commits_count += res.items.len();
                        for item in res.items {
                            let repo_full = item.repository.full_name.clone();
                            let sha = item.sha.clone();
                            let key = (repo_full, sha.clone());
                            let commit_item = GhCommitItem {
                                sha,
                                html_url: item.html_url,
                                commit: item.commit,
                                author: item.author,
                                committer: item.committer,
                            };
                            map.insert(key, (commit_item, CommitMatchRole::Author));
                        }

                        if res.total_count <= 100 {
                            windows_complete += 1;
                        } else if res.total_count <= 300
                            && author_requests_used < author_max_requests
                        {
                            let total_pages = (res.total_count as usize).div_ceil(100);
                            let mut paginated_fully = true;
                            for p in 2..=total_pages {
                                if author_requests_used >= author_max_requests {
                                    paginated_fully = false;
                                    stopped_due_to_budget = true;
                                    break;
                                }
                                author_requests_used += 1;
                                match self
                                    .github
                                    .search_commits(&query, p, 100, Some("author-date"))
                                    .await
                                {
                                    Ok(page_res) => {
                                        author_commits_count += page_res.items.len();
                                        for item in page_res.items {
                                            let repo_full = item.repository.full_name.clone();
                                            let sha = item.sha.clone();
                                            let key = (repo_full, sha.clone());
                                            let commit_item = GhCommitItem {
                                                sha,
                                                html_url: item.html_url,
                                                commit: item.commit,
                                                author: item.author,
                                                committer: item.committer,
                                            };
                                            map.insert(key, (commit_item, CommitMatchRole::Author));
                                        }
                                    }
                                    Err(e) => {
                                        paginated_fully = false;
                                        if first_error.is_none() {
                                            first_error = Some(e);
                                        }
                                        break;
                                    }
                                }
                            }
                            if paginated_fully {
                                windows_complete += 1;
                            } else {
                                windows_partial += 1;
                            }
                        } else if w_start < w_end {
                            let span_days = (w_end - w_start).num_days();
                            if span_days >= 2 {
                                let mid = w_start + chrono::Duration::days(span_days / 2);
                                let next_day = mid + chrono::Duration::days(1);
                                queue.push_back((w_start, mid));
                                queue.push_back((next_day, w_end));
                            } else {
                                windows_partial += 1;
                            }
                        } else {
                            windows_partial += 1;
                        }
                    }
                    Err(e) => {
                        windows_partial += 1;
                        if first_error.is_none() {
                            first_error = Some(e);
                        }
                        break;
                    }
                }
            }
        }

        if mode == AnalysisMode::Fast {
            if committer_requests_used < committer_max_requests {
                committer_requests_used += 1;
                let query = format!("committer:{username}");
                match self
                    .github
                    .search_commits(&query, 1, 100, Some("committer-date"))
                    .await
                {
                    Ok(res) => {
                        let count = res.items.len();
                        committer_commits_count += count;
                        if res.total_count <= 100 {
                            windows_complete += 1;
                        } else {
                            windows_partial += 1;
                        }
                        for item in res.items {
                            let repo_full = item.repository.full_name.clone();
                            let sha = item.sha.clone();
                            let key = (repo_full, sha.clone());
                            let commit_item = GhCommitItem {
                                sha,
                                html_url: item.html_url,
                                commit: item.commit,
                                author: item.author,
                                committer: item.committer,
                            };
                            map.entry(key)
                                .and_modify(|existing| existing.1 = CommitMatchRole::Both)
                                .or_insert((commit_item, CommitMatchRole::Committer));
                        }
                    }
                    Err(e) => {
                        windows_partial += 1;
                        if first_error.is_none() {
                            first_error = Some(e);
                        }
                    }
                }
            }
        } else {
            let mut queue: VecDeque<(NaiveDate, NaiveDate)> = VecDeque::new();
            queue.push_back((start_date, end_date));

            while let Some((w_start, w_end)) = queue.pop_front() {
                if committer_requests_used >= committer_max_requests {
                    windows_partial += 1 + queue.len();
                    stopped_due_to_budget = true;
                    queue.clear();
                    break;
                }

                let is_full_span = w_start == start_date && w_end == end_date;
                let query = if is_full_span {
                    format!("committer:{username}")
                } else {
                    format!("committer:{username} committer-date:{w_start}..{w_end}")
                };

                committer_requests_used += 1;
                match self
                    .github
                    .search_commits(&query, 1, 100, Some("committer-date"))
                    .await
                {
                    Ok(res) => {
                        committer_commits_count += res.items.len();
                        for item in res.items {
                            let repo_full = item.repository.full_name.clone();
                            let sha = item.sha.clone();
                            let key = (repo_full, sha.clone());
                            let commit_item = GhCommitItem {
                                sha,
                                html_url: item.html_url,
                                commit: item.commit,
                                author: item.author,
                                committer: item.committer,
                            };
                            map.entry(key)
                                .and_modify(|existing| existing.1 = CommitMatchRole::Both)
                                .or_insert((commit_item, CommitMatchRole::Committer));
                        }

                        if res.total_count <= 100 {
                            windows_complete += 1;
                        } else if res.total_count <= 300
                            && committer_requests_used < committer_max_requests
                        {
                            let total_pages = (res.total_count as usize).div_ceil(100);
                            let mut paginated_fully = true;
                            for p in 2..=total_pages {
                                if committer_requests_used >= committer_max_requests {
                                    paginated_fully = false;
                                    stopped_due_to_budget = true;
                                    break;
                                }
                                committer_requests_used += 1;
                                match self
                                    .github
                                    .search_commits(&query, p, 100, Some("committer-date"))
                                    .await
                                {
                                    Ok(page_res) => {
                                        committer_commits_count += page_res.items.len();
                                        for item in page_res.items {
                                            let repo_full = item.repository.full_name.clone();
                                            let sha = item.sha.clone();
                                            let key = (repo_full, sha.clone());
                                            let commit_item = GhCommitItem {
                                                sha,
                                                html_url: item.html_url,
                                                commit: item.commit,
                                                author: item.author,
                                                committer: item.committer,
                                            };
                                            map.entry(key)
                                                .and_modify(|existing| {
                                                    existing.1 = CommitMatchRole::Both
                                                })
                                                .or_insert((
                                                    commit_item,
                                                    CommitMatchRole::Committer,
                                                ));
                                        }
                                    }
                                    Err(e) => {
                                        paginated_fully = false;
                                        if first_error.is_none() {
                                            first_error = Some(e);
                                        }
                                        break;
                                    }
                                }
                            }
                            if paginated_fully {
                                windows_complete += 1;
                            } else {
                                windows_partial += 1;
                            }
                        } else if w_start < w_end {
                            let span_days = (w_end - w_start).num_days();
                            if span_days >= 2 {
                                let mid = w_start + chrono::Duration::days(span_days / 2);
                                let next_day = mid + chrono::Duration::days(1);
                                queue.push_back((w_start, mid));
                                queue.push_back((next_day, w_end));
                            } else {
                                windows_partial += 1;
                            }
                        } else {
                            windows_partial += 1;
                        }
                    }
                    Err(e) => {
                        windows_partial += 1;
                        if first_error.is_none() {
                            first_error = Some(e);
                        }
                        break;
                    }
                }
            }
        }

        let _ = (author_requests_used, committer_requests_used);
        let total_commits = map.len();
        let (health, error_message) = if let Some(err) = first_error {
            let msg = err.to_string();
            if total_commits > 0 {
                (ProviderHealth::Partial, Some(msg))
            } else {
                match err {
                    SpoorError::RateLimitExceeded { .. } => {
                        (ProviderHealth::RateLimited, Some(msg))
                    }
                    _ => (ProviderHealth::Unavailable, Some(msg)),
                }
            }
        } else if stopped_due_to_budget || windows_partial > 0 {
            (
                ProviderHealth::Partial,
                Some("Search completed partially within request budget".to_string()),
            )
        } else {
            (ProviderHealth::Ok, None)
        };

        let commits = map
            .into_iter()
            .map(|((repo_full_name, _), (item, role))| DiscoveredCommit {
                repo_full_name,
                item,
                role,
            })
            .collect();

        SearchSessionOutcome {
            commits,
            author_commits_count,
            committer_commits_count,
            windows_complete,
            windows_partial,
            stopped_due_to_budget,
            health,
            error_message,
        }
    }

    async fn analyze_user(
        &self,
        username: &str,
        options: AnalysisOptions,
        event_tx: &Option<Sender<SpoorEvent>>,
    ) -> Result<Report, SpoorError> {
        let mut report = Report::new(
            Target::User {
                username: username.to_string(),
            },
            options.mode,
        );

        Self::emit(
            event_tx,
            SpoorEvent::ProviderStarted {
                provider: "GitHub REST".to_string(),
                operation: "Fetching user profile".to_string(),
            },
        )
        .await;

        let profile_start = Instant::now();
        let gh_user = match self.github.fetch_user_profile(username).await {
            Ok(u) => {
                report.provider_status.insert(
                    "GitHub Profile".to_string(),
                    ProviderStatus {
                        status: ProviderHealth::Ok,
                        latency_ms: Some(profile_start.elapsed().as_millis() as u64),
                        items_retrieved: Some(1),
                        rate_limit_remaining: Some(self.github.rest.rate_limit_remaining()),
                        rate_limit_reset: None,
                        retry_after_seconds: None,
                        message: None,
                    },
                );
                Self::emit(
                    event_tx,
                    SpoorEvent::ProviderCompleted {
                        provider: "GitHub REST".to_string(),
                        details: format!("Profile resolved for '{username}'"),
                    },
                )
                .await;
                u
            }
            Err(e) => {
                Self::emit(
                    event_tx,
                    SpoorEvent::LookupFailed {
                        error: e.to_string(),
                    },
                )
                .await;
                return Err(e);
            }
        };

        let now = Utc::now();
        let account_age_days = (now - gh_user.created_at).num_days();

        let profile_report = ProfileReport {
            login: gh_user.login.clone(),
            id: gh_user.id,
            node_id: gh_user.node_id.clone(),
            name: gh_user.name.clone(),
            bio: gh_user.bio.clone(),
            company: gh_user.company.clone(),
            location: gh_user.location.clone(),
            blog_url: gh_user.blog.clone(),
            twitter_username: gh_user.twitter_username.clone(),
            public_email: gh_user.email.clone(),
            avatar_url: gh_user.avatar_url.clone(),
            html_url: gh_user.html_url.clone(),
            public_repos: gh_user.public_repos,
            public_gists: gh_user.public_gists,
            followers: gh_user.followers,
            following: gh_user.following,
            created_at: gh_user.created_at,
            updated_at: gh_user.updated_at,
            account_age_days,
        };

        report.provenance.push(
            Provenance::new("GitHub REST", Classification::Observed)
                .with_source_url(&gh_user.html_url)
                .with_notes("Public GitHub user profile endpoint"),
        );

        let max_repos = match options.mode {
            AnalysisMode::Fast => 30,
            AnalysisMode::Deep => 100,
        };

        let mut candidate_map: HashMap<String, CandidateRepoEntry> = HashMap::new();
        let mut canonical_commits: HashMap<(String, String), CanonicalCommitRecord> =
            HashMap::new();
        let mut git_inspected = false;

        Self::emit(
            event_tx,
            SpoorEvent::ProviderStarted {
                provider: "GitHub REST".to_string(),
                operation: format!("Fetching public repositories (up to {max_repos})"),
            },
        )
        .await;

        let repos_res = self
            .github
            .fetch_user_repositories(username, max_repos)
            .await;
        let gh_repos = match repos_res {
            Ok(repos) => {
                Self::emit(
                    event_tx,
                    SpoorEvent::ProviderCompleted {
                        provider: "GitHub REST".to_string(),
                        details: format!("Retrieved {} repositories", repos.len()),
                    },
                )
                .await;

                report.provider_status.insert(
                    "GitHub Repositories".to_string(),
                    ProviderStatus {
                        status: ProviderHealth::Ok,
                        latency_ms: None,
                        items_retrieved: Some(repos.len()),
                        rate_limit_remaining: Some(self.github.rest.rate_limit_remaining()),
                        rate_limit_reset: None,
                        retry_after_seconds: None,
                        message: None,
                    },
                );

                for r in &repos {
                    let entry = candidate_map.entry(r.full_name.clone()).or_insert_with(|| {
                        CandidateRepoEntry {
                            owner: username.to_string(),
                            name: r.name.clone(),
                            full_name: r.full_name.clone(),
                            html_url: r.html_url.clone(),
                            is_fork: r.fork,
                            description: r.description.clone(),
                            default_branch: r.default_branch.clone(),
                            sources: Vec::new(),
                            commit_matches: 0,
                        }
                    });
                    if entry.default_branch.is_none() {
                        entry.default_branch = r.default_branch.clone();
                    }
                    if !entry.sources.contains(&"Owned".to_string()) {
                        entry.sources.push("Owned".to_string());
                    }
                }
                repos
            }
            Err(e) => {
                let health = match &e {
                    SpoorError::RateLimitExceeded { .. } => ProviderHealth::RateLimited,
                    SpoorError::Timeout { .. } => ProviderHealth::Unavailable,
                    _ => ProviderHealth::Partial,
                };
                report.provider_status.insert(
                    "GitHub Repositories".to_string(),
                    ProviderStatus {
                        status: health,
                        latency_ms: None,
                        items_retrieved: Some(0),
                        rate_limit_remaining: Some(self.github.rest.rate_limit_remaining()),
                        rate_limit_reset: None,
                        retry_after_seconds: None,
                        message: Some(e.to_string()),
                    },
                );
                report
                    .warnings
                    .push(format!("Failed to retrieve owned repositories: {e}"));
                Vec::new()
            }
        };

        Self::emit(
            event_tx,
            SpoorEvent::ProviderStarted {
                provider: "GitHub Commit Search".to_string(),
                operation: format!("Searching global commits authored or committed by {username}"),
            },
        )
        .await;

        let search_outcome = self
            .run_global_commit_search(username, gh_user.created_at, options.mode)
            .await;

        for disc in &search_outcome.commits {
            let parts: Vec<&str> = disc.repo_full_name.split('/').collect();
            let (owner, name) = if parts.len() == 2 {
                (parts[0].to_string(), parts[1].to_string())
            } else {
                (username.to_string(), disc.repo_full_name.clone())
            };

            let entry = candidate_map
                .entry(disc.repo_full_name.clone())
                .or_insert_with(|| CandidateRepoEntry {
                    owner,
                    name,
                    full_name: disc.repo_full_name.clone(),
                    html_url: format!("https://github.com/{}", disc.repo_full_name),
                    is_fork: false,
                    description: None,
                    default_branch: None,
                    sources: Vec::new(),
                    commit_matches: 0,
                });
            entry.commit_matches += 1;
            if !entry.sources.contains(&"Commit Search".to_string()) {
                entry.sources.push("Commit Search".to_string());
            }

            let key = (disc.repo_full_name.to_lowercase(), disc.item.sha.clone());
            let c_entry = canonical_commits
                .entry(key)
                .or_insert_with(|| CanonicalCommitRecord {
                    repo_full_name: disc.repo_full_name.clone(),
                    sha: disc.item.sha.clone(),
                    html_url: disc.item.html_url.clone(),
                    detail: disc.item.commit.clone(),
                    author_user: disc.item.author.clone(),
                    committer_user: disc.item.committer.clone(),
                    search_role: Some(disc.role),
                    sources: HashSet::new(),
                });
            c_entry.sources.insert("GitHub Commit Search".to_string());
            if c_entry.search_role.is_none() {
                c_entry.search_role = Some(disc.role);
            } else if c_entry.search_role != Some(disc.role) {
                c_entry.search_role = Some(CommitMatchRole::Both);
            }
            if c_entry.author_user.is_none() && disc.item.author.is_some() {
                c_entry.author_user = disc.item.author.clone();
            }
            if c_entry.committer_user.is_none() && disc.item.committer.is_some() {
                c_entry.committer_user = disc.item.committer.clone();
            }
        }

        let total_search_commits =
            search_outcome.author_commits_count + search_outcome.committer_commits_count;
        report.provider_status.insert(
            "GitHub Commit Search".to_string(),
            ProviderStatus {
                status: search_outcome.health,
                latency_ms: None,
                items_retrieved: Some(total_search_commits),
                rate_limit_remaining: Some(self.github.rest.rate_limit_remaining()),
                rate_limit_reset: None,
                retry_after_seconds: None,
                message: search_outcome.error_message.clone(),
            },
        );
        if let Some(ref err) = search_outcome.error_message {
            report
                .warnings
                .push(format!("GitHub commit search encountered an issue: {err}"));
        }

        let mut graphql_years_fully_traversed = false;
        if self.github.is_authenticated() {
            Self::emit(
                event_tx,
                SpoorEvent::ProviderStarted {
                    provider: "GitHub GraphQL".to_string(),
                    operation: format!("Querying contributed repositories for {username}"),
                },
            )
            .await;

            if options.mode == AnalysisMode::Deep {
                match self
                    .github
                    .fetch_user_contributions_historical(username, 5)
                    .await
                {
                    Ok(hist) => {
                        graphql_years_fully_traversed = hist.fully_traversed;
                        report.provider_status.insert(
                            "GitHub Contributions (GraphQL)".to_string(),
                            ProviderStatus {
                                status: ProviderHealth::Ok,
                                latency_ms: None,
                                items_retrieved: Some(hist.contributions.len()),
                                rate_limit_remaining: None,
                                rate_limit_reset: None,
                                retry_after_seconds: None,
                                message: Some(format!(
                                    "Discovered contributions across {} years",
                                    hist.years_queried.len()
                                )),
                            },
                        );
                        for c in hist.contributions {
                            if !c.repository.is_private {
                                let repo_full_name = c.repository.name_with_owner.clone();
                                let years_str = c
                                    .years
                                    .iter()
                                    .map(|y| y.to_string())
                                    .collect::<Vec<_>>()
                                    .join(", ");
                                let src_label = format!("Contribution ({years_str})");

                                let entry = candidate_map
                                    .entry(repo_full_name.clone())
                                    .or_insert_with(|| CandidateRepoEntry {
                                        owner: c.repository.owner.login.clone(),
                                        name: c.repository.name.clone(),
                                        full_name: repo_full_name,
                                        html_url: c.repository.url.clone(),
                                        is_fork: c.repository.is_fork,
                                        description: c.repository.description.clone(),
                                        default_branch: None,
                                        sources: Vec::new(),
                                        commit_matches: 0,
                                    });
                                if !entry.sources.contains(&src_label) {
                                    entry.sources.push(src_label);
                                }
                            }
                        }
                    }
                    Err(e) => {
                        report.provider_status.insert(
                            "GitHub Contributions (GraphQL)".to_string(),
                            ProviderStatus {
                                status: ProviderHealth::Partial,
                                latency_ms: None,
                                items_retrieved: Some(0),
                                rate_limit_remaining: None,
                                rate_limit_reset: None,
                                retry_after_seconds: None,
                                message: Some(e.to_string()),
                            },
                        );
                        report
                            .warnings
                            .push(format!("GraphQL contributions query failed: {e}"));
                    }
                }
            } else {
                match self.github.fetch_user_contributions(username).await {
                    Ok(contribs) => {
                        report.provider_status.insert(
                            "GitHub Contributions (GraphQL)".to_string(),
                            ProviderStatus {
                                status: ProviderHealth::Ok,
                                latency_ms: None,
                                items_retrieved: Some(contribs.len()),
                                rate_limit_remaining: None,
                                rate_limit_reset: None,
                                retry_after_seconds: None,
                                message: None,
                            },
                        );
                        for c in contribs {
                            if !c.repository.is_private {
                                let repo_full_name = c.repository.name_with_owner.clone();
                                let entry = candidate_map
                                    .entry(repo_full_name.clone())
                                    .or_insert_with(|| CandidateRepoEntry {
                                        owner: c.repository.owner.login.clone(),
                                        name: c.repository.name.clone(),
                                        full_name: repo_full_name,
                                        html_url: c.repository.url.clone(),
                                        is_fork: c.repository.is_fork,
                                        description: c.repository.description.clone(),
                                        default_branch: None,
                                        sources: Vec::new(),
                                        commit_matches: 0,
                                    });
                                if !entry.sources.contains(&"Contribution".to_string()) {
                                    entry.sources.push("Contribution".to_string());
                                }
                            }
                        }
                    }
                    Err(e) => {
                        report.provider_status.insert(
                            "GitHub Contributions (GraphQL)".to_string(),
                            ProviderStatus {
                                status: ProviderHealth::Partial,
                                latency_ms: None,
                                items_retrieved: Some(0),
                                rate_limit_remaining: None,
                                rate_limit_reset: None,
                                retry_after_seconds: None,
                                message: Some(e.to_string()),
                            },
                        );
                        report
                            .warnings
                            .push(format!("GraphQL contributions query failed: {e}"));
                    }
                }
            }
        } else {
            report.provider_status.insert(
                "GitHub Contributions (GraphQL)".to_string(),
                ProviderStatus {
                    status: ProviderHealth::Skipped,
                    latency_ms: None,
                    items_retrieved: None,
                    rate_limit_remaining: None,
                    rate_limit_reset: None,
                    retry_after_seconds: None,
                    message: Some("GraphQL requires authenticated GITHUB_TOKEN".to_string()),
                },
            );
        }

        Self::emit(
            event_tx,
            SpoorEvent::ProviderStarted {
                provider: "GitHub Events".to_string(),
                operation: format!("Fetching public events for {username}"),
            },
        )
        .await;

        let event_result = self.github.fetch_user_events(username, 300).await;
        let (events, event_warning, event_health) = match event_result {
            Ok(batch) => {
                let health = if batch.is_complete {
                    ProviderHealth::Ok
                } else {
                    ProviderHealth::Partial
                };
                for ev in &batch.events {
                    let repo_name = ev.repo.name.clone();
                    let parts: Vec<&str> = repo_name.split('/').collect();
                    if parts.len() == 2 {
                        let entry = candidate_map.entry(repo_name.clone()).or_insert_with(|| {
                            CandidateRepoEntry {
                                owner: parts[0].to_string(),
                                name: parts[1].to_string(),
                                full_name: repo_name.clone(),
                                html_url: format!("https://github.com/{repo_name}"),
                                is_fork: false,
                                description: None,
                                default_branch: None,
                                sources: Vec::new(),
                                commit_matches: 0,
                            }
                        });
                        if !entry.sources.contains(&"Recent Activity".to_string()) {
                            entry.sources.push("Recent Activity".to_string());
                        }
                    }
                }
                (batch.events, batch.warning, health)
            }
            Err(error) => {
                let health = if matches!(&error, SpoorError::RateLimitExceeded { .. }) {
                    ProviderHealth::RateLimited
                } else {
                    ProviderHealth::Unavailable
                };
                report
                    .warnings
                    .push(format!("Failed to retrieve public events: {error}"));
                (
                    Vec::new(),
                    Some(format!("Public events request failed: {error}")),
                    health,
                )
            }
        };
        let base_event_note = "GitHub public events cover only the previous 30 days, up to 300 events. Spoor requests up to three pages of 100 and filters older results.";
        let window_note = event_warning.as_ref().map_or_else(
            || base_event_note.to_string(),
            |warning| format!("{base_event_note} {warning}"),
        );
        report.provider_status.insert(
            "Public Events".to_string(),
            ProviderStatus {
                status: event_health,
                latency_ms: None,
                items_retrieved: Some(events.len()),
                rate_limit_remaining: Some(self.github.rest.rate_limit_remaining()),
                rate_limit_reset: None,
                retry_after_seconds: None,
                message: event_warning,
            },
        );

        let mut owned_candidates = Vec::new();
        let mut external_candidates = Vec::new();

        for candidate in candidate_map.values() {
            if candidate.owner.eq_ignore_ascii_case(username) {
                owned_candidates.push(candidate.clone());
            } else {
                external_candidates.push(candidate.clone());
            }
        }

        let owned_discovered = owned_candidates.len();
        let external_discovered = external_candidates.len();

        owned_candidates.sort_by_key(|c| {
            let mut score = 0i32;
            if !c.is_fork {
                score += 50;
            }
            score += (c.commit_matches as i32) * 10;
            if c.sources.contains(&"Recent Activity".to_string()) {
                score += 20;
            }
            std::cmp::Reverse(score)
        });

        external_candidates.sort_by_key(|c| {
            let mut score = 0i32;
            score += (c.commit_matches as i32) * 100;
            if c.sources.iter().any(|s| s.starts_with("Contribution")) {
                score += 50;
            }
            if c.sources.contains(&"Recent Activity".to_string()) {
                score += 20;
            }
            if !c.is_fork {
                score += 10;
            }
            std::cmp::Reverse(score)
        });

        let (max_total_candidates, reserved_external, reserved_owned, commit_per_repo_limit) =
            match options.mode {
                AnalysisMode::Fast => (8, 4, 4, 50),
                AnalysisMode::Deep => (25, 12, 13, 500),
            };

        let mut eligible_candidates = Vec::new();
        let mut ext_iter = external_candidates.into_iter();
        let mut owned_iter = owned_candidates.into_iter();

        for _ in 0..reserved_external {
            if let Some(c) = ext_iter.next() {
                eligible_candidates.push(c);
            }
        }
        for _ in 0..reserved_owned {
            if let Some(c) = owned_iter.next() {
                eligible_candidates.push(c);
            }
        }
        while eligible_candidates.len() < max_total_candidates {
            if let Some(c) = ext_iter.next() {
                eligible_candidates.push(c);
            } else if let Some(c) = owned_iter.next() {
                eligible_candidates.push(c);
            } else {
                break;
            }
        }

        let candidates_selected = eligible_candidates.len();
        let owned_selected = eligible_candidates
            .iter()
            .filter(|c| c.owner.eq_ignore_ascii_case(username))
            .count();
        let external_selected = eligible_candidates
            .iter()
            .filter(|c| !c.owner.eq_ignore_ascii_case(username))
            .count();
        let candidates_skipped = candidate_map.len().saturating_sub(candidates_selected);

        let mut scanned_repo_meta: HashMap<String, ScannedRepoMeta> = HashMap::new();
        let mut repository_scans_attempted = 0usize;
        let mut repository_scans_succeeded = 0usize;
        let mut repository_scans_failed = 0usize;

        for (idx, r) in eligible_candidates.iter().enumerate() {
            Self::emit(
                event_tx,
                SpoorEvent::ScanProgress {
                    current: idx + 1,
                    total: eligible_candidates.len(),
                    message: format!("Inspecting repository {}", r.full_name),
                },
            )
            .await;

            repository_scans_attempted += 1;
            let commits_res = self
                .github
                .fetch_repository_commits(&r.owner, &r.name, commit_per_repo_limit)
                .await;

            match commits_res {
                Ok(commits) => {
                    repository_scans_succeeded += 1;
                    if !commits.is_empty() {
                        for c in commits {
                            let key = (r.full_name.to_lowercase(), c.sha.clone());
                            let c_entry = canonical_commits.entry(key).or_insert_with(|| {
                                CanonicalCommitRecord {
                                    repo_full_name: r.full_name.clone(),
                                    sha: c.sha.clone(),
                                    html_url: c.html_url.clone(),
                                    detail: c.commit.clone(),
                                    author_user: c.author.clone(),
                                    committer_user: c.committer.clone(),
                                    search_role: None,
                                    sources: HashSet::new(),
                                }
                            });
                            c_entry.sources.insert("GitHub REST API".to_string());
                            if c_entry.author_user.is_none() && c.author.is_some() {
                                c_entry.author_user = c.author.clone();
                            }
                            if c_entry.committer_user.is_none() && c.committer.is_some() {
                                c_entry.committer_user = c.committer.clone();
                            }
                        }
                    }
                }
                Err(e) => {
                    repository_scans_failed += 1;
                    report.warnings.push(format!(
                        "Failed to retrieve commits for {}: {e}",
                        r.full_name
                    ));
                }
            }

            let langs_res = self
                .github
                .fetch_repository_languages(&r.owner, &r.name)
                .await;
            let langs = match langs_res {
                Ok(l) => Some(l),
                Err(e) => {
                    if !matches!(&e, SpoorError::RepoNotFound(_)) {
                        report.warnings.push(format!(
                            "Failed to retrieve languages for {}: {e}",
                            r.full_name
                        ));
                    }
                    None
                }
            };

            let default_branch = r.default_branch.as_deref().unwrap_or("HEAD");
            let tree_res = self
                .github
                .fetch_repository_tree(&r.owner, &r.name, default_branch)
                .await;

            let hygiene = match tree_res {
                Ok(tree) if !tree.tree.is_empty() => Some(detect_hygiene_from_paths(
                    tree.tree.iter().map(|item| item.path.as_str()),
                )),
                Ok(_) => None,
                Err(e) => {
                    if !matches!(&e, SpoorError::RepoNotFound(_)) {
                        report
                            .warnings
                            .push(format!("Failed to inspect tree for {}: {e}", r.full_name));
                    }
                    None
                }
            };

            scanned_repo_meta.insert(r.full_name.clone(), (langs, hygiene));
        }

        let mut git_attempted = 0usize;
        let mut git_succeeded = 0usize;
        let mut git_failed = 0usize;
        let mut git_timed_out = 0usize;

        if options.mode == AnalysisMode::Deep {
            if GitInspector::is_git_available().await {
                Self::emit(
                    event_tx,
                    SpoorEvent::ProviderStarted {
                        provider: "Git History Inspection".to_string(),
                        operation: "Cloning bare repository metadata for deep history scan"
                            .to_string(),
                    },
                )
                .await;

                let git_inspector = GitInspector::default();
                let mut git_commits_found = 0usize;
                let git_candidates: Vec<_> = eligible_candidates
                    .iter()
                    .filter(|c| c.owner.eq_ignore_ascii_case(username) || c.commit_matches > 0)
                    .take(5)
                    .collect();

                for candidate in &git_candidates {
                    git_attempted += 1;
                    match git_inspector
                        .inspect_repo_history(&candidate.owner, &candidate.name, 2000)
                        .await
                    {
                        Ok(git_commits) => {
                            git_succeeded += 1;
                            git_commits_found += git_commits.len();
                            for gc in git_commits {
                                let key = (gc.repository.to_lowercase(), gc.sha.clone());
                                let c_entry = canonical_commits.entry(key).or_insert_with(|| {
                                    CanonicalCommitRecord {
                                        repo_full_name: gc.repository.clone(),
                                        sha: gc.sha.clone(),
                                        html_url: format!(
                                            "https://github.com/{}/{}/commit/{}",
                                            candidate.owner, candidate.name, gc.sha
                                        ),
                                        detail: GhCommitDetail {
                                            author: Some(GhCommitActor {
                                                name: gc.author_name,
                                                email: gc.author_email,
                                                date: gc.author_date,
                                            }),
                                            committer: Some(GhCommitActor {
                                                name: gc.committer_name,
                                                email: gc.committer_email,
                                                date: gc.committer_date,
                                            }),
                                            message: gc.message_subject,
                                            verification: if gc.is_verified {
                                                Some(GhVerification {
                                                    verified: true,
                                                    reason: "valid".to_string(),
                                                    signature: None,
                                                })
                                            } else {
                                                None
                                            },
                                        },
                                        author_user: None,
                                        committer_user: None,
                                        search_role: None,
                                        sources: HashSet::new(),
                                    }
                                });
                                c_entry.sources.insert("Bare Git History".to_string());
                            }
                        }
                        Err(SpoorError::Timeout { .. }) => {
                            git_timed_out += 1;
                            git_failed += 1;
                            report.warnings.push(format!(
                                "Git bare clone timed out for {}",
                                candidate.full_name
                            ));
                        }
                        Err(e) => {
                            git_failed += 1;
                            report.warnings.push(format!(
                                "Git bare clone failed for {}: {e}",
                                candidate.full_name
                            ));
                        }
                    }
                }

                git_inspected = git_succeeded > 0;
                let (status, msg) = if git_attempted == 0 {
                    (
                        ProviderHealth::Skipped,
                        "No candidate repositories matched git history inspection criteria"
                            .to_string(),
                    )
                } else if git_succeeded == git_attempted {
                    (ProviderHealth::Ok, format!("Inspected all {git_succeeded} candidate repositories via bare git clone"))
                } else if git_succeeded > 0 {
                    (ProviderHealth::Partial, format!("{git_succeeded} of {git_attempted} candidate repositories inspected ({git_failed} failed)"))
                } else if git_timed_out > 0 {
                    (
                        ProviderHealth::Unavailable,
                        format!("All {git_attempted} candidate git clone operations timed out"),
                    )
                } else {
                    (
                        ProviderHealth::Unavailable,
                        format!("All {git_attempted} candidate git clone operations failed"),
                    )
                };

                report.provider_status.insert(
                    "Git History".to_string(),
                    ProviderStatus {
                        status,
                        latency_ms: None,
                        items_retrieved: Some(git_commits_found),
                        rate_limit_remaining: None,
                        rate_limit_reset: None,
                        retry_after_seconds: None,
                        message: Some(msg),
                    },
                );
            } else {
                report.provider_status.insert(
                    "Git History".to_string(),
                    ProviderStatus {
                        status: ProviderHealth::Skipped,
                        latency_ms: None,
                        items_retrieved: None,
                        rate_limit_remaining: None,
                        rate_limit_reset: None,
                        retry_after_seconds: None,
                        message: Some(
                            "git command line tool not available in system PATH".to_string(),
                        ),
                    },
                );
            }
        } else {
            report.provider_status.insert(
                "Git History".to_string(),
                ProviderStatus {
                    status: ProviderHealth::Skipped,
                    latency_ms: None,
                    items_retrieved: None,
                    rate_limit_remaining: None,
                    rate_limit_reset: None,
                    retry_after_seconds: None,
                    message: Some("Native git cloning excluded in FAST mode".to_string()),
                },
            );
        }

        let mut repo_reports = Vec::new();
        let mut total_stars = 0u64;
        let mut total_forks = 0u64;
        let mut fork_count = 0usize;
        let mut archived_count = 0usize;
        let mut language_counts: BTreeMap<String, u64> = BTreeMap::new();

        for r in &gh_repos {
            total_stars += r.stargazers_count;
            total_forks += r.forks_count;
            if r.fork {
                fork_count += 1;
            }
            if r.archived.unwrap_or(false) {
                archived_count += 1;
            }
            if let Some(lang) = &r.language {
                *language_counts.entry(lang.clone()).or_insert(0) += 1;
            }

            let (langs, hyg) = match scanned_repo_meta.remove(&r.full_name) {
                Some((l, h)) => (l, h),
                None => (None, None),
            };

            repo_reports.push(RepoReport {
                name: r.name.clone(),
                full_name: r.full_name.clone(),
                description: r.description.clone(),
                is_fork: r.fork,
                is_archived: r.archived.unwrap_or(false),
                default_branch: r.default_branch.clone().unwrap_or_else(|| "main".into()),
                stars: r.stargazers_count,
                forks: r.forks_count,
                open_issues: r.open_issues_count,
                size_kb: r.size,
                primary_language: r.language.clone(),
                languages: langs,
                created_at: r.created_at,
                updated_at: r.updated_at,
                pushed_at: r.pushed_at,
                topics: r.topics.clone().unwrap_or_default(),
                license_spdx: r.license.as_ref().and_then(|l| l.spdx_id.clone()),
                hygiene: hyg,
                html_url: r.html_url.clone(),
            });
        }

        for r in &eligible_candidates {
            if !gh_repos
                .iter()
                .any(|gr| gr.full_name.eq_ignore_ascii_case(&r.full_name))
            {
                let (langs, hyg) = match scanned_repo_meta.remove(&r.full_name) {
                    Some((l, h)) => (l, h),
                    None => (None, None),
                };
                repo_reports.push(RepoReport {
                    name: r.name.clone(),
                    full_name: r.full_name.clone(),
                    description: r.description.clone(),
                    is_fork: r.is_fork,
                    is_archived: false,
                    default_branch: r.default_branch.clone().unwrap_or_else(|| "HEAD".into()),
                    stars: 0,
                    forks: 0,
                    open_issues: 0,
                    size_kb: 0,
                    primary_language: None,
                    languages: langs,
                    created_at: None,
                    updated_at: None,
                    pushed_at: None,
                    topics: Vec::new(),
                    license_spdx: None,
                    hygiene: hyg,
                    html_url: r.html_url.clone(),
                });
            }
        }

        Self::emit(
            event_tx,
            SpoorEvent::ProviderStarted {
                provider: "GitHub Keys".to_string(),
                operation: "Fetching public GPG and SSH keys".to_string(),
            },
        )
        .await;

        let gpg_keys = self
            .github
            .fetch_user_gpg_keys(username)
            .await
            .unwrap_or_default();
        let ssh_keys = self
            .github
            .fetch_user_ssh_keys(username)
            .await
            .unwrap_or_default();

        let mut key_records = Vec::new();
        for k in &gpg_keys {
            let emails = k
                .emails
                .as_ref()
                .map(|list| list.iter().map(|e| e.email.clone()).collect())
                .unwrap_or_default();

            key_records.push(KeyRecord {
                key_type: KeyType::Gpg,
                key_id: k.key_id.clone(),
                fingerprint: k.key_id.clone(),
                raw_preview: k
                    .public_key
                    .as_deref()
                    .or(k.raw_key.as_deref())
                    .unwrap_or("")
                    .chars()
                    .take(80)
                    .collect::<String>()
                    + "...",
                emails,
                created_at: k.created_at,
                expires_at: k.expires_at,
                can_sign: k.can_sign,
                source: format!("https://api.github.com/users/{username}/gpg_keys"),
            });
        }

        for k in &ssh_keys {
            key_records.push(KeyRecord {
                key_type: KeyType::Ssh,
                key_id: Some(k.id.to_string()),
                fingerprint: None,
                raw_preview: k.key.chars().take(80).collect::<String>() + "...",
                emails: Vec::new(),
                created_at: k.created_at,
                expires_at: None,
                can_sign: Some(true),
                source: format!("https://api.github.com/users/{username}/keys"),
            });
        }

        report.provider_status.insert(
            "Public Keys".to_string(),
            ProviderStatus {
                status: ProviderHealth::Ok,
                latency_ms: None,
                items_retrieved: Some(key_records.len()),
                rate_limit_remaining: Some(self.github.rest.rate_limit_remaining()),
                rate_limit_reset: None,
                retry_after_seconds: None,
                message: Some(format!(
                    "{} GPG keys, {} SSH keys",
                    gpg_keys.len(),
                    ssh_keys.len()
                )),
            },
        );

        let mut activity_report = ActivityReport {
            window_note,
            total_events_observed: events.len(),
            ..Default::default()
        };

        let mut active_repos_set = HashSet::new();
        for ev in &events {
            active_repos_set.insert(ev.repo.name.clone());
            match ev.event_type.as_str() {
                "PushEvent" => activity_report.push_events += 1,
                "PullRequestEvent" => activity_report.pull_request_events += 1,
                "IssuesEvent" => activity_report.issue_events += 1,
                "ReleaseEvent" => activity_report.release_events += 1,
                "ForkEvent" => activity_report.fork_events += 1,
                _ => activity_report.other_events += 1,
            }
            if activity_report.recent_actions.len() < 15 {
                activity_report.recent_actions.push(ActivityEventSummary {
                    event_type: ev.event_type.clone(),
                    repository: ev.repo.name.clone(),
                    created_at: ev.created_at,
                    detail: None,
                });
            }
        }
        activity_report.active_repositories = active_repos_set.into_iter().collect();

        let mut gitleak_data: Option<GitLeakResponse> = None;
        if options.enrich {
            Self::emit(
                event_tx,
                SpoorEvent::ProviderStarted {
                    provider: "GitLeak".to_string(),
                    operation: "Requesting GitLeak enrichment".to_string(),
                },
            )
            .await;

            let gitleak_cache_key = format!("gitleak:{username}");
            if let Ok(Some(cached_str)) = self.cache.get("gitleak", &gitleak_cache_key, "v2").await
            {
                if let Ok(resp) = serde_json::from_str::<GitLeakResponse>(&cached_str) {
                    report.provider_status.insert(
                        "GitLeak".to_string(),
                        ProviderStatus {
                            status: ProviderHealth::Cached,
                            latency_ms: None,
                            items_retrieved: Some(resp.emails.len()),
                            rate_limit_remaining: None,
                            rate_limit_reset: None,
                            retry_after_seconds: None,
                            message: Some("Loaded from cache (TTL 24h)".to_string()),
                        },
                    );
                    gitleak_data = Some(resp);
                }
            }

            if gitleak_data.is_none() {
                match self.gitleak_budget.try_acquire().await {
                    Ok(()) => {
                        let gl_outcome = self.gitleak.lookup(username, false, true).await?;
                        match gl_outcome {
                            GitLeakOutcome::Success(resp) => {
                                if let Ok(s) = serde_json::to_string(&resp) {
                                    let _ = self
                                        .cache
                                        .set("gitleak", &gitleak_cache_key, "v2", &s, 86400)
                                        .await;
                                }
                                report.provider_status.insert(
                                    "GitLeak".to_string(),
                                    ProviderStatus {
                                        status: ProviderHealth::Ok,
                                        latency_ms: resp
                                            .metadata
                                            .as_ref()
                                            .and_then(|m| m.execution_time_ms),
                                        items_retrieved: Some(resp.emails.len()),
                                        rate_limit_remaining: Some(
                                            self.gitleak_budget.remaining().await as u64,
                                        ),
                                        rate_limit_reset: None,
                                        retry_after_seconds: None,
                                        message: None,
                                    },
                                );
                                gitleak_data = Some(*resp);
                            }
                            GitLeakOutcome::RateLimited {
                                retry_after,
                                message,
                            } => {
                                report
                                    .warnings
                                    .push(format!("GitLeak rate limited: {message}"));
                                report.provider_status.insert(
                                    "GitLeak".to_string(),
                                    ProviderStatus {
                                        status: ProviderHealth::RateLimited,
                                        latency_ms: None,
                                        items_retrieved: Some(0),
                                        rate_limit_remaining: Some(0),
                                        rate_limit_reset: None,
                                        retry_after_seconds: retry_after,
                                        message: Some(message),
                                    },
                                );
                            }
                            GitLeakOutcome::NoActivity => {
                                report.provider_status.insert(
                                    "GitLeak".to_string(),
                                    ProviderStatus {
                                        status: ProviderHealth::Ok,
                                        latency_ms: None,
                                        items_retrieved: Some(0),
                                        rate_limit_remaining: Some(
                                            self.gitleak_budget.remaining().await as u64,
                                        ),
                                        rate_limit_reset: None,
                                        retry_after_seconds: None,
                                        message: Some(
                                            "No public commit activity found by GitLeak"
                                                .to_string(),
                                        ),
                                    },
                                );
                            }
                            GitLeakOutcome::NotFound => {
                                report.provider_status.insert(
                                    "GitLeak".to_string(),
                                    ProviderStatus {
                                        status: ProviderHealth::Unavailable,
                                        latency_ms: None,
                                        items_retrieved: None,
                                        rate_limit_remaining: Some(
                                            self.gitleak_budget.remaining().await as u64,
                                        ),
                                        rate_limit_reset: None,
                                        retry_after_seconds: None,
                                        message: Some(
                                            "Target user not found in GitLeak index".to_string(),
                                        ),
                                    },
                                );
                            }
                            GitLeakOutcome::TimedOut => {
                                report
                                    .warnings
                                    .push("GitLeak lookup timed out (8s limit)".to_string());
                                report.provider_status.insert(
                                    "GitLeak".to_string(),
                                    ProviderStatus {
                                        status: ProviderHealth::Unavailable,
                                        latency_ms: None,
                                        items_retrieved: None,
                                        rate_limit_remaining: Some(
                                            self.gitleak_budget.remaining().await as u64,
                                        ),
                                        rate_limit_reset: None,
                                        retry_after_seconds: None,
                                        message: Some(
                                            "Upstream GitLeak 8-second timeout exceeded"
                                                .to_string(),
                                        ),
                                    },
                                );
                            }
                            GitLeakOutcome::Unavailable(msg) => {
                                report.warnings.push(format!("GitLeak unavailable: {msg}"));
                                report.provider_status.insert(
                                    "GitLeak".to_string(),
                                    ProviderStatus {
                                        status: ProviderHealth::Unavailable,
                                        latency_ms: None,
                                        items_retrieved: None,
                                        rate_limit_remaining: Some(
                                            self.gitleak_budget.remaining().await as u64,
                                        ),
                                        rate_limit_reset: None,
                                        retry_after_seconds: None,
                                        message: Some(msg),
                                    },
                                );
                            }
                        }
                    }
                    Err(retry_after) => {
                        let msg = "GitLeak provider hourly budget exhausted; skipped to preserve upstream quota".to_string();
                        report.warnings.push(msg.clone());
                        report.provider_status.insert(
                            "GitLeak".to_string(),
                            ProviderStatus {
                                status: ProviderHealth::RateLimited,
                                latency_ms: None,
                                items_retrieved: Some(0),
                                rate_limit_remaining: Some(0),
                                rate_limit_reset: None,
                                retry_after_seconds: Some(retry_after),
                                message: Some(msg),
                            },
                        );
                    }
                }
            }
        } else {
            // enrich == false: skip GitLeak to preserve quota
            report.provider_status.insert(
                "GitLeak".to_string(),
                ProviderStatus {
                    status: ProviderHealth::Skipped,
                    latency_ms: None,
                    items_retrieved: None,
                    rate_limit_remaining: None,
                    rate_limit_reset: None,
                    retry_after_seconds: None,
                    message: Some("Skipped to preserve rate-limit quota".to_string()),
                },
            );
        }

        let mut target_anchors: HashSet<String> = HashSet::new();

        if let Some(pub_email) = &profile_report.public_email {
            let email_clean = pub_email.trim().to_lowercase();
            if !email_clean.is_empty() && !is_github_platform_system(&email_clean) {
                target_anchors.insert(email_clean);
            }
        }

        let noreply_plain = format!("{}@users.noreply.github.com", username.to_lowercase());
        target_anchors.insert(noreply_plain);
        let noreply_id = format!(
            "{}+{}@users.noreply.github.com",
            gh_user.id,
            username.to_lowercase()
        );
        target_anchors.insert(noreply_id);

        for k in &gpg_keys {
            if let Some(emails) = &k.emails {
                for e in emails {
                    if e.verified == Some(true) {
                        let email_clean = e.email.trim().to_lowercase();
                        if !email_clean.is_empty() && !is_github_platform_system(&email_clean) {
                            target_anchors.insert(email_clean);
                        }
                    }
                }
            }
        }

        if let Some(gl) = &gitleak_data {
            for e in &gl.emails {
                let email_clean = e.email.trim().to_lowercase();
                if !email_clean.is_empty() && !is_github_platform_system(&email_clean) {
                    target_anchors.insert(email_clean);
                }
            }
        }

        for commit in canonical_commits.values() {
            let author_email = commit
                .detail
                .author
                .as_ref()
                .map(|a| a.email.trim().to_lowercase())
                .filter(|e| !e.is_empty());
            let committer_email = commit
                .detail
                .committer
                .as_ref()
                .map(|c| c.email.trim().to_lowercase())
                .filter(|e| !e.is_empty());

            if let Some(a_email) = author_email {
                if !is_github_platform_system(&a_email) {
                    let search_match = matches!(
                        commit.search_role,
                        Some(CommitMatchRole::Author) | Some(CommitMatchRole::Both)
                    );
                    let rest_match = commit.author_user.as_ref().is_some_and(|u| {
                        u.login.eq_ignore_ascii_case(username) && u.id == gh_user.id
                    });
                    let noreply_match = is_target_noreply(&a_email, username, Some(gh_user.id));
                    if search_match || rest_match || noreply_match {
                        target_anchors.insert(a_email);
                    }
                }
            }

            if let Some(c_email) = committer_email {
                if !is_github_platform_system(&c_email) {
                    let search_match = matches!(
                        commit.search_role,
                        Some(CommitMatchRole::Committer) | Some(CommitMatchRole::Both)
                    );
                    let rest_match = commit.committer_user.as_ref().is_some_and(|u| {
                        u.login.eq_ignore_ascii_case(username) && u.id == gh_user.id
                    });
                    let noreply_match = is_target_noreply(&c_email, username, Some(gh_user.id));
                    if search_match || rest_match || noreply_match {
                        target_anchors.insert(c_email);
                    }
                }
            }
        }

        let mut identities_map: HashMap<String, IdentityRecord> = HashMap::new();
        let mut related_actors_map: HashMap<String, RelatedActorRecord> = HashMap::new();

        if let Some(pub_email) = &profile_report.public_email {
            let email_clean = pub_email.trim().to_string();
            let email_norm = email_clean.to_lowercase();
            if !email_norm.is_empty() && !is_github_platform_system(&email_norm) {
                let email_type = classify_email(&email_clean, Some(&email_clean));
                identities_map.insert(
                    email_norm,
                    IdentityRecord {
                        name: profile_report.name.clone(),
                        email: email_clean,
                        email_type,
                        first_seen: None,
                        last_seen: None,
                        commits_observed: 0,
                        repositories: Vec::new(),
                        sources: vec!["GitHub Public Profile".to_string()],
                        is_verified_signer: false,
                    },
                );
            }
        }

        for k in &gpg_keys {
            if let Some(emails) = &k.emails {
                for e in emails {
                    if e.verified == Some(true) {
                        let email_clean = e.email.trim().to_string();
                        let email_norm = email_clean.to_lowercase();
                        if !email_norm.is_empty() && !is_github_platform_system(&email_norm) {
                            let email_type = classify_email(
                                &email_clean,
                                profile_report.public_email.as_deref(),
                            );
                            let entry = identities_map.entry(email_norm).or_insert_with(|| {
                                IdentityRecord {
                                    name: profile_report.name.clone(),
                                    email: email_clean,
                                    email_type,
                                    first_seen: None,
                                    last_seen: None,
                                    commits_observed: 0,
                                    repositories: Vec::new(),
                                    sources: vec!["GitHub GPG Key".to_string()],
                                    is_verified_signer: true,
                                }
                            });
                            if !entry.sources.iter().any(|s| s == "GitHub GPG Key") {
                                entry.sources.push("GitHub GPG Key".to_string());
                            }
                            entry.is_verified_signer = true;
                        }
                    }
                }
            }
        }

        if let Some(gl) = &gitleak_data {
            for email_entry in &gl.emails {
                let norm = email_entry.email.trim().to_lowercase();
                if norm.is_empty() || is_github_platform_system(&norm) {
                    continue;
                }

                let extracted_repo = email_entry
                    .commit_url
                    .as_deref()
                    .and_then(extract_repo_from_commit_url);

                let entry = identities_map
                    .entry(norm)
                    .or_insert_with(|| IdentityRecord {
                        name: email_entry.committer.clone(),
                        email: email_entry.email.trim().to_string(),
                        email_type: classify_email(
                            &email_entry.email,
                            profile_report.public_email.as_deref(),
                        ),
                        first_seen: None,
                        last_seen: None,
                        commits_observed: 0,
                        repositories: Vec::new(),
                        sources: vec!["GitLeak".to_string()],
                        is_verified_signer: false,
                    });

                if !entry.sources.iter().any(|s| s == "GitLeak") {
                    entry.sources.push("GitLeak".to_string());
                }

                if entry.name.is_none() && email_entry.committer.is_some() {
                    entry.name = email_entry.committer.clone();
                }

                if let Some(repo) = &extracted_repo {
                    if !entry.repositories.contains(repo) {
                        entry.repositories.push(repo.clone());
                    }
                }

                if let Some(commit_url) = &email_entry.commit_url {
                    let mut prov = Provenance::new("GitLeak", Classification::Observed)
                        .with_source_url(commit_url)
                        .with_notes(format!("Email observed via GitLeak: {}", email_entry.email));
                    if let Some(repo) = &extracted_repo {
                        prov = prov.with_repository(repo);
                    }
                    report.provenance.push(prov);
                }
            }

            if let Some(meta) = &gl.metadata {
                for gpg in &meta.gpg_keys {
                    let key_id = gpg
                        .get("keyId")
                        .or(gpg.get("key_id"))
                        .and_then(|v| v.as_str());
                    if let Some(kid) = key_id {
                        let exists = key_records.iter().any(|k| k.key_id.as_deref() == Some(kid));
                        if !exists {
                            key_records.push(KeyRecord {
                                key_type: KeyType::Gpg,
                                key_id: Some(kid.to_string()),
                                fingerprint: Some(kid.to_string()),
                                raw_preview: format!("GPG Key ID: {kid}"),
                                emails: Vec::new(),
                                created_at: None,
                                expires_at: None,
                                can_sign: Some(true),
                                source: "GitLeak Metadata".to_string(),
                            });
                        }
                    }
                }

                for ssh in &meta.ssh_keys {
                    let fp = ssh
                        .get("fingerprint")
                        .or(ssh.get("id"))
                        .and_then(|v| v.as_str());
                    if let Some(f) = fp {
                        let exists = key_records.iter().any(|k| {
                            k.fingerprint.as_deref() == Some(f) || k.key_id.as_deref() == Some(f)
                        });
                        if !exists {
                            key_records.push(KeyRecord {
                                key_type: KeyType::Ssh,
                                key_id: Some(f.to_string()),
                                fingerprint: Some(f.to_string()),
                                raw_preview: format!("SSH Key: {f}"),
                                emails: Vec::new(),
                                created_at: None,
                                expires_at: None,
                                can_sign: Some(true),
                                source: "GitLeak Metadata".to_string(),
                            });
                        }
                    }
                }

                if let Some(heatmap) = &meta.activity_heatmap {
                    if let Some(peak) = &heatmap.peak_activity {
                        report.inferences.push(InferenceRecord {
                            topic: "Peak Public Activity Hour".to_string(),
                            inference: format!(
                                "Peak commit activity observed at {:02}:00 UTC ({} commits)",
                                peak.hour, peak.count
                            ),
                            confidence: "Moderate".to_string(),
                            basis: format!(
                                "Analyzed {} public commits in GitLeak activity heatmap",
                                heatmap.total_commits
                            ),
                            classification: Classification::Inferred,
                            limitation: "Activity hours vary due to travel, nocturnal work habits, or automated CI runs; this does not establish physical location or timezone of residence.".to_string(),
                        });
                    }
                }
            }
        }

        for commit in canonical_commits.values() {
            let repo = &commit.repo_full_name;
            let repo_is_owned = candidate_map.get(repo).map_or_else(
                || {
                    repo.split('/')
                        .next()
                        .is_some_and(|o| o.eq_ignore_ascii_case(username))
                },
                |c| c.owner.eq_ignore_ascii_case(username),
            );
            let is_verified = commit
                .detail
                .verification
                .as_ref()
                .is_some_and(|v| v.verified);
            let mut sources_list: Vec<String> = commit.sources.iter().cloned().collect();
            sources_list.sort();

            let author = commit.detail.author.as_ref();
            let committer = commit.detail.committer.as_ref();

            let author_email = author.map(|a| a.email.trim()).filter(|e| !e.is_empty());
            let committer_email = committer.map(|c| c.email.trim()).filter(|e| !e.is_empty());

            let is_author_target = author_email.is_some_and(|e| {
                is_target_actor(
                    e,
                    commit.author_user.as_ref(),
                    username,
                    Some(gh_user.id),
                    &target_anchors,
                )
            });

            let is_committer_target = committer_email.is_some_and(|e| {
                is_target_actor(
                    e,
                    commit.committer_user.as_ref(),
                    username,
                    Some(gh_user.id),
                    &target_anchors,
                )
            });

            match (author_email, committer_email) {
                (Some(a_email), Some(c_email)) if a_email.eq_ignore_ascii_case(c_email) => {
                    let a_norm = a_email.to_lowercase();
                    let a_actor = author.unwrap();
                    if is_author_target || is_committer_target {
                        let entry =
                            identities_map
                                .entry(a_norm)
                                .or_insert_with(|| IdentityRecord {
                                    name: Some(a_actor.name.clone()),
                                    email: a_actor.email.trim().to_string(),
                                    email_type: classify_email(
                                        &a_actor.email,
                                        profile_report.public_email.as_deref(),
                                    ),
                                    first_seen: a_actor.date,
                                    last_seen: a_actor.date,
                                    commits_observed: 0,
                                    repositories: Vec::new(),
                                    sources: Vec::new(),
                                    is_verified_signer: false,
                                });
                        entry.commits_observed += 1;
                        if is_verified {
                            entry.is_verified_signer = true;
                        }
                        if !entry.repositories.contains(repo) {
                            entry.repositories.push(repo.clone());
                        }
                        for s in &sources_list {
                            if !entry.sources.contains(s) {
                                entry.sources.push(s.clone());
                            }
                        }
                        if let Some(d) = a_actor.date {
                            if entry.first_seen.is_none_or(|existing| d < existing) {
                                entry.first_seen = Some(d);
                            }
                            if entry.last_seen.is_none_or(|existing| d > existing) {
                                entry.last_seen = Some(d);
                            }
                        }
                        report.provenance.push(
                            Provenance::new(
                                sources_list
                                    .first()
                                    .map(|s| s.as_str())
                                    .unwrap_or("Public Git Commit Metadata"),
                                Classification::Observed,
                            )
                            .with_source_url(&commit.html_url)
                            .with_repository(repo)
                            .with_commit(&commit.sha)
                            .with_notes(format!(
                                "Email observed as target author & committer: {}",
                                a_actor.email
                            )),
                        );
                    } else {
                        let entry = related_actors_map.entry(a_norm).or_insert_with(|| {
                            RelatedActorRecord {
                                name: Some(a_actor.name.clone()),
                                email: a_actor.email.trim().to_string(),
                                email_type: classify_email(&a_actor.email, None),
                                first_seen: a_actor.date,
                                last_seen: a_actor.date,
                                commits_observed: 0,
                                repositories: Vec::new(),
                                sources: Vec::new(),
                                relationship: if is_github_platform_system(&a_actor.email) {
                                    "platform_bot".to_string()
                                } else if repo_is_owned {
                                    "repository_contributor".to_string()
                                } else {
                                    "co_contributor".to_string()
                                },
                            }
                        });
                        entry.commits_observed += 1;
                        if !entry.repositories.contains(repo) {
                            entry.repositories.push(repo.clone());
                        }
                        for s in &sources_list {
                            if !entry.sources.contains(s) {
                                entry.sources.push(s.clone());
                            }
                        }
                        if let Some(d) = a_actor.date {
                            if entry.first_seen.is_none_or(|existing| d < existing) {
                                entry.first_seen = Some(d);
                            }
                            if entry.last_seen.is_none_or(|existing| d > existing) {
                                entry.last_seen = Some(d);
                            }
                        }
                    }
                }
                _ => {
                    if let Some(a_email) = author_email {
                        let a_norm = a_email.to_lowercase();
                        let a_actor = author.unwrap();
                        if is_author_target {
                            let entry =
                                identities_map
                                    .entry(a_norm)
                                    .or_insert_with(|| IdentityRecord {
                                        name: Some(a_actor.name.clone()),
                                        email: a_actor.email.trim().to_string(),
                                        email_type: classify_email(
                                            &a_actor.email,
                                            profile_report.public_email.as_deref(),
                                        ),
                                        first_seen: a_actor.date,
                                        last_seen: a_actor.date,
                                        commits_observed: 0,
                                        repositories: Vec::new(),
                                        sources: Vec::new(),
                                        is_verified_signer: false,
                                    });
                            entry.commits_observed += 1;
                            if is_verified {
                                entry.is_verified_signer = true;
                            }
                            if !entry.repositories.contains(repo) {
                                entry.repositories.push(repo.clone());
                            }
                            for s in &sources_list {
                                if !entry.sources.contains(s) {
                                    entry.sources.push(s.clone());
                                }
                            }
                            if let Some(d) = a_actor.date {
                                if entry.first_seen.is_none_or(|existing| d < existing) {
                                    entry.first_seen = Some(d);
                                }
                                if entry.last_seen.is_none_or(|existing| d > existing) {
                                    entry.last_seen = Some(d);
                                }
                            }
                            report.provenance.push(
                                Provenance::new(
                                    sources_list
                                        .first()
                                        .map(|s| s.as_str())
                                        .unwrap_or("Public Git Commit Metadata"),
                                    Classification::Observed,
                                )
                                .with_source_url(&commit.html_url)
                                .with_repository(repo)
                                .with_commit(&commit.sha)
                                .with_notes(format!(
                                    "Email observed as target author: {}",
                                    a_actor.email
                                )),
                            );
                        } else {
                            let entry = related_actors_map.entry(a_norm).or_insert_with(|| {
                                RelatedActorRecord {
                                    name: Some(a_actor.name.clone()),
                                    email: a_actor.email.trim().to_string(),
                                    email_type: classify_email(&a_actor.email, None),
                                    first_seen: a_actor.date,
                                    last_seen: a_actor.date,
                                    commits_observed: 0,
                                    repositories: Vec::new(),
                                    sources: Vec::new(),
                                    relationship: if is_github_platform_system(&a_actor.email) {
                                        "platform_bot".to_string()
                                    } else if is_committer_target {
                                        "patch_author".to_string()
                                    } else if repo_is_owned {
                                        "repository_contributor".to_string()
                                    } else {
                                        "co_contributor".to_string()
                                    },
                                }
                            });
                            entry.commits_observed += 1;
                            if !entry.repositories.contains(repo) {
                                entry.repositories.push(repo.clone());
                            }
                            for s in &sources_list {
                                if !entry.sources.contains(s) {
                                    entry.sources.push(s.clone());
                                }
                            }
                            if let Some(d) = a_actor.date {
                                if entry.first_seen.is_none_or(|existing| d < existing) {
                                    entry.first_seen = Some(d);
                                }
                                if entry.last_seen.is_none_or(|existing| d > existing) {
                                    entry.last_seen = Some(d);
                                }
                            }
                        }
                    }

                    if let Some(c_email) = committer_email {
                        let c_norm = c_email.to_lowercase();
                        let c_actor = committer.unwrap();
                        if is_committer_target {
                            let entry =
                                identities_map
                                    .entry(c_norm)
                                    .or_insert_with(|| IdentityRecord {
                                        name: Some(c_actor.name.clone()),
                                        email: c_actor.email.trim().to_string(),
                                        email_type: classify_email(
                                            &c_actor.email,
                                            profile_report.public_email.as_deref(),
                                        ),
                                        first_seen: c_actor.date,
                                        last_seen: c_actor.date,
                                        commits_observed: 0,
                                        repositories: Vec::new(),
                                        sources: Vec::new(),
                                        is_verified_signer: false,
                                    });
                            entry.commits_observed += 1;
                            if is_verified {
                                entry.is_verified_signer = true;
                            }
                            if !entry.repositories.contains(repo) {
                                entry.repositories.push(repo.clone());
                            }
                            for s in &sources_list {
                                if !entry.sources.contains(s) {
                                    entry.sources.push(s.clone());
                                }
                            }
                            if let Some(d) = c_actor.date {
                                if entry.first_seen.is_none_or(|existing| d < existing) {
                                    entry.first_seen = Some(d);
                                }
                                if entry.last_seen.is_none_or(|existing| d > existing) {
                                    entry.last_seen = Some(d);
                                }
                            }
                            report.provenance.push(
                                Provenance::new(
                                    sources_list
                                        .first()
                                        .map(|s| s.as_str())
                                        .unwrap_or("Public Git Commit Metadata"),
                                    Classification::Observed,
                                )
                                .with_source_url(&commit.html_url)
                                .with_repository(repo)
                                .with_commit(&commit.sha)
                                .with_notes(format!(
                                    "Email observed as target committer: {}",
                                    c_actor.email
                                )),
                            );
                        } else {
                            let entry = related_actors_map.entry(c_norm).or_insert_with(|| {
                                RelatedActorRecord {
                                    name: Some(c_actor.name.clone()),
                                    email: c_actor.email.trim().to_string(),
                                    email_type: classify_email(&c_actor.email, None),
                                    first_seen: c_actor.date,
                                    last_seen: c_actor.date,
                                    commits_observed: 0,
                                    repositories: Vec::new(),
                                    sources: Vec::new(),
                                    relationship: if is_github_platform_system(&c_actor.email) {
                                        "platform_bot".to_string()
                                    } else if is_author_target {
                                        "committer".to_string()
                                    } else if repo_is_owned {
                                        "repository_contributor".to_string()
                                    } else {
                                        "co_contributor".to_string()
                                    },
                                }
                            });
                            entry.commits_observed += 1;
                            if !entry.repositories.contains(repo) {
                                entry.repositories.push(repo.clone());
                            }
                            for s in &sources_list {
                                if !entry.sources.contains(s) {
                                    entry.sources.push(s.clone());
                                }
                            }
                            if let Some(d) = c_actor.date {
                                if entry.first_seen.is_none_or(|existing| d < existing) {
                                    entry.first_seen = Some(d);
                                }
                                if entry.last_seen.is_none_or(|existing| d > existing) {
                                    entry.last_seen = Some(d);
                                }
                            }
                        }
                    }
                }
            }
        }

        // GitLeak identities without observed commits default to 1 commit observation
        for id in identities_map.values_mut() {
            if id.commits_observed == 0 && id.sources.iter().any(|s| s == "GitLeak") {
                id.commits_observed = 1;
            }
            id.repositories.sort();
            id.repositories.dedup();
        }

        for actor in related_actors_map.values_mut() {
            actor.repositories.sort();
            actor.repositories.dedup();
        }

        let mut identities: Vec<IdentityRecord> = identities_map.into_values().collect();
        identities.sort_by_key(|b| std::cmp::Reverse(b.commits_observed));

        let mut related_commit_actors: Vec<RelatedActorRecord> =
            related_actors_map.into_values().collect();
        related_commit_actors.sort_by_key(|b| std::cmp::Reverse(b.commits_observed));

        let mut sorted_langs: Vec<(String, u64)> = language_counts.into_iter().collect();
        sorted_langs.sort_by_key(|b| std::cmp::Reverse(b.1));

        let total_repos_count = gh_repos.len();
        let fork_ratio = if total_repos_count > 0 {
            fork_count as f64 / total_repos_count as f64
        } else {
            0.0
        };
        let archived_ratio = if total_repos_count > 0 {
            archived_count as f64 / total_repos_count as f64
        } else {
            0.0
        };

        let owned_repositories_total =
            std::cmp::max(gh_user.public_repos as usize, owned_discovered);
        let owned_repositories_scanned = eligible_candidates
            .iter()
            .filter(|c| c.owner.eq_ignore_ascii_case(username))
            .count();
        let external_repositories_discovered = external_discovered;
        let external_repositories_scanned = eligible_candidates
            .iter()
            .filter(|c| !c.owner.eq_ignore_ascii_case(username))
            .count();
        let candidate_repositories_total = candidate_map.len();
        let candidate_repositories_scanned = eligible_candidates.len();

        let is_partial = (owned_repositories_scanned < owned_repositories_total)
            || (external_repositories_scanned < external_repositories_discovered)
            || candidates_skipped > 0
            || search_outcome.windows_partial > 0
            || search_outcome.stopped_due_to_budget
            || !graphql_years_fully_traversed;

        let notes = format!(
            "Scanned {owned_repositories_scanned} of {owned_repositories_total} owned repositories and {external_repositories_scanned} of {external_repositories_discovered} external repositories ({candidate_repositories_scanned} of {candidate_repositories_total} candidate repositories discovered) with a budget limit of {commit_per_repo_limit} commits per repo in {} mode",
            options.mode
        );

        report.coverage = CoverageRecord {
            commits_scanned: canonical_commits.len(),
            repositories_scanned: candidate_repositories_scanned,
            repositories_total: candidate_repositories_total,
            owned_repositories_total,
            owned_repositories_scanned,
            external_repositories_discovered,
            external_repositories_scanned,
            candidate_repositories_total,
            candidate_repositories_scanned,
            candidate_repositories_found: candidate_map.len(),
            global_commits_found: total_search_commits,
            git_history_inspected: git_inspected,
            is_partial,
            notes: Some(notes),
            discovery: Some(DiscoveryCoverage {
                owned_repositories_discovered: owned_discovered,
                external_repositories_discovered: external_discovered,
                candidates_selected,
                owned_selected,
                external_selected,
                candidates_skipped,
                repository_scans_attempted,
                repository_scans_succeeded,
                repository_scans_failed,
                global_author_commits_found: search_outcome.author_commits_count,
                global_committer_commits_found: search_outcome.committer_commits_count,
                commit_search_windows_complete: search_outcome.windows_complete,
                commit_search_windows_partial: search_outcome.windows_partial,
                git_history_attempted: git_attempted,
                git_history_succeeded: git_succeeded,
                git_history_failed: git_failed,
                git_history_timed_out: git_timed_out,
                graphql_years_fully_traversed,
                stopped_due_to_budget: search_outcome.stopped_due_to_budget
                    || candidates_skipped > 0,
            }),
        };

        report.summary = ReportSummary {
            display_name: profile_report
                .name
                .clone()
                .unwrap_or_else(|| profile_report.login.clone()),
            target_type: "GitHub User".to_string(),
            account_created: Some(profile_report.created_at),
            total_public_repositories: gh_user.public_repos as usize,
            total_stars_accumulated: total_stars,
            total_forks_accumulated: total_forks,
            primary_languages: sorted_langs,
            distinct_emails_observed: identities.len(),
            public_keys_found: key_records.len(),
            fork_ratio,
            archived_ratio,
        };

        report.profile = Some(profile_report);
        report.repositories = repo_reports;
        report.activity = activity_report;
        report.identities = identities;
        report.related_commit_actors = related_commit_actors;
        report.keys = key_records;

        report.recompute_summary();

        Ok(report)
    }

    async fn analyze_repo(
        &self,
        owner: &str,
        repo: &str,
        mode: AnalysisMode,
        event_tx: &Option<Sender<SpoorEvent>>,
    ) -> Result<Report, SpoorError> {
        let mut report = Report::new(
            Target::Repo {
                owner: owner.to_string(),
                repo: repo.to_string(),
            },
            mode,
        );

        Self::emit(
            event_tx,
            SpoorEvent::ProviderStarted {
                provider: "GitHub REST".to_string(),
                operation: format!("Fetching repository '{owner}/{repo}'"),
            },
        )
        .await;

        let gh_repo = match self.github.fetch_repository(owner, repo).await {
            Ok(r) => {
                report.provider_status.insert(
                    "GitHub Repository".to_string(),
                    ProviderStatus {
                        status: ProviderHealth::Ok,
                        latency_ms: None,
                        items_retrieved: Some(1),
                        rate_limit_remaining: Some(self.github.rest.rate_limit_remaining()),
                        rate_limit_reset: None,
                        retry_after_seconds: None,
                        message: None,
                    },
                );
                Self::emit(
                    event_tx,
                    SpoorEvent::ProviderCompleted {
                        provider: "GitHub REST".to_string(),
                        details: format!("Repository metadata retrieved for '{owner}/{repo}'"),
                    },
                )
                .await;
                r
            }
            Err(e) => {
                Self::emit(
                    event_tx,
                    SpoorEvent::LookupFailed {
                        error: e.to_string(),
                    },
                )
                .await;
                return Err(e);
            }
        };

        let languages = self
            .github
            .fetch_repository_languages(owner, repo)
            .await
            .unwrap_or_default();

        // Repository file tree for accurate hygiene detection
        let default_branch = gh_repo.default_branch.as_deref().unwrap_or("main");
        let tree = self
            .github
            .fetch_repository_tree(owner, repo, default_branch)
            .await
            .unwrap_or_default();
        let hygiene = if !tree.tree.is_empty() {
            detect_hygiene_from_paths(tree.tree.iter().map(|i| i.path.as_str()))
        } else {
            let community = self
                .github
                .fetch_repository_community_profile(owner, repo)
                .await
                .unwrap_or_default();
            let files = community.files.unwrap_or_default();
            RepoHygiene {
                has_readme: files.readme.is_some(),
                has_license: files.license.is_some(),
                has_security_policy: files.security.is_some(),
                has_contributing: files.contributing.is_some(),
                has_codeowners: false,
                has_actions_ci: false,
                has_dependabot: false,
            }
        };

        let commit_budget = match mode {
            AnalysisMode::Fast => 30,
            AnalysisMode::Deep => 100,
        };

        Self::emit(
            event_tx,
            SpoorEvent::ScanProgress {
                current: 1,
                total: 1,
                message: format!("Inspecting recent commits in {owner}/{repo}"),
            },
        )
        .await;

        let commits = self
            .github
            .fetch_repository_commits(owner, repo, commit_budget)
            .await
            .unwrap_or_default();

        let mut identities_map: HashMap<String, IdentityRecord> = HashMap::new();
        let repo_full_name = format!("{owner}/{repo}");

        for item in &commits {
            let author = item.commit.author.as_ref();
            let committer = item.commit.committer.as_ref();

            let is_verified = item
                .commit
                .verification
                .as_ref()
                .is_some_and(|v| v.verified);

            let author_email = author.map(|a| a.email.trim()).filter(|e| !e.is_empty());
            let committer_email = committer.map(|c| c.email.trim()).filter(|e| !e.is_empty());

            match (author_email, committer_email) {
                (Some(a_email), Some(c_email)) if a_email.eq_ignore_ascii_case(c_email) => {
                    let a_actor = author.unwrap();
                    let norm = a_email.to_lowercase();
                    let entry = identities_map
                        .entry(norm)
                        .or_insert_with(|| IdentityRecord {
                            name: Some(a_actor.name.clone()),
                            email: a_actor.email.trim().to_string(),
                            email_type: classify_email(&a_actor.email, None),
                            first_seen: a_actor.date,
                            last_seen: a_actor.date,
                            commits_observed: 0,
                            repositories: vec![repo_full_name.clone()],
                            sources: vec!["Public Git Commit Metadata".to_string()],
                            is_verified_signer: false,
                        });

                    entry.commits_observed += 1;
                    if is_verified {
                        entry.is_verified_signer = true;
                    }
                    if !entry.repositories.contains(&repo_full_name) {
                        entry.repositories.push(repo_full_name.clone());
                    }
                    if let Some(d) = a_actor.date {
                        if entry.first_seen.is_none_or(|existing| d < existing) {
                            entry.first_seen = Some(d);
                        }
                        if entry.last_seen.is_none_or(|existing| d > existing) {
                            entry.last_seen = Some(d);
                        }
                    }

                    report.provenance.push(
                        Provenance::new("Git Commit Metadata", Classification::Observed)
                            .with_source_url(&item.html_url)
                            .with_repository(&repo_full_name)
                            .with_commit(&item.sha)
                            .with_notes(format!(
                                "Email observed as author & committer: {}",
                                a_actor.email
                            )),
                    );
                }
                _ => {
                    if let Some(a_actor) = author {
                        let email_norm = a_actor.email.trim().to_lowercase();
                        if !email_norm.is_empty() {
                            let entry = identities_map.entry(email_norm).or_insert_with(|| {
                                IdentityRecord {
                                    name: Some(a_actor.name.clone()),
                                    email: a_actor.email.trim().to_string(),
                                    email_type: classify_email(&a_actor.email, None),
                                    first_seen: a_actor.date,
                                    last_seen: a_actor.date,
                                    commits_observed: 0,
                                    repositories: vec![repo_full_name.clone()],
                                    sources: vec!["Public Git Commit Metadata".to_string()],
                                    is_verified_signer: false,
                                }
                            });

                            entry.commits_observed += 1;
                            if is_verified {
                                entry.is_verified_signer = true;
                            }
                            if !entry.repositories.contains(&repo_full_name) {
                                entry.repositories.push(repo_full_name.clone());
                            }
                            if let Some(d) = a_actor.date {
                                if entry.first_seen.is_none_or(|existing| d < existing) {
                                    entry.first_seen = Some(d);
                                }
                                if entry.last_seen.is_none_or(|existing| d > existing) {
                                    entry.last_seen = Some(d);
                                }
                            }

                            report.provenance.push(
                                Provenance::new("Git Commit Metadata", Classification::Observed)
                                    .with_source_url(&item.html_url)
                                    .with_repository(&repo_full_name)
                                    .with_commit(&item.sha)
                                    .with_notes(format!(
                                        "Email observed as author: {}",
                                        a_actor.email
                                    )),
                            );
                        }
                    }

                    if let Some(c_actor) = committer {
                        let email_norm = c_actor.email.trim().to_lowercase();
                        if !email_norm.is_empty() {
                            let entry = identities_map.entry(email_norm).or_insert_with(|| {
                                IdentityRecord {
                                    name: Some(c_actor.name.clone()),
                                    email: c_actor.email.trim().to_string(),
                                    email_type: classify_email(&c_actor.email, None),
                                    first_seen: c_actor.date,
                                    last_seen: c_actor.date,
                                    commits_observed: 0,
                                    repositories: vec![repo_full_name.clone()],
                                    sources: vec!["Public Git Commit Metadata".to_string()],
                                    is_verified_signer: false,
                                }
                            });

                            entry.commits_observed += 1;
                            if is_verified {
                                entry.is_verified_signer = true;
                            }
                            if !entry.repositories.contains(&repo_full_name) {
                                entry.repositories.push(repo_full_name.clone());
                            }
                            if let Some(d) = c_actor.date {
                                if entry.first_seen.is_none_or(|existing| d < existing) {
                                    entry.first_seen = Some(d);
                                }
                                if entry.last_seen.is_none_or(|existing| d > existing) {
                                    entry.last_seen = Some(d);
                                }
                            }

                            report.provenance.push(
                                Provenance::new("Git Commit Metadata", Classification::Observed)
                                    .with_source_url(&item.html_url)
                                    .with_repository(&repo_full_name)
                                    .with_commit(&item.sha)
                                    .with_notes(format!(
                                        "Email observed as committer: {}",
                                        c_actor.email
                                    )),
                            );
                        }
                    }
                }
            }
        }

        for id in identities_map.values_mut() {
            id.repositories.sort();
            id.repositories.dedup();
        }

        let repo_report = RepoReport {
            name: gh_repo.name.clone(),
            full_name: gh_repo.full_name.clone(),
            description: gh_repo.description.clone(),
            is_fork: gh_repo.fork,
            is_archived: gh_repo.archived.unwrap_or(false),
            default_branch: gh_repo
                .default_branch
                .clone()
                .unwrap_or_else(|| "main".into()),
            stars: gh_repo.stargazers_count,
            forks: gh_repo.forks_count,
            open_issues: gh_repo.open_issues_count,
            size_kb: gh_repo.size,
            primary_language: gh_repo.language.clone(),
            languages: Some(languages.clone()),
            created_at: gh_repo.created_at,
            updated_at: gh_repo.updated_at,
            pushed_at: gh_repo.pushed_at,
            topics: gh_repo.topics.clone().unwrap_or_default(),
            license_spdx: gh_repo.license.as_ref().and_then(|l| l.spdx_id.clone()),
            hygiene: Some(hygiene),
            html_url: gh_repo.html_url.clone(),
        };

        let mut sorted_langs: Vec<(String, u64)> = languages.into_iter().collect();
        sorted_langs.sort_by_key(|b| std::cmp::Reverse(b.1));

        let mut identities: Vec<IdentityRecord> = identities_map.into_values().collect();
        identities.sort_by_key(|b| std::cmp::Reverse(b.commits_observed));

        report.summary = ReportSummary {
            display_name: gh_repo.full_name.clone(),
            target_type: "GitHub Repository".to_string(),
            account_created: gh_repo.created_at,
            total_public_repositories: 1,
            total_stars_accumulated: gh_repo.stargazers_count,
            total_forks_accumulated: gh_repo.forks_count,
            primary_languages: sorted_langs,
            distinct_emails_observed: identities.len(),
            public_keys_found: 0,
            fork_ratio: if gh_repo.fork { 1.0 } else { 0.0 },
            archived_ratio: if gh_repo.archived.unwrap_or(false) {
                1.0
            } else {
                0.0
            },
        };

        report.coverage = CoverageRecord {
            commits_scanned: commits.len(),
            repositories_scanned: 1,
            repositories_total: 1,
            owned_repositories_total: 1,
            owned_repositories_scanned: 1,
            external_repositories_discovered: 0,
            external_repositories_scanned: 0,
            candidate_repositories_total: 1,
            candidate_repositories_scanned: 1,
            candidate_repositories_found: 1,
            global_commits_found: 0,
            git_history_inspected: false,
            discovery: None,
            is_partial: commits.len() >= commit_budget,
            notes: Some(format!(
                "Scanned {} commits from target repository in {mode} mode",
                commits.len()
            )),
        };

        report.repositories = vec![repo_report];
        report.identities = identities;

        // GitLeak is not queried for repository targets per requirements
        report.provider_status.insert(
            "GitLeak".to_string(),
            ProviderStatus {
                status: ProviderHealth::Skipped,
                latency_ms: None,
                items_retrieved: None,
                rate_limit_remaining: None,
                rate_limit_reset: None,
                retry_after_seconds: None,
                message: Some("GitLeak enrichment is scoped to user targets only".to_string()),
            },
        );

        report.recompute_summary();

        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_target_noreply_rules() {
        let username = "alice";
        let user_id = Some(1001);

        // Legacy form
        assert!(is_target_noreply(
            "alice@users.noreply.github.com",
            username,
            user_id
        ));
        assert!(is_target_noreply(
            "ALICE@users.noreply.github.com",
            username,
            user_id
        ));
        assert!(is_target_noreply(
            "alice@users.noreply.github.com",
            username,
            None
        ));

        // Modern form with exact matching ID
        assert!(is_target_noreply(
            "1001+alice@users.noreply.github.com",
            username,
            user_id
        ));
        assert!(is_target_noreply(
            "1001+ALICE@users.noreply.github.com",
            username,
            user_id
        ));

        // Negative regression: arbitrary prefix with known ID must NOT match
        assert!(!is_target_noreply(
            "999999+alice@users.noreply.github.com",
            username,
            user_id
        ));
        assert!(!is_target_noreply(
            "random+alice@users.noreply.github.com",
            username,
            user_id
        ));

        // Negative regression: modern form with unknown target ID must NOT match arbitrary prefix
        assert!(!is_target_noreply(
            "999999+alice@users.noreply.github.com",
            username,
            None
        ));

        // Negative: mismatched username
        assert!(!is_target_noreply(
            "bob@users.noreply.github.com",
            username,
            user_id
        ));
        assert!(!is_target_noreply(
            "1001+bob@users.noreply.github.com",
            username,
            user_id
        ));
    }

    #[test]
    fn test_is_target_actor_id_verification_and_no_weak_heuristics() {
        let target_username = "alice";
        let target_id = Some(1001);
        let target_anchors = HashSet::new();

        // Exact login and exact matching ID
        let valid_user = GhSimpleUser {
            login: "alice".to_string(),
            id: 1001,
        };
        assert!(is_target_actor(
            "custom@domain.com",
            Some(&valid_user),
            target_username,
            target_id,
            &target_anchors
        ));

        // Negative regression: exact login but mismatched ID (e.g. 9999 vs 1001) MUST NOT match
        let mismatched_user = GhSimpleUser {
            login: "alice".to_string(),
            id: 9999,
        };
        assert!(!is_target_actor(
            "custom@domain.com",
            Some(&mismatched_user),
            target_username,
            target_id,
            &target_anchors
        ));

        // Negative regression: weak name/local-part heuristic rejected
        assert!(!is_target_actor(
            "alice@thirdparty.com",
            None,
            target_username,
            target_id,
            &target_anchors
        ));
        assert!(!is_target_actor(
            "alice+sub@thirdparty.com",
            None,
            target_username,
            target_id,
            &target_anchors
        ));

        // Negative: platform bots rejected
        assert!(!is_target_actor(
            "noreply@github.com",
            Some(&valid_user),
            target_username,
            target_id,
            &target_anchors
        ));
    }
}
