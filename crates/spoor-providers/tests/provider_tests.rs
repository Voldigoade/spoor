use std::sync::Arc;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use spoor_core::{AnalysisMode, NoopCacheStore, Target};
use spoor_providers::{AnalysisEngine, GitHubProvider, GitLeakClient, GitLeakOutcome, HttpClient};

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
