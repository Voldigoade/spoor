use axum::http::StatusCode;
use reqwest::Client;
use spoor_core::NoopCacheStore;
use spoor_providers::{AnalysisEngine, GitHubProvider, GitLeakClient, HttpClient};
use spoor_server::{create_router, AppState};
use std::sync::Arc;

#[tokio::test]
async fn test_server_health_and_lookups() {
    let http = HttpClient::new();
    let gh = GitHubProvider::new(http.clone(), None, None, None);
    let gl = GitLeakClient::new(http, None);
    let engine = Arc::new(AnalysisEngine::new(gh, gl, Arc::new(NoopCacheStore)));
    let state = AppState::new(engine);

    let app = create_router(state).into_make_service_with_connect_info::<std::net::SocketAddr>();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = Client::new();

    // 1. Check healthz
    let health_resp = client
        .get(format!("http://{addr}/healthz"))
        .send()
        .await
        .unwrap();
    assert_eq!(health_resp.status(), StatusCode::OK);
    let health_body: serde_json::Value = health_resp.json().await.unwrap();
    assert_eq!(health_body["status"], "ok");

    // 2. Reject SSRF / invalid target
    let bad_req = client
        .post(format!("http://{addr}/v1/lookups"))
        .json(&serde_json::json!({
            "target": "https://evil.attacker.com/profile"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(bad_req.status(), StatusCode::BAD_REQUEST);

    // 3. Create valid lookup
    let post_resp = client
        .post(format!("http://{addr}/v1/lookups"))
        .json(&serde_json::json!({
            "target": "torvalds",
            "mode": "fast"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(post_resp.status(), StatusCode::ACCEPTED);
    let meta: serde_json::Value = post_resp.json().await.unwrap();
    let lookup_id = meta["id"].as_str().unwrap();

    // 4. Check status
    let get_resp = client
        .get(format!("http://{addr}/v1/lookups/{lookup_id}"))
        .send()
        .await
        .unwrap();
    assert_eq!(get_resp.status(), StatusCode::OK);
}
