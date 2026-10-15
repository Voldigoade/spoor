use chrono::{Duration, Utc};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use spoor_providers::{GitHubProvider, HttpClient};

fn events_json(count: usize, offset: usize) -> String {
    let created_at = (Utc::now() - Duration::hours(1)).to_rfc3339();
    let events: Vec<_> = (0..count)
        .map(|index| {
            serde_json::json!({
                "id": format!("event-{}", offset + index),
                "type": "PushEvent",
                "repo": { "name": format!("example/repo-{}", offset + index) },
                "created_at": created_at.clone()
            })
        })
        .collect();
    serde_json::to_string(&events).unwrap()
}

async fn mount_page(
    server: &MockServer,
    page: usize,
    per_page: usize,
    count: usize,
    has_next: bool,
    status: u16,
) {
    let mut response = ResponseTemplate::new(status).set_body_string(if status == 200 {
        events_json(count, page * 100)
    } else {
        "{}".to_string()
    });
    if has_next {
        response = response.insert_header(
            "Link",
            format!(
                "<{}/users/alice/events/public?per_page={per_page}&page={}>; rel=\"next\"",
                server.uri(),
                page + 1
            ),
        );
    }
    Mock::given(method("GET"))
        .and(path("/users/alice/events/public"))
        .and(query_param("per_page", per_page.to_string()))
        .and(query_param("page", page.to_string()))
        .respond_with(response)
        .mount(server)
        .await;
}

fn provider(server: &MockServer) -> GitHubProvider {
    GitHubProvider::new(HttpClient::new(), None, Some(server.uri()), None)
}

#[tokio::test]
async fn zero_limit_returns_without_events() {
    let server = MockServer::start().await;
    let result = provider(&server)
        .fetch_user_events("alice", 0)
        .await
        .unwrap();

    assert!(result.events.is_empty());
    assert!(result.is_complete);
}

#[tokio::test]
async fn empty_first_page_returns_complete_empty_batch() {
    let server = MockServer::start().await;
    mount_page(&server, 1, 100, 0, false, 200).await;

    let result = provider(&server)
        .fetch_user_events("alice", 300)
        .await
        .unwrap();

    assert!(result.events.is_empty());
    assert!(result.is_complete);
    assert!(result.warning.is_none());
}

#[tokio::test]
async fn short_page_returns_all_available_events() {
    let server = MockServer::start().await;
    mount_page(&server, 1, 50, 20, false, 200).await;

    let result = provider(&server)
        .fetch_user_events("alice", 50)
        .await
        .unwrap();

    assert_eq!(result.events.len(), 20);
    assert!(result.is_complete);
    assert!(result.warning.is_none());
}

#[tokio::test]
async fn request_limit_100_returns_one_full_page() {
    let server = MockServer::start().await;
    mount_page(&server, 1, 100, 100, false, 200).await;

    let result = provider(&server)
        .fetch_user_events("alice", 100)
        .await
        .unwrap();

    assert_eq!(result.events.len(), 100);
    assert!(result.is_complete);
}

#[tokio::test]
async fn fetches_101_events_across_two_pages() {
    let server = MockServer::start().await;
    mount_page(&server, 1, 100, 100, true, 200).await;
    mount_page(&server, 2, 100, 1, false, 200).await;

    let result = provider(&server)
        .fetch_user_events("alice", 101)
        .await
        .unwrap();

    assert_eq!(result.events.len(), 101);
    assert!(result.is_complete);
}

#[tokio::test]
async fn fetches_two_full_pages() {
    let server = MockServer::start().await;
    mount_page(&server, 1, 100, 100, true, 200).await;
    mount_page(&server, 2, 100, 100, false, 200).await;

    let result = provider(&server)
        .fetch_user_events("alice", 200)
        .await
        .unwrap();

    assert_eq!(result.events.len(), 200);
    assert!(result.is_complete);
}

#[tokio::test]
async fn stops_at_three_pages_and_300_events() {
    let server = MockServer::start().await;
    mount_page(&server, 1, 100, 100, true, 200).await;
    mount_page(&server, 2, 100, 100, true, 200).await;
    mount_page(&server, 3, 100, 100, false, 200).await;

    let result = provider(&server)
        .fetch_user_events("alice", 500)
        .await
        .unwrap();

    assert_eq!(result.events.len(), 300);
    assert!(result.is_complete);
}

#[tokio::test]
async fn first_page_failure_returns_an_error() {
    let server = MockServer::start().await;
    mount_page(&server, 1, 100, 0, false, 404).await;

    assert!(provider(&server)
        .fetch_user_events("alice", 300)
        .await
        .is_err());
}

#[tokio::test]
async fn later_page_failure_returns_partial_data_and_warning() {
    let server = MockServer::start().await;
    mount_page(&server, 1, 100, 100, true, 200).await;
    mount_page(&server, 2, 100, 0, false, 404).await;

    let result = provider(&server)
        .fetch_user_events("alice", 300)
        .await
        .unwrap();

    assert_eq!(result.events.len(), 100);
    assert!(!result.is_complete);
    assert!(result.warning.is_some());
}

#[tokio::test]
async fn filters_events_older_than_30_days() {
    let server = MockServer::start().await;
    let body = serde_json::json!([
        {
            "id": "recent",
            "type": "PushEvent",
            "repo": { "name": "example/recent" },
            "created_at": (Utc::now() - Duration::hours(1)).to_rfc3339()
        },
        {
            "id": "old",
            "type": "PushEvent",
            "repo": { "name": "example/old" },
            "created_at": (Utc::now() - Duration::days(31)).to_rfc3339()
        }
    ]);
    Mock::given(method("GET"))
        .and(path("/users/alice/events/public"))
        .and(query_param("per_page", "100"))
        .and(query_param("page", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_string(body.to_string()))
        .mount(&server)
        .await;

    let result = provider(&server)
        .fetch_user_events("alice", 300)
        .await
        .unwrap();

    assert_eq!(result.events.len(), 1);
    assert_eq!(result.events[0].id, "recent");
}
