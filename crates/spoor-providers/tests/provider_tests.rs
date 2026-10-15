use std::sync::Arc;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use spoor_core::email::EmailType;
use spoor_core::{AnalysisMode, AnalysisOptions, NoopCacheStore, ProviderHealth, Target};
use spoor_providers::{
    AnalysisEngine, GitHubProvider, GitLeakBudget, GitLeakClient, GitLeakOutcome, HttpClient,
};

#[tokio::test]
async fn test_github_rest_user_mock() {
    let mock_server = MockServer::start().await;

    let user_json = r#"{
        "login": "octocat",
        "id": 583231,
        "avatar_url": "https://avatars.githubusercontent.com/u/583231",
        "html_url": "https://github.com/octocat",
        "name": "The Octocat",
        "company": "@github",
        "blog": "https://github.blog",
        "location": "San Francisco",
        "email": "octocat@github.com",
        "bio": null,
        "twitter_username": null,
        "public_repos": 8,
        "public_gists": 8,
        "followers": 1000,
        "following": 9,
        "created_at": "2011-01-25T18:44:36Z",
        "updated_at": "2026-01-25T18:44:36Z"
    }"#;

    Mock::given(method("GET"))
        .and(path("/users/octocat"))
        .respond_with(ResponseTemplate::new(200).set_body_string(user_json))
        .mount(&mock_server)
        .await;

    let http = HttpClient::new();
    let provider = GitHubProvider::new(http.clone(), None, Some(mock_server.uri()), None);

    let user = provider.fetch_user_profile("octocat").await.unwrap();
    assert_eq!(user.login, "octocat");
    assert_eq!(user.id, 583231);
    assert_eq!(user.name.as_deref(), Some("The Octocat"));
}

#[tokio::test]
async fn test_gitleak_success_and_rate_limit_mock() {
    let mock_server = MockServer::start().await;

    let gitleak_json = r#"{
        "emails": [
            {
                "email": "octocat@github.com",
                "committer": "The Octocat",
                "commitUrl": "https://github.com/octocat/Hello-World/commit/abc"
            }
        ],
        "metadata": {
            "totalCommitsProcessed": 42,
            "emailsFound": 1,
            "executionTimeMs": 350
        }
    }"#;

    Mock::given(method("GET"))
        .and(path("/api/v1/users/octocat"))
        .respond_with(ResponseTemplate::new(200).set_body_string(gitleak_json))
        .mount(&mock_server)
        .await;

    let http = HttpClient::new();
    let gitleak = GitLeakClient::new(http.clone(), Some(mock_server.uri()));

    let res = gitleak.lookup("octocat", false, false).await.unwrap();
    match res {
        GitLeakOutcome::Success(resp) => {
            assert_eq!(resp.emails.len(), 1);
            assert_eq!(resp.emails[0].email, "octocat@github.com");
        }
        other => panic!("Expected Success, got {:?}", other),
    }

    // Now test 429 rate limit
    let mock_server2 = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/users/rate_limited_user"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("retry-after", "120")
                .set_body_string(r#"{"message":"rate limit exceeded"}"#),
        )
        .mount(&mock_server2)
        .await;

    let gitleak2 = GitLeakClient::new(http, Some(mock_server2.uri()));
    let res2 = gitleak2
        .lookup("rate_limited_user", false, false)
        .await
        .unwrap();
    match res2 {
        GitLeakOutcome::RateLimited { retry_after, .. } => {
            assert_eq!(retry_after, Some(120));
        }
        other => panic!("Expected RateLimited, got {:?}", other),
    }
}

#[tokio::test]
async fn test_engine_user_fast_flow() {
    let gh_mock = MockServer::start().await;
    let gl_mock = MockServer::start().await;

    let user_json = r#"{
        "login": "alice",
        "id": 1234,
        "avatar_url": "https://avatars.githubusercontent.com/u/1234",
        "html_url": "https://github.com/alice",
        "name": "Alice Developer",
        "created_at": "2020-01-01T00:00:00Z",
        "updated_at": "2026-01-01T00:00:00Z",
        "public_repos": 2
    }"#;

    let repos_json = r#"[
        {
            "name": "cool-project",
            "full_name": "alice/cool-project",
            "html_url": "https://github.com/alice/cool-project",
            "fork": false,
            "stargazers_count": 50,
            "forks_count": 5,
            "size": 100,
            "language": "Rust",
            "open_issues_count": 2
        }
    ]"#;

    let commits_json = r#"[
        {
            "sha": "1234567890abcdef",
            "html_url": "https://github.com/alice/cool-project/commit/1234567890abcdef",
            "commit": {
                "message": "Initial commit",
                "author": {
                    "name": "Alice",
                    "email": "alice@customdomain.io",
                    "date": "2026-01-10T12:00:00Z"
                },
                "committer": {
                    "name": "Alice",
                    "email": "alice@customdomain.io",
                    "date": "2026-01-10T12:00:00Z"
                },
                "verification": {
                    "verified": true,
                    "reason": "valid"
                }
            }
        }
    ]"#;

    Mock::given(method("GET"))
        .and(path("/users/alice"))
        .respond_with(ResponseTemplate::new(200).set_body_string(user_json))
        .mount(&gh_mock)
        .await;

    Mock::given(method("GET"))
        .and(path("/users/alice/repos"))
        .respond_with(ResponseTemplate::new(200).set_body_string(repos_json))
        .mount(&gh_mock)
        .await;

    Mock::given(method("GET"))
        .and(path("/repos/alice/cool-project/commits"))
        .respond_with(ResponseTemplate::new(200).set_body_string(commits_json))
        .mount(&gh_mock)
        .await;

    Mock::given(method("GET"))
        .and(path("/users/alice/events/public"))
        .respond_with(ResponseTemplate::new(200).set_body_string("[]"))
        .mount(&gh_mock)
        .await;

    Mock::given(method("GET"))
        .and(path("/users/alice/gpg_keys"))
        .respond_with(ResponseTemplate::new(200).set_body_string("[]"))
        .mount(&gh_mock)
        .await;

    Mock::given(method("GET"))
        .and(path("/users/alice/keys"))
        .respond_with(ResponseTemplate::new(200).set_body_string("[]"))
        .mount(&gh_mock)
        .await;

    let http = HttpClient::new();
    let gh_provider = GitHubProvider::new(http.clone(), None, Some(gh_mock.uri()), None);
    let gl_client = GitLeakClient::new(http, Some(gl_mock.uri()));
    let cache = Arc::new(NoopCacheStore);

    let engine = AnalysisEngine::new(gh_provider, gl_client, cache);

    let target = Target::parse("alice").unwrap();
    let report = engine
        .run_analysis(&target, AnalysisMode::Fast, None)
        .await
        .unwrap();

    assert_eq!(report.summary.display_name, "Alice Developer");
    assert_eq!(report.summary.total_stars_accumulated, 50);
    assert_eq!(report.identities.len(), 1);
    assert_eq!(report.identities[0].email, "alice@customdomain.io");
    assert!(report.identities[0].is_verified_signer);
    assert_eq!(report.repositories.len(), 1);
    assert_eq!(report.repositories[0].name, "cool-project");
}

#[tokio::test]
async fn test_engine_user_deep_flow_with_gitleak_and_repo_hygiene() {
    let gh_mock = MockServer::start().await;
    let gl_mock = MockServer::start().await;

    let user_json = r#"{
        "login": "bob",
        "id": 5678,
        "avatar_url": "https://avatars.githubusercontent.com/u/5678",
        "html_url": "https://github.com/bob",
        "name": "Bob Builder",
        "created_at": "2019-03-01T00:00:00Z",
        "updated_at": "2026-03-01T00:00:00Z",
        "public_repos": 1
    }"#;

    let repos_json = r#"[
        {
            "name": "secure-app",
            "full_name": "bob/secure-app",
            "html_url": "https://github.com/bob/secure-app",
            "default_branch": "main",
            "fork": false,
            "stargazers_count": 120,
            "forks_count": 12,
            "size": 500,
            "language": "Rust",
            "open_issues_count": 0
        }
    ]"#;

    let tree_json = r#"{
        "sha": "tree123",
        "tree": [
            { "path": "README.md", "mode": "100644", "type": "blob", "sha": "abc1" },
            { "path": "SECURITY.md", "mode": "100644", "type": "blob", "sha": "abc2" },
            { "path": ".github/workflows/ci.yml", "mode": "100644", "type": "blob", "sha": "abc3" }
        ],
        "truncated": false
    }"#;

    // Two commits:
    // Commit 1: author == committer (bob@company.com)
    // Commit 2: automated bot (noreply@github.com)
    let commits_json = r#"[
        {
            "sha": "commit_sha_1",
            "html_url": "https://github.com/bob/secure-app/commit/commit_sha_1",
            "commit": {
                "message": "Feat: implement secure auth",
                "author": {
                    "name": "Bob",
                    "email": "bob@company.com",
                    "date": "2026-02-01T10:00:00Z"
                },
                "committer": {
                    "name": "Bob",
                    "email": "bob@company.com",
                    "date": "2026-02-01T10:00:00Z"
                },
                "verification": {
                    "verified": true,
                    "reason": "valid"
                }
            }
        },
        {
            "sha": "commit_sha_2",
            "html_url": "https://github.com/bob/secure-app/commit/commit_sha_2",
            "commit": {
                "message": "Merge pull request #1",
                "author": {
                    "name": "GitHub Web Flow",
                    "email": "noreply@github.com",
                    "date": "2026-02-02T10:00:00Z"
                },
                "committer": {
                    "name": "GitHub Web Flow",
                    "email": "noreply@github.com",
                    "date": "2026-02-02T10:00:00Z"
                },
                "verification": {
                    "verified": true,
                    "reason": "valid"
                }
            }
        }
    ]"#;

    // GitLeak response returning an external commit with repository shorthand:
    let gitleak_json = r#"{
        "emails": [
            {
                "email": "user@example.com",
                "committer": "ExampleUser",
                "commitUrl": "https://github.com/example-user/example-repo/commit/caf051bdcbba3caed40a5e60ba861033662dd4ce"
            }
        ],
        "metadata": {
            "totalCommitsProcessed": 100,
            "emailsFound": 1,
            "filteredOutEmails": ["123456+example-user@users.noreply.github.com"],
            "executionTimeMs": 240,
            "discoveredUsernames": ["bob", "ExampleUser"],
            "gpgKeys": [
                {
                    "keyId": "GPG_KEY_FROM_GITLEAK"
                }
            ],
            "sshKeys": [
                {
                    "fingerprint": "SHA256:SSH_KEY_FROM_GITLEAK"
                }
            ]
        }
    }"#;

    Mock::given(method("GET"))
        .and(path("/users/bob"))
        .respond_with(ResponseTemplate::new(200).set_body_string(user_json))
        .mount(&gh_mock)
        .await;

    Mock::given(method("GET"))
        .and(path("/users/bob/repos"))
        .respond_with(ResponseTemplate::new(200).set_body_string(repos_json))
        .mount(&gh_mock)
        .await;

    Mock::given(method("GET"))
        .and(path("/repos/bob/secure-app/commits"))
        .respond_with(ResponseTemplate::new(200).set_body_string(commits_json))
        .mount(&gh_mock)
        .await;

    Mock::given(method("GET"))
        .and(path("/repos/bob/secure-app/git/trees/main"))
        .respond_with(ResponseTemplate::new(200).set_body_string(tree_json))
        .mount(&gh_mock)
        .await;

    Mock::given(method("GET"))
        .and(path("/repos/bob/secure-app/languages"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"Rust": 50000}"#))
        .mount(&gh_mock)
        .await;

    Mock::given(method("GET"))
        .and(path("/users/bob/events/public"))
        .respond_with(ResponseTemplate::new(200).set_body_string("[]"))
        .mount(&gh_mock)
        .await;

    Mock::given(method("GET"))
        .and(path("/users/bob/gpg_keys"))
        .respond_with(ResponseTemplate::new(200).set_body_string("[]"))
        .mount(&gh_mock)
        .await;

    Mock::given(method("GET"))
        .and(path("/users/bob/keys"))
        .respond_with(ResponseTemplate::new(200).set_body_string("[]"))
        .mount(&gh_mock)
        .await;

    Mock::given(method("GET"))
        .and(path("/api/v1/users/bob"))
        .respond_with(ResponseTemplate::new(200).set_body_string(gitleak_json))
        .mount(&gl_mock)
        .await;

    let http = HttpClient::new();
    let gh_provider = GitHubProvider::new(http.clone(), None, Some(gh_mock.uri()), None);
    let gl_client = GitLeakClient::new(http, Some(gl_mock.uri()));
    let cache = Arc::new(NoopCacheStore);

    let engine = AnalysisEngine::new(gh_provider, gl_client, cache);

    let target = Target::parse("bob").unwrap();
    let report = engine
        .run_analysis(&target, AnalysisOptions::deep_enriched(), None)
        .await
        .unwrap();

    assert_eq!(report.repositories.len(), 1);
    let repo = &report.repositories[0];
    let hygiene = repo
        .hygiene
        .as_ref()
        .expect("Hygiene must be populated for scanned repo");
    assert!(hygiene.has_readme, "Must detect README.md");
    assert!(hygiene.has_security_policy, "Must detect SECURITY.md");
    assert!(
        hygiene.has_actions_ci,
        "Must detect .github/workflows/ci.yml"
    );
    assert!(
        !hygiene.has_dependabot,
        "Should not detect dependabot when absent"
    );

    let langs = repo
        .languages
        .as_ref()
        .expect("Languages must be populated for scanned repo");
    assert_eq!(langs.get("Rust"), Some(&50000));

    // We expect: bob@company.com, noreply@github.com, and user@example.com
    assert_eq!(report.identities.len(), 3);

    // bob@company.com: author == committer, commits_observed must be 1 (NOT 2)
    let bob_id = report
        .identities
        .iter()
        .find(|i| i.email == "bob@company.com")
        .unwrap();
    assert_eq!(
        bob_id.commits_observed, 1,
        "Author == committer must not double-count commits"
    );
    assert_eq!(bob_id.repositories, vec!["bob/secure-app"]);
    assert_eq!(bob_id.email_type, EmailType::DomainEmail);

    // noreply@github.com: must be classified as GitHubSystem
    let sys_id = report
        .identities
        .iter()
        .find(|i| i.email == "noreply@github.com")
        .unwrap();
    assert_eq!(sys_id.email_type, EmailType::GitHubSystem);
    assert!(sys_id.email_type.is_platform_system());

    // user@example.com: must preserve GitLeak finding, committer name, and extracted repo
    let gl_id = report
        .identities
        .iter()
        .find(|i| i.email == "user@example.com")
        .unwrap();
    assert_eq!(gl_id.name.as_deref(), Some("ExampleUser"));
    assert_eq!(gl_id.repositories, vec!["example-user/example-repo"]);
    assert!(gl_id.sources.contains(&"GitLeak".to_string()));
    assert_eq!(gl_id.commits_observed, 1);

    // 4. Verify Summary Metrics: distinct_emails_observed must EXCLUDE platform system bots
    // Here: bob@company.com and user@example.com = 2 distinct emails
    assert_eq!(report.summary.distinct_emails_observed, 2);

    // 5. Verify Public Keys Reconciled from GitLeak
    assert_eq!(report.keys.len(), 2);
    assert_eq!(report.summary.public_keys_found, 2);
    let gpg = report
        .keys
        .iter()
        .find(|k| k.key_id.as_deref() == Some("GPG_KEY_FROM_GITLEAK"))
        .unwrap();
    assert_eq!(gpg.source, "GitLeak Metadata");
    let ssh = report
        .keys
        .iter()
        .find(|k| k.fingerprint.as_deref() == Some("SHA256:SSH_KEY_FROM_GITLEAK"))
        .unwrap();
    assert_eq!(ssh.source, "GitLeak Metadata");

    let gl_prov = report
        .provenance
        .iter()
        .find(|p| p.provider == "GitLeak")
        .unwrap();
    assert_eq!(
        gl_prov.source_url.as_deref(),
        Some("https://github.com/example-user/example-repo/commit/caf051bdcbba3caed40a5e60ba861033662dd4ce")
    );
    assert_eq!(
        gl_prov.repository.as_deref(),
        Some("example-user/example-repo")
    );
}

#[test]
fn test_recompute_summary_mathematical_consistency() {
    use spoor_core::{IdentityRecord, KeyRecord, KeyType, RepoHygiene, RepoReport, Report};

    let mut report = Report::new(Target::parse("sample-user").unwrap(), AnalysisMode::Fast);

    report.repositories.push(RepoReport {
        name: "repo1".into(),
        full_name: "sample-user/repo1".into(),
        description: None,
        is_fork: false,
        is_archived: false,
        default_branch: "main".into(),
        stars: 100,
        forks: 10,
        open_issues: 0,
        size_kb: 50,
        primary_language: Some("Rust".into()),
        languages: None,
        created_at: Some(chrono::Utc::now()),
        updated_at: Some(chrono::Utc::now()),
        pushed_at: Some(chrono::Utc::now()),
        topics: vec![],
        license_spdx: Some("MIT".into()),
        hygiene: Some(RepoHygiene::default()),
        html_url: "https://github.com/sample-user/repo1".into(),
    });

    report.repositories.push(RepoReport {
        name: "repo2".into(),
        full_name: "sample-user/repo2".into(),
        description: None,
        is_fork: true,
        is_archived: true,
        default_branch: "main".into(),
        stars: 50,
        forks: 5,
        open_issues: 0,
        size_kb: 20,
        primary_language: Some("Rust".into()),
        languages: None,
        created_at: Some(chrono::Utc::now()),
        updated_at: Some(chrono::Utc::now()),
        pushed_at: Some(chrono::Utc::now()),
        topics: vec![],
        license_spdx: None,
        hygiene: None,
        html_url: "https://github.com/sample-user/repo2".into(),
    });

    report.identities.push(IdentityRecord {
        name: Some("Human Dev".into()),
        email: "dev@example.com".into(),
        email_type: EmailType::DomainEmail,
        first_seen: None,
        last_seen: None,
        commits_observed: 5,
        repositories: vec!["sample-user/repo1".into()],
        sources: vec!["Commit".into()],
        is_verified_signer: true,
    });

    report.identities.push(IdentityRecord {
        name: Some("GitHub Bot".into()),
        email: "noreply@github.com".into(),
        email_type: EmailType::GitHubSystem,
        first_seen: None,
        last_seen: None,
        commits_observed: 2,
        repositories: vec!["sample-user/repo1".into()],
        sources: vec!["Commit".into()],
        is_verified_signer: false,
    });

    report.keys.push(KeyRecord {
        key_type: KeyType::Gpg,
        key_id: Some("KEY123".into()),
        fingerprint: None,
        raw_preview: "KEY123".into(),
        emails: vec!["dev@example.com".into()],
        created_at: None,
        expires_at: None,
        can_sign: Some(true),
        source: "GitHub API".into(),
    });

    report.recompute_summary();

    assert_eq!(report.summary.total_stars_accumulated, 150);
    assert_eq!(report.summary.total_forks_accumulated, 15);
    assert_eq!(
        report.summary.distinct_emails_observed, 1,
        "Excludes GitHubSystem bots"
    );
    assert_eq!(report.summary.public_keys_found, 1);
    assert_eq!(report.summary.fork_ratio, 0.5);
    assert_eq!(report.summary.archived_ratio, 0.5);
    assert_eq!(
        report.summary.primary_languages,
        vec![("Rust".to_string(), 2)]
    );
}

#[tokio::test]
async fn test_engine_fast_mode_enrich_semantics() {
    let gh_mock = MockServer::start().await;
    let gl_mock = MockServer::start().await;

    let user_json = r#"{
        "login": "carol",
        "id": 9999,
        "avatar_url": "https://avatars.githubusercontent.com/u/9999",
        "html_url": "https://github.com/carol",
        "name": "Carol Analyst",
        "created_at": "2020-01-01T00:00:00Z",
        "updated_at": "2026-01-01T00:00:00Z",
        "public_repos": 1
    }"#;

    let repos_json = r#"[
        {
            "name": "data-repo",
            "full_name": "carol/data-repo",
            "html_url": "https://github.com/carol/data-repo",
            "default_branch": "main",
            "fork": false,
            "stargazers_count": 10,
            "forks_count": 2,
            "size": 100,
            "language": "Python",
            "open_issues_count": 0
        }
    ]"#;

    let gitleak_json = r#"{
        "emails": [
            {
                "email": "carol@secops.io",
                "committer": "Carol Analyst",
                "commitUrl": "https://github.com/carol/data-repo/commit/123456"
            }
        ],
        "metadata": {
            "totalCommitsProcessed": 5,
            "emailsFound": 1,
            "executionTimeMs": 120
        }
    }"#;

    Mock::given(method("GET"))
        .and(path("/users/carol"))
        .respond_with(ResponseTemplate::new(200).set_body_string(user_json))
        .mount(&gh_mock)
        .await;

    Mock::given(method("GET"))
        .and(path("/users/carol/repos"))
        .respond_with(ResponseTemplate::new(200).set_body_string(repos_json))
        .mount(&gh_mock)
        .await;

    Mock::given(method("GET"))
        .and(path("/repos/carol/data-repo/commits"))
        .respond_with(ResponseTemplate::new(200).set_body_string("[]"))
        .mount(&gh_mock)
        .await;

    Mock::given(method("GET"))
        .and(path("/repos/carol/data-repo/languages"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"Python": 5000}"#))
        .mount(&gh_mock)
        .await;

    Mock::given(method("GET"))
        .and(path("/repos/carol/data-repo/git/trees/main"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"tree": []}"#))
        .mount(&gh_mock)
        .await;

    Mock::given(method("GET"))
        .and(path("/users/carol/gpg_keys"))
        .respond_with(ResponseTemplate::new(200).set_body_string("[]"))
        .mount(&gh_mock)
        .await;

    Mock::given(method("GET"))
        .and(path("/users/carol/keys"))
        .respond_with(ResponseTemplate::new(200).set_body_string("[]"))
        .mount(&gh_mock)
        .await;

    Mock::given(method("GET"))
        .and(path("/users/carol/events/public"))
        .respond_with(ResponseTemplate::new(200).set_body_string("[]"))
        .mount(&gh_mock)
        .await;

    Mock::given(method("GET"))
        .and(path("/api/v1/users/carol"))
        .respond_with(ResponseTemplate::new(200).set_body_string(gitleak_json))
        .mount(&gl_mock)
        .await;

    let http = HttpClient::new();
    let gh_provider = GitHubProvider::new(http.clone(), None, Some(gh_mock.uri()), None);
    let gl_client = GitLeakClient::new(http, Some(gl_mock.uri()));
    let cache = Arc::new(NoopCacheStore);

    let engine = AnalysisEngine::new(gh_provider, gl_client, cache);
    let target = Target::parse("carol").unwrap();

    // 1. FAST with enrich: false -> GitLeak is skipped
    let report_fast = engine
        .run_analysis(&target, AnalysisOptions::fast(), None)
        .await
        .unwrap();
    assert_eq!(
        report_fast.provider_status.get("GitLeak").unwrap().status,
        ProviderHealth::Skipped
    );

    // 2. FAST with enrich: true -> GitLeak is executed and identities merged, while repo bounds remain FAST
    let report_enriched = engine
        .run_analysis(&target, AnalysisOptions::fast_enriched(), None)
        .await
        .unwrap();
    assert_eq!(
        report_enriched
            .provider_status
            .get("GitLeak")
            .unwrap()
            .status,
        ProviderHealth::Ok
    );
    let carol_id = report_enriched
        .identities
        .iter()
        .find(|i| i.email == "carol@secops.io");
    assert!(
        carol_id.is_some(),
        "GitLeak email must be merged into identities"
    );
}

#[tokio::test]
async fn test_gitleak_budget_exhaustion_in_engine() {
    let gh_mock = MockServer::start().await;
    let gl_mock = MockServer::start().await;

    let user_json = r#"{
        "login": "dave",
        "id": 1111,
        "avatar_url": "https://avatars.githubusercontent.com/u/1111",
        "html_url": "https://github.com/dave",
        "name": "Dave Developer",
        "created_at": "2021-01-01T00:00:00Z",
        "updated_at": "2026-01-01T00:00:00Z",
        "public_repos": 0
    }"#;

    Mock::given(method("GET"))
        .and(path("/users/dave"))
        .respond_with(ResponseTemplate::new(200).set_body_string(user_json))
        .mount(&gh_mock)
        .await;

    Mock::given(method("GET"))
        .and(path("/users/dave/repos"))
        .respond_with(ResponseTemplate::new(200).set_body_string("[]"))
        .mount(&gh_mock)
        .await;

    Mock::given(method("GET"))
        .and(path("/users/dave/gpg_keys"))
        .respond_with(ResponseTemplate::new(200).set_body_string("[]"))
        .mount(&gh_mock)
        .await;

    Mock::given(method("GET"))
        .and(path("/users/dave/keys"))
        .respond_with(ResponseTemplate::new(200).set_body_string("[]"))
        .mount(&gh_mock)
        .await;

    Mock::given(method("GET"))
        .and(path("/users/dave/events/public"))
        .respond_with(ResponseTemplate::new(200).set_body_string("[]"))
        .mount(&gh_mock)
        .await;

    let http = HttpClient::new();
    let gh_provider = GitHubProvider::new(http.clone(), None, Some(gh_mock.uri()), None);
    let gl_client = GitLeakClient::new(http, Some(gl_mock.uri()));
    let cache = Arc::new(NoopCacheStore);

    // GitLeak budget is strictly 0 requests
    let budget = GitLeakBudget::new(0, 60);

    let engine = AnalysisEngine::with_budget(gh_provider, gl_client, budget, cache);
    let target = Target::parse("dave").unwrap();

    let report = engine
        .run_analysis(&target, AnalysisOptions::fast_enriched(), None)
        .await
        .unwrap();

    // Verify lookup still succeeded with GitHub data
    assert_eq!(report.summary.display_name, "Dave Developer");

    // Verify GitLeak status is RateLimited with hourly budget message
    let gl_status = report.provider_status.get("GitLeak").unwrap();
    assert_eq!(gl_status.status, ProviderHealth::RateLimited);
    assert!(gl_status
        .message
        .as_ref()
        .unwrap()
        .contains("GitLeak provider hourly budget exhausted"));

    // Verify report warnings records the budget event
    assert!(report
        .warnings
        .iter()
        .any(|w| w.contains("GitLeak provider hourly budget exhausted")));
}

#[tokio::test]
async fn test_global_commit_search_role_tagging() {
    let gh_mock = MockServer::start().await;
    let gl_mock = MockServer::start().await;

    let user_json = r#"{
        "login": "eve",
        "id": 9999,
        "avatar_url": "https://avatars.githubusercontent.com/u/9999",
        "html_url": "https://github.com/eve",
        "name": "Eve Engineer",
        "created_at": "2024-01-01T00:00:00Z",
        "updated_at": "2024-01-01T00:00:00Z",
        "public_repos": 0
    }"#;

    Mock::given(method("GET"))
        .and(path("/users/eve"))
        .respond_with(ResponseTemplate::new(200).set_body_string(user_json))
        .mount(&gh_mock)
        .await;

    Mock::given(method("GET"))
        .and(path("/users/eve/repos"))
        .respond_with(ResponseTemplate::new(200).set_body_string("[]"))
        .mount(&gh_mock)
        .await;

    Mock::given(method("GET"))
        .and(path("/users/eve/gpg_keys"))
        .respond_with(ResponseTemplate::new(200).set_body_string("[]"))
        .mount(&gh_mock)
        .await;

    Mock::given(method("GET"))
        .and(path("/users/eve/keys"))
        .respond_with(ResponseTemplate::new(200).set_body_string("[]"))
        .mount(&gh_mock)
        .await;

    Mock::given(method("GET"))
        .and(path("/users/eve/events/public"))
        .respond_with(ResponseTemplate::new(200).set_body_string("[]"))
        .mount(&gh_mock)
        .await;

    let author_search_json = r#"{
        "total_count": 2,
        "incomplete_results": false,
        "items": [
            {
                "sha": "aaaa1111",
                "html_url": "https://github.com/external/repoA/commit/aaaa1111",
                "commit": {
                    "author": { "name": "Eve", "email": "eve@author.org", "date": "2024-02-01T10:00:00Z" },
                    "committer": { "name": "Eve", "email": "eve@author.org", "date": "2024-02-01T10:00:00Z" },
                    "message": "feat: author commit"
                },
                "author": { "login": "eve", "id": 9999 },
                "committer": null,
                "repository": { "id": 101, "name": "repoA", "full_name": "external/repoA", "html_url": "https://github.com/external/repoA", "fork": false }
            },
            {
                "sha": "bbbb2222",
                "html_url": "https://github.com/external/repoB/commit/bbbb2222",
                "commit": {
                    "author": { "name": "Eve", "email": "eve@both.org", "date": "2024-03-01T10:00:00Z" },
                    "committer": { "name": "Eve", "email": "eve@both.org", "date": "2024-03-01T10:00:00Z" },
                    "message": "fix: both commit"
                },
                "author": { "login": "eve", "id": 9999 },
                "committer": { "login": "eve", "id": 9999 },
                "repository": { "id": 102, "name": "repoB", "full_name": "external/repoB", "html_url": "https://github.com/external/repoB", "fork": false }
            }
        ]
    }"#;

    let committer_search_json = r#"{
        "total_count": 2,
        "incomplete_results": false,
        "items": [
            {
                "sha": "bbbb2222",
                "html_url": "https://github.com/external/repoB/commit/bbbb2222",
                "commit": {
                    "author": { "name": "Eve", "email": "eve@both.org", "date": "2024-03-01T10:00:00Z" },
                    "committer": { "name": "Eve", "email": "eve@both.org", "date": "2024-03-01T10:00:00Z" },
                    "message": "fix: both commit"
                },
                "author": { "login": "eve", "id": 9999 },
                "committer": { "login": "eve", "id": 9999 },
                "repository": { "id": 102, "name": "repoB", "full_name": "external/repoB", "html_url": "https://github.com/external/repoB", "fork": false }
            },
            {
                "sha": "cccc3333",
                "html_url": "https://github.com/external/repoC/commit/cccc3333",
                "commit": {
                    "author": { "name": "Alice", "email": "alice@corp.com", "date": "2024-04-01T10:00:00Z" },
                    "committer": { "name": "Eve", "email": "eve@committer.org", "date": "2024-04-01T10:00:00Z" },
                    "message": "merge: committer commit"
                },
                "author": { "login": "alice", "id": 8888 },
                "committer": { "login": "eve", "id": 9999 },
                "repository": { "id": 103, "name": "repoC", "full_name": "external/repoC", "html_url": "https://github.com/external/repoC", "fork": false }
            }
        ]
    }"#;

    use wiremock::matchers::query_param;

    Mock::given(method("GET"))
        .and(path("/search/commits"))
        .and(query_param("q", "author:eve"))
        .respond_with(ResponseTemplate::new(200).set_body_string(author_search_json))
        .mount(&gh_mock)
        .await;

    Mock::given(method("GET"))
        .and(path("/search/commits"))
        .and(query_param("q", "committer:eve"))
        .respond_with(ResponseTemplate::new(200).set_body_string(committer_search_json))
        .mount(&gh_mock)
        .await;

    let http = HttpClient::new();
    let gh_provider = GitHubProvider::new(http.clone(), None, Some(gh_mock.uri()), None);
    let gl_client = GitLeakClient::new(http, Some(gl_mock.uri()));
    let cache = Arc::new(NoopCacheStore);
    let engine = AnalysisEngine::new(gh_provider, gl_client, cache);
    let target = Target::parse("eve").unwrap();

    let report = engine
        .run_analysis(&target, AnalysisOptions::fast(), None)
        .await
        .unwrap();

    assert!(report.coverage.discovery.is_some());
    let disc = report.coverage.discovery.as_ref().unwrap();
    assert_eq!(disc.global_author_commits_found, 2);
    assert_eq!(disc.global_committer_commits_found, 2);

    assert!(report.identities.iter().any(|id| id.email == "eve@author.org"));
    assert!(report.identities.iter().any(|id| id.email == "eve@both.org"));
    assert!(report.identities.iter().any(|id| id.email == "eve@committer.org"));
}

#[tokio::test]
async fn test_graphql_historical_years_traversal() {
    let gh_mock = MockServer::start().await;

    let gql_user_years = r#"{
        "data": {
            "user": {
                "contributionsCollection": {
                    "contributionYears": [2024, 2023],
                    "totalCommitContributions": 150,
                    "contributionCalendar": { "totalContributions": 200 },
                    "commitContributionsByRepository": [
                        {
                            "repository": {
                                "nameWithOwner": "eve/project-2024",
                                "name": "project-2024",
                                "owner": { "login": "eve" },
                                "isPrivate": false,
                                "isFork": false,
                                "description": null,
                                "url": "https://github.com/eve/project-2024"
                            },
                            "contributions": { "totalCount": 100 }
                        }
                    ]
                }
            }
        }
    }"#;

    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_string(gql_user_years))
        .mount(&gh_mock)
        .await;

    let http = HttpClient::new();
    let gh_provider = GitHubProvider::new(
        http.clone(),
        Some("mock_token".to_string()),
        None,
        Some(format!("{}/graphql", gh_mock.uri())),
    );
    let res = gh_provider
        .fetch_user_contributions_historical("eve", 5)
        .await
        .unwrap();

    assert_eq!(res.years_queried, vec![2024, 2023]);
    assert_eq!(res.years_discovered, vec![2024, 2023]);
    assert!(res.fully_traversed);
    assert!(!res.contributions.is_empty());
}
