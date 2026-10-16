use std::sync::Arc;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use spoor_core::{AnalysisOptions, NoopCacheStore, Target};
use spoor_providers::{AnalysisEngine, GitHubProvider, GitLeakClient, HttpClient};

#[tokio::test]
async fn test_scenario_a_global_search_author_attributes_author_only() {
    let gh_mock = MockServer::start().await;

    let user_json = r#"{
        "login": "alice",
        "id": 1001,
        "avatar_url": "https://avatars.githubusercontent.com/u/1001",
        "html_url": "https://github.com/alice",
        "name": "Alice Target",
        "company": null,
        "blog": null,
        "location": null,
        "email": null,
        "bio": null,
        "twitter_username": null,
        "public_repos": 0,
        "public_gists": 0,
        "followers": 10,
        "following": 5,
        "created_at": "2020-01-01T00:00:00Z",
        "updated_at": "2026-01-01T00:00:00Z"
    }"#;
    Mock::given(method("GET"))
        .and(path("/users/alice"))
        .respond_with(ResponseTemplate::new(200).set_body_string(user_json))
        .mount(&gh_mock)
        .await;

    Mock::given(method("GET"))
        .and(path("/users/alice/repos"))
        .respond_with(ResponseTemplate::new(200).set_body_string("[]"))
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

    let gql_resp = r#"{
        "data": {
            "user": {
                "contributionsCollection": {
                    "contributionYears": []
                }
            }
        }
    }"#;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_string(gql_resp))
        .mount(&gh_mock)
        .await;

    // author:alice returns commit where author is alice, committer is GitHub web-flow
    let search_author_json = r#"{
        "total_count": 1,
        "incomplete_results": false,
        "items": [
            {
                "sha": "aaa111",
                "html_url": "https://github.com/otherorg/public-lib/commit/aaa111",
                "commit": {
                    "author": {
                        "name": "Alice Target",
                        "email": "alice@personal.org",
                        "date": "2024-05-01T12:00:00Z"
                    },
                    "committer": {
                        "name": "GitHub Web Flow",
                        "email": "noreply@github.com",
                        "date": "2024-05-01T12:00:00Z"
                    },
                    "message": "Update README.md"
                },
                "repository": {
                    "id": 501,
                    "name": "public-lib",
                    "full_name": "otherorg/public-lib",
                    "html_url": "https://github.com/otherorg/public-lib",
                    "stargazers_count": 50,
                    "forks_count": 10,
                    "fork": false
                },
                "author": {
                    "login": "alice",
                    "id": 1001
                },
                "committer": {
                    "login": "web-flow",
                    "id": 19864447
                }
            }
        ]
    }"#;
    Mock::given(method("GET"))
        .and(path("/search/commits"))
        .and(query_param("q", "author:alice"))
        .respond_with(ResponseTemplate::new(200).set_body_string(search_author_json))
        .mount(&gh_mock)
        .await;

    // committer:alice returns empty
    let empty_search_json = r#"{"total_count":0,"incomplete_results":false,"items":[]}"#;
    Mock::given(method("GET"))
        .and(path("/search/commits"))
        .and(query_param("q", "committer:alice"))
        .respond_with(ResponseTemplate::new(200).set_body_string(empty_search_json))
        .mount(&gh_mock)
        .await;

    let http = HttpClient::new();
    let gh_provider = GitHubProvider::new(
        http.clone(),
        Some("mock_token".into()),
        Some(gh_mock.uri()),
        Some(format!("{}/graphql", gh_mock.uri())),
    );
    let gl_client = GitLeakClient::new(http, None);
    let cache = Arc::new(NoopCacheStore);
    let engine = AnalysisEngine::new(gh_provider, gl_client, cache);

    let target = Target::parse("alice").unwrap();
    let report = engine
        .run_analysis(&target, AnalysisOptions::fast(), None)
        .await
        .unwrap();

    // Verify: Only alice@personal.org is in identities
    assert_eq!(report.identities.len(), 1);
    let id = &report.identities[0];
    assert_eq!(id.email, "alice@personal.org");
    assert_eq!(id.name.as_deref(), Some("Alice Target"));
    assert_eq!(id.commits_observed, 1);

    // Verify: noreply@github.com is routed to related_commit_actors as platform_bot
    assert_eq!(report.related_commit_actors.len(), 1);
    let actor = &report.related_commit_actors[0];
    assert_eq!(actor.email, "noreply@github.com");
    assert_eq!(actor.relationship, "platform_bot");
}

#[tokio::test]
async fn test_scenario_b_target_owned_repo_third_party_contributor() {
    let gh_mock = MockServer::start().await;

    let user_json = r#"{
        "login": "alice",
        "id": 1001,
        "avatar_url": "https://avatars.githubusercontent.com/u/1001",
        "html_url": "https://github.com/alice",
        "name": "Alice Target",
        "company": null,
        "blog": null,
        "location": null,
        "email": "alice@primary.com",
        "bio": null,
        "twitter_username": null,
        "public_repos": 1,
        "public_gists": 0,
        "followers": 10,
        "following": 5,
        "created_at": "2020-01-01T00:00:00Z",
        "updated_at": "2026-01-01T00:00:00Z"
    }"#;
    Mock::given(method("GET"))
        .and(path("/users/alice"))
        .respond_with(ResponseTemplate::new(200).set_body_string(user_json))
        .mount(&gh_mock)
        .await;

    let repos_json = r#"[
        {
            "id": 601,
            "name": "my-cool-project",
            "full_name": "alice/my-cool-project",
            "html_url": "https://github.com/alice/my-cool-project",
            "default_branch": "main",
            "stargazers_count": 100,
            "forks_count": 10,
            "open_issues_count": 0,
            "size": 1000,
            "language": "Rust",
            "fork": false,
            "archived": false,
            "topics": []
        }
    ]"#;
    Mock::given(method("GET"))
        .and(path("/users/alice/repos"))
        .respond_with(ResponseTemplate::new(200).set_body_string(repos_json))
        .mount(&gh_mock)
        .await;

    let commits_json = r#"[
        {
            "sha": "c111",
            "html_url": "https://github.com/alice/my-cool-project/commit/c111",
            "commit": {
                "author": { "name": "Alice Target", "email": "alice@primary.com", "date": "2024-01-01T10:00:00Z" },
                "committer": { "name": "Alice Target", "email": "alice@primary.com", "date": "2024-01-01T10:00:00Z" },
                "message": "Initial commit"
            },
            "author": { "login": "alice", "id": 1001 },
            "committer": { "login": "alice", "id": 1001 }
        },
        {
            "sha": "c222",
            "html_url": "https://github.com/alice/my-cool-project/commit/c222",
            "commit": {
                "author": { "name": "Charlie Contributor", "email": "charlie@external.com", "date": "2024-02-01T10:00:00Z" },
                "committer": { "name": "Charlie Contributor", "email": "charlie@external.com", "date": "2024-02-01T10:00:00Z" },
                "message": "Add feature X"
            },
            "author": { "login": "charlie", "id": 9999 },
            "committer": { "login": "charlie", "id": 9999 }
        }
    ]"#;
    Mock::given(method("GET"))
        .and(path("/repos/alice/my-cool-project/commits"))
        .respond_with(ResponseTemplate::new(200).set_body_string(commits_json))
        .mount(&gh_mock)
        .await;

    Mock::given(method("GET"))
        .and(path("/repos/alice/my-cool-project/git/trees/main"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"tree": []}"#))
        .mount(&gh_mock)
        .await;

    Mock::given(method("GET"))
        .and(path("/repos/alice/my-cool-project/languages"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"Rust": 1000}"#))
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

    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"{"data":{"user":{"contributionsCollection":{"contributionYears":[]}}}}"#,
        ))
        .mount(&gh_mock)
        .await;

    let empty_search_json = r#"{"total_count":0,"incomplete_results":false,"items":[]}"#;
    Mock::given(method("GET"))
        .and(path("/search/commits"))
        .respond_with(ResponseTemplate::new(200).set_body_string(empty_search_json))
        .mount(&gh_mock)
        .await;

    let http = HttpClient::new();
    let gh_provider = GitHubProvider::new(
        http.clone(),
        Some("mock_token".into()),
        Some(gh_mock.uri()),
        Some(format!("{}/graphql", gh_mock.uri())),
    );
    let gl_client = GitLeakClient::new(http, None);
    let cache = Arc::new(NoopCacheStore);
    let engine = AnalysisEngine::new(gh_provider, gl_client, cache);

    let target = Target::parse("alice").unwrap();
    let report = engine
        .run_analysis(&target, AnalysisOptions::fast(), None)
        .await
        .unwrap();

    // Verify: alice@primary.com is target identity
    assert_eq!(report.identities.len(), 1);
    assert_eq!(report.identities[0].email, "alice@primary.com");

    // Verify: Charlie is NOT in identities, but in related_commit_actors as repository_contributor
    assert_eq!(report.related_commit_actors.len(), 1);
    let charlie = &report.related_commit_actors[0];
    assert_eq!(charlie.email, "charlie@external.com");
    assert_eq!(charlie.name.as_deref(), Some("Charlie Contributor"));
    assert_eq!(charlie.relationship, "repository_contributor");
    assert_eq!(charlie.commits_observed, 1);
    assert_eq!(charlie.repositories, vec!["alice/my-cool-project"]);
}

#[tokio::test]
async fn test_scenario_c_external_repo_co_contributor() {
    let gh_mock = MockServer::start().await;

    let user_json = r#"{
        "login": "alice",
        "id": 1001,
        "avatar_url": "https://avatars.githubusercontent.com/u/1001",
        "html_url": "https://github.com/alice",
        "name": "Alice Target",
        "public_repos": 0,
        "followers": 1,
        "following": 1,
        "created_at": "2020-01-01T00:00:00Z",
        "updated_at": "2026-01-01T00:00:00Z"
    }"#;
    Mock::given(method("GET"))
        .and(path("/users/alice"))
        .respond_with(ResponseTemplate::new(200).set_body_string(user_json))
        .mount(&gh_mock)
        .await;

    Mock::given(method("GET"))
        .and(path("/users/alice/repos"))
        .respond_with(ResponseTemplate::new(200).set_body_string("[]"))
        .mount(&gh_mock)
        .await;

    let gql_resp = r#"{
        "data": {
            "user": {
                "contributionsCollection": {
                    "contributionYears": [2024]
                }
            }
        }
    }"#;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_string(gql_resp))
        .mount(&gh_mock)
        .await;

    let commits_json = r#"[
        {
            "sha": "ext1",
            "html_url": "https://github.com/bigorg/shared-engine/commit/ext1",
            "commit": {
                "author": { "name": "Alice Target", "email": "1001+alice@users.noreply.github.com", "date": "2024-03-01T10:00:00Z" },
                "committer": { "name": "Alice Target", "email": "1001+alice@users.noreply.github.com", "date": "2024-03-01T10:00:00Z" },
                "message": "Bugfix"
            },
            "author": { "login": "alice", "id": 1001 },
            "committer": { "login": "alice", "id": 1001 }
        },
        {
            "sha": "ext2",
            "html_url": "https://github.com/bigorg/shared-engine/commit/ext2",
            "commit": {
                "author": { "name": "David Maintainer", "email": "david@bigorg.com", "date": "2024-03-02T10:00:00Z" },
                "committer": { "name": "David Maintainer", "email": "david@bigorg.com", "date": "2024-03-02T10:00:00Z" },
                "message": "Merge pull request"
            },
            "author": { "login": "david", "id": 8888 },
            "committer": { "login": "david", "id": 8888 }
        }
    ]"#;
    Mock::given(method("GET"))
        .and(path("/repos/bigorg/shared-engine/commits"))
        .respond_with(ResponseTemplate::new(200).set_body_string(commits_json))
        .mount(&gh_mock)
        .await;

    Mock::given(method("GET"))
        .and(path("/repos/bigorg/shared-engine/git/trees/main"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"tree": []}"#))
        .mount(&gh_mock)
        .await;

    Mock::given(method("GET"))
        .and(path("/repos/bigorg/shared-engine/languages"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"Rust": 5000}"#))
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

    let search_json = r#"{
        "total_count": 1,
        "incomplete_results": false,
        "items": [
            {
                "sha": "ext1",
                "html_url": "https://github.com/bigorg/shared-engine/commit/ext1",
                "commit": {
                    "author": { "name": "Alice Target", "email": "1001+alice@users.noreply.github.com", "date": "2024-03-01T10:00:00Z" },
                    "committer": { "name": "Alice Target", "email": "1001+alice@users.noreply.github.com", "date": "2024-03-01T10:00:00Z" },
                    "message": "Bugfix"
                },
                "repository": {
                    "id": 801,
                    "name": "shared-engine",
                    "full_name": "bigorg/shared-engine",
                    "html_url": "https://github.com/bigorg/shared-engine",
                    "stargazers_count": 500,
                    "forks_count": 50,
                    "fork": false
                },
                "author": { "login": "alice", "id": 1001 },
                "committer": { "login": "alice", "id": 1001 }
            }
        ]
    }"#;
    Mock::given(method("GET"))
        .and(path("/search/commits"))
        .and(query_param("q", "author:alice"))
        .respond_with(ResponseTemplate::new(200).set_body_string(search_json))
        .mount(&gh_mock)
        .await;

    let empty_search_json = r#"{"total_count":0,"incomplete_results":false,"items":[]}"#;
    Mock::given(method("GET"))
        .and(path("/search/commits"))
        .and(query_param("q", "committer:alice"))
        .respond_with(ResponseTemplate::new(200).set_body_string(empty_search_json))
        .mount(&gh_mock)
        .await;

    let http = HttpClient::new();
    let gh_provider = GitHubProvider::new(
        http.clone(),
        Some("mock_token".into()),
        Some(gh_mock.uri()),
        Some(format!("{}/graphql", gh_mock.uri())),
    );
    let gl_client = GitLeakClient::new(http, None);
    let cache = Arc::new(NoopCacheStore);
    let engine = AnalysisEngine::new(gh_provider, gl_client, cache);

    let target = Target::parse("alice").unwrap();
    let report = engine
        .run_analysis(&target, AnalysisOptions::fast(), None)
        .await
        .unwrap();

    // Verify: alice's noreply is target identity
    assert_eq!(report.identities.len(), 1);
    assert_eq!(
        report.identities[0].email,
        "1001+alice@users.noreply.github.com"
    );

    // Verify: David Maintainer is in related_commit_actors with co_contributor relation
    let david = report
        .related_commit_actors
        .iter()
        .find(|a| a.email == "david@bigorg.com")
        .expect("David should be in related actors");
    assert_eq!(david.name.as_deref(), Some("David Maintainer"));
    assert_eq!(david.relationship, "co_contributor");
    assert_eq!(david.repositories, vec!["bigorg/shared-engine"]);
}

#[tokio::test]
async fn test_scenario_d_global_search_committer_target_attributes_committer_only() {
    let gh_mock = MockServer::start().await;

    let user_json = r#"{
        "login": "alice",
        "id": 1001,
        "avatar_url": "https://avatars.githubusercontent.com/u/1001",
        "html_url": "https://github.com/alice",
        "name": "Alice Maintainer",
        "public_repos": 0,
        "followers": 10,
        "following": 5,
        "created_at": "2020-01-01T00:00:00Z",
        "updated_at": "2026-01-01T00:00:00Z"
    }"#;
    Mock::given(method("GET"))
        .and(path("/users/alice"))
        .respond_with(ResponseTemplate::new(200).set_body_string(user_json))
        .mount(&gh_mock)
        .await;

    Mock::given(method("GET"))
        .and(path("/users/alice/repos"))
        .respond_with(ResponseTemplate::new(200).set_body_string("[]"))
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

    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"{"data":{"user":{"contributionsCollection":{"contributionYears":[]}}}}"#,
        ))
        .mount(&gh_mock)
        .await;

    // author:alice query returns empty
    let empty_search_json = r#"{"total_count":0,"incomplete_results":false,"items":[]}"#;
    Mock::given(method("GET"))
        .and(path("/search/commits"))
        .and(query_param("q", "author:alice"))
        .respond_with(ResponseTemplate::new(200).set_body_string(empty_search_json))
        .mount(&gh_mock)
        .await;

    // committer:alice query returns commit where committer is alice, but author is eve
    let committer_search_json = r#"{
        "total_count": 1,
        "incomplete_results": false,
        "items": [
            {
                "sha": "patch999",
                "html_url": "https://github.com/org/core-kernel/commit/patch999",
                "commit": {
                    "author": { "name": "Eve Submitter", "email": "eve@patch.org", "date": "2024-04-01T10:00:00Z" },
                    "committer": { "name": "Alice Maintainer", "email": "alice@maintainer.io", "date": "2024-04-02T10:00:00Z" },
                    "message": "Fix typo in doc"
                },
                "repository": {
                    "id": 901,
                    "name": "core-kernel",
                    "full_name": "org/core-kernel",
                    "html_url": "https://github.com/org/core-kernel",
                    "stargazers_count": 1000,
                    "forks_count": 100,
                    "fork": false
                },
                "author": { "login": "eve", "id": 7777 },
                "committer": { "login": "alice", "id": 1001 }
            }
        ]
    }"#;
    Mock::given(method("GET"))
        .and(path("/search/commits"))
        .and(query_param("q", "committer:alice"))
        .respond_with(ResponseTemplate::new(200).set_body_string(committer_search_json))
        .mount(&gh_mock)
        .await;

    let http = HttpClient::new();
    let gh_provider = GitHubProvider::new(
        http.clone(),
        Some("mock_token".into()),
        Some(gh_mock.uri()),
        Some(format!("{}/graphql", gh_mock.uri())),
    );
    let gl_client = GitLeakClient::new(http, None);
    let cache = Arc::new(NoopCacheStore);
    let engine = AnalysisEngine::new(gh_provider, gl_client, cache);

    let target = Target::parse("alice").unwrap();
    let report = engine
        .run_analysis(&target, AnalysisOptions::fast(), None)
        .await
        .unwrap();

    // Alice is attributed via committer role
    assert_eq!(report.identities.len(), 1);
    assert_eq!(report.identities[0].email, "alice@maintainer.io");

    // Eve is routed to related_commit_actors as patch_author
    assert_eq!(report.related_commit_actors.len(), 1);
    let eve = &report.related_commit_actors[0];
    assert_eq!(eve.email, "eve@patch.org");
    assert_eq!(eve.name.as_deref(), Some("Eve Submitter"));
    assert_eq!(eve.relationship, "patch_author");
}

#[tokio::test]
async fn test_scenario_e_canonical_commit_deduplication_across_search_and_rest() {
    let gh_mock = MockServer::start().await;

    let user_json = r#"{
        "login": "alice",
        "id": 1001,
        "avatar_url": "https://avatars.githubusercontent.com/u/1001",
        "html_url": "https://github.com/alice",
        "name": "Alice Target",
        "email": "alice@company.com",
        "public_repos": 1,
        "followers": 5,
        "following": 5,
        "created_at": "2020-01-01T00:00:00Z",
        "updated_at": "2026-01-01T00:00:00Z"
    }"#;
    Mock::given(method("GET"))
        .and(path("/users/alice"))
        .respond_with(ResponseTemplate::new(200).set_body_string(user_json))
        .mount(&gh_mock)
        .await;

    let repos_json = r#"[
        {
            "id": 101,
            "name": "my-repo",
            "full_name": "alice/my-repo",
            "html_url": "https://github.com/alice/my-repo",
            "default_branch": "main",
            "stargazers_count": 10,
            "forks_count": 2,
            "open_issues_count": 0,
            "size": 500,
            "language": "Rust",
            "fork": false,
            "archived": false,
            "topics": []
        }
    ]"#;
    Mock::given(method("GET"))
        .and(path("/users/alice/repos"))
        .respond_with(ResponseTemplate::new(200).set_body_string(repos_json))
        .mount(&gh_mock)
        .await;

    let commits_json = r#"[
        {
            "sha": "deadbeef12345678",
            "html_url": "https://github.com/alice/my-repo/commit/deadbeef12345678",
            "commit": {
                "author": { "name": "Alice Target", "email": "alice@company.com", "date": "2024-01-15T12:00:00Z" },
                "committer": { "name": "Alice Target", "email": "alice@company.com", "date": "2024-01-15T12:00:00Z" },
                "message": "Deduplicated commit"
            },
            "author": { "login": "alice", "id": 1001 },
            "committer": { "login": "alice", "id": 1001 }
        }
    ]"#;
    Mock::given(method("GET"))
        .and(path("/repos/alice/my-repo/commits"))
        .respond_with(ResponseTemplate::new(200).set_body_string(commits_json))
        .mount(&gh_mock)
        .await;

    Mock::given(method("GET"))
        .and(path("/repos/alice/my-repo/git/trees/main"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"tree": []}"#))
        .mount(&gh_mock)
        .await;

    Mock::given(method("GET"))
        .and(path("/repos/alice/my-repo/languages"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"Rust": 500}"#))
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

    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"{"data":{"user":{"contributionsCollection":{"contributionYears":[]}}}}"#,
        ))
        .mount(&gh_mock)
        .await;

    let search_json = r#"{
        "total_count": 1,
        "incomplete_results": false,
        "items": [
            {
                "sha": "deadbeef12345678",
                "html_url": "https://github.com/alice/my-repo/commit/deadbeef12345678",
                "commit": {
                    "author": { "name": "Alice Target", "email": "alice@company.com", "date": "2024-01-15T12:00:00Z" },
                    "committer": { "name": "Alice Target", "email": "alice@company.com", "date": "2024-01-15T12:00:00Z" },
                    "message": "Deduplicated commit"
                },
                "repository": {
                    "id": 101,
                    "name": "my-repo",
                    "full_name": "alice/my-repo",
                    "html_url": "https://github.com/alice/my-repo",
                    "stargazers_count": 10,
                    "forks_count": 2,
                    "fork": false
                },
                "author": { "login": "alice", "id": 1001 },
                "committer": { "login": "alice", "id": 1001 }
            }
        ]
    }"#;
    Mock::given(method("GET"))
        .and(path("/search/commits"))
        .and(query_param("q", "author:alice"))
        .respond_with(ResponseTemplate::new(200).set_body_string(search_json))
        .mount(&gh_mock)
        .await;

    let empty_search_json = r#"{"total_count":0,"incomplete_results":false,"items":[]}"#;
    Mock::given(method("GET"))
        .and(path("/search/commits"))
        .and(query_param("q", "committer:alice"))
        .respond_with(ResponseTemplate::new(200).set_body_string(empty_search_json))
        .mount(&gh_mock)
        .await;

    let http = HttpClient::new();
    let gh_provider = GitHubProvider::new(
        http.clone(),
        Some("mock_token".into()),
        Some(gh_mock.uri()),
        Some(format!("{}/graphql", gh_mock.uri())),
    );
    let gl_client = GitLeakClient::new(http, None);
    let cache = Arc::new(NoopCacheStore);
    let engine = AnalysisEngine::new(gh_provider, gl_client, cache);

    let target = Target::parse("alice").unwrap();
    let report = engine
        .run_analysis(&target, AnalysisOptions::fast(), None)
        .await
        .unwrap();

    // Verify: commits_observed is exactly 1, not 2!
    assert_eq!(report.identities.len(), 1);
    assert_eq!(report.identities[0].commits_observed, 1);
    assert_eq!(report.coverage.commits_scanned, 1);
}

#[tokio::test]
async fn test_scenario_f_coverage_denominator_accounting() {
    let gh_mock = MockServer::start().await;

    let user_json = r#"{
        "login": "alice",
        "id": 1001,
        "avatar_url": "https://avatars.githubusercontent.com/u/1001",
        "html_url": "https://github.com/alice",
        "name": "Alice Target",
        "public_repos": 2,
        "followers": 1,
        "following": 1,
        "created_at": "2020-01-01T00:00:00Z",
        "updated_at": "2026-01-01T00:00:00Z"
    }"#;
    Mock::given(method("GET"))
        .and(path("/users/alice"))
        .respond_with(ResponseTemplate::new(200).set_body_string(user_json))
        .mount(&gh_mock)
        .await;

    let repos_json = r#"[
        {
            "id": 101,
            "name": "repo-one",
            "full_name": "alice/repo-one",
            "html_url": "https://github.com/alice/repo-one",
            "default_branch": "main",
            "stargazers_count": 5,
            "forks_count": 1,
            "open_issues_count": 0,
            "size": 100,
            "language": "Rust",
            "fork": false,
            "archived": false,
            "topics": []
        },
        {
            "id": 102,
            "name": "repo-two",
            "full_name": "alice/repo-two",
            "html_url": "https://github.com/alice/repo-two",
            "default_branch": "main",
            "stargazers_count": 10,
            "forks_count": 2,
            "open_issues_count": 0,
            "size": 200,
            "language": "Rust",
            "fork": false,
            "archived": false,
            "topics": []
        }
    ]"#;
    Mock::given(method("GET"))
        .and(path("/users/alice/repos"))
        .respond_with(ResponseTemplate::new(200).set_body_string(repos_json))
        .mount(&gh_mock)
        .await;

    Mock::given(method("GET"))
        .and(path("/repos/alice/repo-one/commits"))
        .respond_with(ResponseTemplate::new(200).set_body_string("[]"))
        .mount(&gh_mock)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/alice/repo-one/git/trees/main"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"tree": []}"#))
        .mount(&gh_mock)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/alice/repo-one/languages"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"Rust": 100}"#))
        .mount(&gh_mock)
        .await;

    Mock::given(method("GET"))
        .and(path("/repos/alice/repo-two/commits"))
        .respond_with(ResponseTemplate::new(200).set_body_string("[]"))
        .mount(&gh_mock)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/alice/repo-two/git/trees/main"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"tree": []}"#))
        .mount(&gh_mock)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/alice/repo-two/languages"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"Rust": 200}"#))
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

    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"{"data":{"user":{"contributionsCollection":{"contributionYears":[]}}}}"#,
        ))
        .mount(&gh_mock)
        .await;

    let empty_search_json = r#"{"total_count":0,"incomplete_results":false,"items":[]}"#;
    Mock::given(method("GET"))
        .and(path("/search/commits"))
        .respond_with(ResponseTemplate::new(200).set_body_string(empty_search_json))
        .mount(&gh_mock)
        .await;

    let http = HttpClient::new();
    let gh_provider = GitHubProvider::new(
        http.clone(),
        Some("mock_token".into()),
        Some(gh_mock.uri()),
        Some(format!("{}/graphql", gh_mock.uri())),
    );
    let gl_client = GitLeakClient::new(http, None);
    let cache = Arc::new(NoopCacheStore);
    let engine = AnalysisEngine::new(gh_provider, gl_client, cache);

    let target = Target::parse("alice").unwrap();
    let report = engine
        .run_analysis(&target, AnalysisOptions::fast(), None)
        .await
        .unwrap();

    // Verify separate accounting:
    assert_eq!(report.coverage.owned_repositories_total, 2);
    assert_eq!(report.coverage.owned_repositories_scanned, 2);
    assert_eq!(report.coverage.external_repositories_discovered, 0);
    assert_eq!(report.coverage.external_repositories_scanned, 0);
    assert_eq!(report.coverage.candidate_repositories_total, 2);
    assert_eq!(report.coverage.candidate_repositories_scanned, 2);
    // Denominator consistency
    assert!(report.coverage.repositories_scanned <= report.coverage.repositories_total);
}
