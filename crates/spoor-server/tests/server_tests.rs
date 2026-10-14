use axum::http::StatusCode;
use reqwest::Client;
use spoor_core::NoopCacheStore;
use spoor_providers::{AnalysisEngine, GitHubProvider, GitLeakClient, HttpClient};
use spoor_server::{create_router, AppState, ServerOptions, TrustedProxyConfig};
use std::sync::Arc;

fn test_engine() -> Arc<AnalysisEngine> {
    let http = HttpClient::new();
    let gh = GitHubProvider::new(http.clone(), None, None, None);
    let gl = GitLeakClient::new(http, None);
    Arc::new(AnalysisEngine::new(gh, gl, Arc::new(NoopCacheStore)))
}

#[tokio::test]
async fn test_server_health_and_lookups() {
    let engine = test_engine();
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

#[tokio::test]
async fn test_default_config_spoofed_xff_does_not_bypass_rate_limit() {
    // Default config: trusted proxies disabled.
    // Configure rate limit of 2 requests per window.
    let engine = test_engine();
    let state = AppState::with_options(engine, TrustedProxyConfig::default(), 2, 60);

    let app = create_router(state).into_make_service_with_connect_info::<std::net::SocketAddr>();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = Client::new();

    // Request 1: Client sends spoofed X-Forwarded-For: 1.1.1.1
    let r1 = client
        .post(format!("http://{addr}/v1/lookups"))
        .header("X-Forwarded-For", "1.1.1.1")
        .json(&serde_json::json!({ "target": "torvalds" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r1.status(), StatusCode::ACCEPTED);

    // Request 2: Client changes spoofed X-Forwarded-For: 2.2.2.2
    let r2 = client
        .post(format!("http://{addr}/v1/lookups"))
        .header("X-Forwarded-For", "2.2.2.2")
        .json(&serde_json::json!({ "target": "torvalds" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r2.status(), StatusCode::ACCEPTED);

    // Request 3: Client changes spoofed X-Forwarded-For: 3.3.3.3
    // In default mode, X-Forwarded-For MUST be ignored and rate limiting MUST be applied
    // to the real socket peer IP (127.0.0.1). Therefore, request 3 is rejected!
    let r3 = client
        .post(format!("http://{addr}/v1/lookups"))
        .header("X-Forwarded-For", "3.3.3.3")
        .json(&serde_json::json!({ "target": "torvalds" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r3.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(r3.headers().contains_key("retry-after"));

    let err: serde_json::Value = r3.json().await.unwrap();
    assert_eq!(err["code"], "rate_limited");
    assert!(err["retry_after"].is_number());
}

#[tokio::test]
async fn test_trusted_proxy_honors_forwarded_ip_and_blocks_spoofing() {
    // Trusted proxy enabled: 127.0.0.1 is configured as a trusted reverse proxy.
    // Configure rate limit of 2 requests per IP per window.
    let engine = test_engine();
    let proxy_config = TrustedProxyConfig::parse("127.0.0.1").unwrap();
    let state = AppState::with_options(engine, proxy_config, 2, 60);

    let app = create_router(state).into_make_service_with_connect_info::<std::net::SocketAddr>();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = Client::new();

    // Client IP 203.0.113.10 forwarded by proxy
    let r1 = client
        .post(format!("http://{addr}/v1/lookups"))
        .header("X-Forwarded-For", "203.0.113.10")
        .json(&serde_json::json!({ "target": "torvalds" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r1.status(), StatusCode::ACCEPTED);

    let r2 = client
        .post(format!("http://{addr}/v1/lookups"))
        .header("X-Forwarded-For", "203.0.113.10")
        .json(&serde_json::json!({ "target": "torvalds" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r2.status(), StatusCode::ACCEPTED);

    // 3rd request for 203.0.113.10 exceeds rate limit
    let r3 = client
        .post(format!("http://{addr}/v1/lookups"))
        .header("X-Forwarded-For", "203.0.113.10")
        .json(&serde_json::json!({ "target": "torvalds" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r3.status(), StatusCode::TOO_MANY_REQUESTS);

    // Attacker attempts to spoof an IP to the left: "8.8.8.8, 203.0.113.10".
    // Right-to-left resolution stops at 203.0.113.10 (first untrusted IP from right).
    // Because 203.0.113.10 has exceeded its limit, it is still rejected!
    let r4 = client
        .post(format!("http://{addr}/v1/lookups"))
        .header("X-Forwarded-For", "8.8.8.8, 203.0.113.10")
        .json(&serde_json::json!({ "target": "torvalds" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r4.status(), StatusCode::TOO_MANY_REQUESTS);

    // A different genuine client IP 198.51.100.50 forwarded by the proxy is allowed!
    let r5 = client
        .post(format!("http://{addr}/v1/lookups"))
        .header("X-Forwarded-For", "198.51.100.50")
        .json(&serde_json::json!({ "target": "torvalds" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r5.status(), StatusCode::ACCEPTED);
}

#[tokio::test]
async fn test_untrusted_peer_cannot_spoof_even_when_other_proxies_are_trusted() {
    // Trusted proxy is 10.0.0.1, but client connects directly from 127.0.0.1.
    // Configure rate limit of 2 requests per window.
    let engine = test_engine();
    let proxy_config = TrustedProxyConfig::parse("10.0.0.1").unwrap();
    let state = AppState::with_options(engine, proxy_config, 2, 60);

    let app = create_router(state).into_make_service_with_connect_info::<std::net::SocketAddr>();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = Client::new();

    let r1 = client
        .post(format!("http://{addr}/v1/lookups"))
        .header("X-Forwarded-For", "10.0.0.1")
        .json(&serde_json::json!({ "target": "torvalds" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r1.status(), StatusCode::ACCEPTED);

    let r2 = client
        .post(format!("http://{addr}/v1/lookups"))
        .header("X-Forwarded-For", "1.2.3.4")
        .json(&serde_json::json!({ "target": "torvalds" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r2.status(), StatusCode::ACCEPTED);

    // 3rd request from 127.0.0.1 must be blocked because the peer is untrusted,
    // so headers are ignored and rate limiting keys on the peer (127.0.0.1).
    let r3 = client
        .post(format!("http://{addr}/v1/lookups"))
        .header("X-Forwarded-For", "5.6.7.8")
        .json(&serde_json::json!({ "target": "torvalds" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r3.status(), StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn test_cors_exact_origin_matching() {
    // Test exact origin matching with SPOOR_ALLOWED_ORIGINS
    std::env::set_var(
        "SPOOR_ALLOWED_ORIGINS",
        "https://voldigoade-portfolio.vercel.app, https://portfolio.example.com",
    );

    let engine = test_engine();
    let state = AppState::new(engine);

    let app = create_router(state).into_make_service_with_connect_info::<std::net::SocketAddr>();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = Client::new();

    // 1. Authorized origin: OPTIONS preflight
    let resp = client
        .request(
            reqwest::Method::OPTIONS,
            format!("http://{addr}/v1/lookups"),
        )
        .header("Origin", "https://voldigoade-portfolio.vercel.app")
        .header("Access-Control-Request-Method", "POST")
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers().get("access-control-allow-origin").unwrap(),
        "https://voldigoade-portfolio.vercel.app"
    );

    // 2. Unauthorized origin: preflight must NOT return access-control-allow-origin
    let bad_resp = client
        .request(
            reqwest::Method::OPTIONS,
            format!("http://{addr}/v1/lookups"),
        )
        .header("Origin", "https://malicious-site.com")
        .header("Access-Control-Request-Method", "POST")
        .send()
        .await
        .unwrap();

    assert!(bad_resp
        .headers()
        .get("access-control-allow-origin")
        .is_none());

    // Clean up env var
    std::env::remove_var("SPOOR_ALLOWED_ORIGINS");
}

#[tokio::test]
async fn test_http_deep_disabled_returns_403() {
    let engine = test_engine();
    let state = AppState::new(engine); // http_deep_enabled is false by default

    let app = create_router(state).into_make_service_with_connect_info::<std::net::SocketAddr>();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = Client::new();

    let resp = client
        .post(format!("http://{addr}/v1/lookups"))
        .json(&serde_json::json!({
            "target": "torvalds",
            "mode": "deep"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["code"], "mode_disabled");
    assert!(body["error"].as_str().unwrap().contains("disabled"));
}

#[tokio::test]
async fn test_http_deep_enabled_allows_deep_and_enrich_flag() {
    let engine = test_engine();
    // Enable http_deep_enabled explicitly
    let state = AppState::with_server_options(
        engine,
        ServerOptions {
            http_deep_enabled: true,
            ip_limit: 10,
            ip_window_secs: 60,
            global_limit: 100,
            global_window_secs: 60,
            max_concurrency: 4,
            retention_secs: 3600,
            ..Default::default()
        },
    );

    let app = create_router(state).into_make_service_with_connect_info::<std::net::SocketAddr>();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = Client::new();

    // 1. Deep mode is allowed when enabled
    let resp = client
        .post(format!("http://{addr}/v1/lookups"))
        .json(&serde_json::json!({
            "target": "torvalds",
            "mode": "deep"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::ACCEPTED);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["mode"], "deep");

    // 2. Fast mode with enrich: true is accepted
    let resp_enrich = client
        .post(format!("http://{addr}/v1/lookups"))
        .json(&serde_json::json!({
            "target": "torvalds",
            "mode": "fast",
            "enrich": true
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp_enrich.status(), StatusCode::ACCEPTED);
    let enrich_body: serde_json::Value = resp_enrich.json().await.unwrap();
    assert_eq!(enrich_body["mode"], "fast");
    assert_eq!(enrich_body["enrich"], true);
}

#[tokio::test]
async fn test_global_rate_limiter_breach() {
    let engine = test_engine();
    // Per-IP limit is 10, but global limit is strictly 2
    let state = AppState::with_server_options(
        engine,
        ServerOptions {
            ip_limit: 10,
            ip_window_secs: 60,
            global_limit: 2, // global limit: 2
            global_window_secs: 60,
            ..Default::default()
        },
    );

    let app = create_router(state).into_make_service_with_connect_info::<std::net::SocketAddr>();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = Client::new();

    let r1 = client
        .post(format!("http://{addr}/v1/lookups"))
        .json(&serde_json::json!({ "target": "torvalds" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r1.status(), StatusCode::ACCEPTED);

    let r2 = client
        .post(format!("http://{addr}/v1/lookups"))
        .json(&serde_json::json!({ "target": "torvalds" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r2.status(), StatusCode::ACCEPTED);

    // 3rd request breaches global capacity of 2
    let r3 = client
        .post(format!("http://{addr}/v1/lookups"))
        .json(&serde_json::json!({ "target": "torvalds" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r3.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(r3.headers().contains_key("retry-after"));
    let err: serde_json::Value = r3.json().await.unwrap();
    assert_eq!(err["code"], "rate_limited");
    assert!(err["retry_after"].is_number());
}

#[tokio::test]
async fn test_in_memory_lookup_ttl_retention() {
    use chrono::{Duration as CDuration, Utc};
    use spoor_core::{AnalysisMode, Target};
    use spoor_server::{LookupEntry, LookupStatus};
    use tokio::sync::broadcast;
    use uuid::Uuid;

    let engine = test_engine();
    let state = AppState::new(engine);

    let (tx, _) = broadcast::channel(16);

    let id_pending = Uuid::new_v4();
    let id_running = Uuid::new_v4();
    let id_old_completed = Uuid::new_v4();
    let id_old_failed = Uuid::new_v4();
    let id_recent_completed = Uuid::new_v4();

    let now = Utc::now();
    let old_time = now - CDuration::hours(3);

    {
        let mut map = state.lookups.write().await;

        // Pending: old, but MUST NOT be evicted
        map.insert(
            id_pending,
            LookupEntry {
                id: id_pending,
                target: Target::User {
                    username: "user1".into(),
                },
                mode: AnalysisMode::Fast,
                enrich: false,
                status: LookupStatus::Pending,
                created_at: old_time,
                report: None,
                error: None,
                event_tx: tx.clone(),
            },
        );

        // Running: old, but MUST NOT be evicted
        map.insert(
            id_running,
            LookupEntry {
                id: id_running,
                target: Target::User {
                    username: "user2".into(),
                },
                mode: AnalysisMode::Fast,
                enrich: false,
                status: LookupStatus::Running,
                created_at: old_time,
                report: None,
                error: None,
                event_tx: tx.clone(),
            },
        );

        // Old completed: MUST be evicted
        map.insert(
            id_old_completed,
            LookupEntry {
                id: id_old_completed,
                target: Target::User {
                    username: "user3".into(),
                },
                mode: AnalysisMode::Fast,
                enrich: false,
                status: LookupStatus::Completed,
                created_at: old_time,
                report: None,
                error: None,
                event_tx: tx.clone(),
            },
        );

        // Old failed: MUST be evicted
        map.insert(
            id_old_failed,
            LookupEntry {
                id: id_old_failed,
                target: Target::User {
                    username: "user4".into(),
                },
                mode: AnalysisMode::Fast,
                enrich: false,
                status: LookupStatus::Failed,
                created_at: old_time,
                report: None,
                error: None,
                event_tx: tx.clone(),
            },
        );

        // Recent completed: MUST NOT be evicted
        map.insert(
            id_recent_completed,
            LookupEntry {
                id: id_recent_completed,
                target: Target::User {
                    username: "user5".into(),
                },
                mode: AnalysisMode::Fast,
                enrich: false,
                status: LookupStatus::Completed,
                created_at: now,
                report: None,
                error: None,
                event_tx: tx.clone(),
            },
        );
    }

    let pruned = state.prune_expired_lookups().await;
    assert_eq!(pruned, 2);

    let map = state.lookups.read().await;
    assert!(map.contains_key(&id_pending));
    assert!(map.contains_key(&id_running));
    assert!(!map.contains_key(&id_old_completed));
    assert!(!map.contains_key(&id_old_failed));
    assert!(map.contains_key(&id_recent_completed));
}
