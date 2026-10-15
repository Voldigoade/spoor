use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use chrono::Utc;
use spoor_core::{
    classify_email, ActivityEventSummary, ActivityReport, AnalysisMode, AnalysisOptions,
    CacheStore, Classification, CoverageRecord, IdentityRecord, InferenceRecord, KeyRecord,
    KeyType, ProfileReport, Provenance, ProviderHealth, ProviderStatus, RepoHygiene, RepoReport,
    Report, ReportSummary, SpoorError, SpoorEvent, Target,
};
use tokio::sync::mpsc::Sender;

use crate::github::models::{GhCommitItem, GhRepo};
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

type ScannedRepoMeta = (Option<HashMap<String, u64>>, Option<RepoHygiene>);

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

        // Check full report cache (key differentiates analysis depth and enrichment)
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
                // Mark provider statuses as cached where appropriate
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

        // Cache completed report (FAST for 1 hour, DEEP for 6 hours)
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

        Self::emit(
            event_tx,
            SpoorEvent::ProviderStarted {
                provider: "GitHub REST".to_string(),
                operation: format!("Fetching public repositories (up to {max_repos})"),
            },
        )
        .await;

        let gh_repos = self
            .github
            .fetch_user_repositories(username, max_repos)
            .await
            .unwrap_or_default();

        Self::emit(
            event_tx,
            SpoorEvent::ProviderCompleted {
                provider: "GitHub REST".to_string(),
                details: format!("Retrieved {} repositories", gh_repos.len()),
            },
        )
        .await;

        report.provider_status.insert(
            "GitHub Repositories".to_string(),
            ProviderStatus {
                status: ProviderHealth::Ok,
                latency_ms: None,
                items_retrieved: Some(gh_repos.len()),
                rate_limit_remaining: Some(self.github.rest.rate_limit_remaining()),
                rate_limit_reset: None,
                retry_after_seconds: None,
                message: None,
            },
        );

        let (repo_scan_limit, commit_per_repo_limit) = match options.mode {
            AnalysisMode::Fast => (3, 30),
            AnalysisMode::Deep => (8, 50),
        };

        // Filter non-fork repositories first for commit and hygiene analysis
        let mut eligible_repos: Vec<&GhRepo> = gh_repos.iter().filter(|r| !r.fork).collect();
        if eligible_repos.is_empty() {
            eligible_repos = gh_repos.iter().collect();
        }
        eligible_repos.truncate(repo_scan_limit);

        let mut raw_commits: Vec<(String, GhCommitItem)> = Vec::new();
        let mut repos_scanned_count = 0;
        let mut scanned_repo_meta: HashMap<String, ScannedRepoMeta> = HashMap::new();

        for (idx, r) in eligible_repos.iter().enumerate() {
            Self::emit(
                event_tx,
                SpoorEvent::ScanProgress {
                    current: idx + 1,
                    total: eligible_repos.len(),
                    message: format!("Inspecting repository {}", r.full_name),
                },
            )
            .await;

            let commits = self
                .github
                .fetch_repository_commits(username, &r.name, commit_per_repo_limit)
                .await
                .unwrap_or_default();

            if !commits.is_empty() {
                repos_scanned_count += 1;
                for c in commits {
                    raw_commits.push((r.full_name.clone(), c));
                }
            }

            let langs = self
                .github
                .fetch_repository_languages(username, &r.name)
                .await
                .ok();

            let default_branch = r.default_branch.as_deref().unwrap_or("main");
            let tree = self
                .github
                .fetch_repository_tree(username, &r.name, default_branch)
                .await
                .unwrap_or_default();

            let hygiene = if !tree.tree.is_empty() {
                Some(detect_hygiene_from_paths(
                    tree.tree.iter().map(|item| item.path.as_str()),
                ))
            } else {
                None
            };

            scanned_repo_meta.insert(r.full_name.clone(), (langs, hygiene));
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

        let event_result = self.github.fetch_user_events(username, 300).await;
        let (events, event_warning, event_health) = match event_result {
            Ok(batch) => {
                let health = if batch.is_complete {
                    ProviderHealth::Ok
                } else {
                    ProviderHealth::Partial
                };
                (batch.events, batch.warning, health)
            }
            Err(error) => (
                Vec::new(),
                Some(format!("Public events request failed: {error}")),
                ProviderHealth::Unavailable,
            ),
        };
        let base_event_note = "GitHub public events cover only the previous 30 days, up to 300 events. Spoor requests up to three pages of 100 and filters older results.";
        let window_note = event_warning
            .as_ref()
            .map_or_else(|| base_event_note.to_string(), |warning| {
                format!("{base_event_note} {warning}")
            });
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

        // 7. Identity Aggregation and Deduplication
        let mut identities_map: HashMap<String, IdentityRecord> = HashMap::new();

        // A. Profile email
        if let Some(pub_email) = &profile_report.public_email {
            let email_clean = pub_email.trim().to_string();
            let email_type = classify_email(&email_clean, Some(&email_clean));
            identities_map.insert(
                email_clean.to_lowercase(),
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

        // B. Commit metadata identities
        for (repo_full_name, item) in &raw_commits {
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
                    // Author and committer are the same identity: count commit once, emit single provenance
                    let a_actor = author.unwrap();
                    let norm = a_email.to_lowercase();
                    let entry = identities_map
                        .entry(norm)
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
                            sources: vec!["Public Git Commit Metadata".to_string()],
                            is_verified_signer: false,
                        });

                    entry.commits_observed += 1;
                    if is_verified {
                        entry.is_verified_signer = true;
                    }
                    if !entry.repositories.contains(repo_full_name) {
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
                            .with_repository(repo_full_name)
                            .with_commit(&item.sha)
                            .with_notes(format!(
                                "Email observed as author & committer: {}",
                                a_actor.email
                            )),
                    );
                }
                _ => {
                    // Process author and committer individually if distinct
                    if let Some(a_actor) = author {
                        let email_norm = a_actor.email.trim().to_lowercase();
                        if !email_norm.is_empty() {
                            let entry = identities_map.entry(email_norm).or_insert_with(|| {
                                IdentityRecord {
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
                                    sources: vec!["Public Git Commit Metadata".to_string()],
                                    is_verified_signer: false,
                                }
                            });

                            entry.commits_observed += 1;
                            if is_verified {
                                entry.is_verified_signer = true;
                            }
                            if !entry.repositories.contains(repo_full_name) {
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
                                    .with_repository(repo_full_name)
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
                                    email_type: classify_email(
                                        &c_actor.email,
                                        profile_report.public_email.as_deref(),
                                    ),
                                    first_seen: c_actor.date,
                                    last_seen: c_actor.date,
                                    commits_observed: 0,
                                    repositories: Vec::new(),
                                    sources: vec!["Public Git Commit Metadata".to_string()],
                                    is_verified_signer: false,
                                }
                            });

                            entry.commits_observed += 1;
                            if is_verified {
                                entry.is_verified_signer = true;
                            }
                            if !entry.repositories.contains(repo_full_name) {
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
                                    .with_repository(repo_full_name)
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

        // C. GitLeak identities
        if let Some(gl) = &gitleak_data {
            for email_entry in &gl.emails {
                let norm = email_entry.email.trim().to_lowercase();
                if norm.is_empty() {
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

                if entry.commits_observed == 0 {
                    entry.commits_observed = 1;
                }

                if let Some(repo) = &extracted_repo {
                    if !entry.repositories.contains(repo) {
                        entry.repositories.push(repo.clone());
                    }
                }

                if let Some(commit_url) = &email_entry.commit_url {
                    let mut prov = Provenance::new("GitLeak", Classification::Observed)
                        .with_source_url(commit_url)
                        .with_notes(format!(
                            "Email observed via GitLeak commit: {}",
                            email_entry.email
                        ));
                    if let Some(repo) = &extracted_repo {
                        prov = prov.with_repository(repo);
                    }
                    report.provenance.push(prov);
                }
            }

            // Reconcile GitLeak GPG & SSH keys
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
            }

            // D. Timezone & Activity Heatmap Inferences
            if let Some(meta) = &gl.metadata {
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

        for id in identities_map.values_mut() {
            id.repositories.sort();
            id.repositories.dedup();
        }

        // Collect sorted identities
        let mut identities: Vec<IdentityRecord> = identities_map.into_values().collect();
        identities.sort_by_key(|b| std::cmp::Reverse(b.commits_observed));

        // Top languages
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

        // Coverage accounting
        let is_partial = (repos_scanned_count < total_repos_count)
            || (gh_user.public_repos as usize > max_repos);
        report.coverage = CoverageRecord {
            commits_scanned: raw_commits.len(),
            repositories_scanned: repos_scanned_count,
            repositories_total: gh_user.public_repos as usize,
            is_partial,
            notes: Some(format!(
                "Scanned {repos_scanned_count} of {} public repositories with a budget limit of {commit_per_repo_limit} commits per repo in {} mode",
                gh_user.public_repos, options.mode
            )),
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
