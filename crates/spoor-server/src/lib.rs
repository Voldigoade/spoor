pub mod limiter;
pub mod proxy;
pub mod routes;
pub mod state;

use std::net::SocketAddr;

use axum::extract::DefaultBodyLimit;
use axum::http::{header, Method};
use axum::routing::{get, post};
use axum::Router;
use spoor_core::SpoorError;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing::info;

pub use limiter::{EphemeralIpRateLimiter, GlobalRateLimiter, RateLimitResult, ServerRateLimiter};
pub use proxy::TrustedProxyConfig;
pub use routes::*;
pub use state::{AppState, LookupEntry, LookupStatus, ServerOptions};

pub fn create_router(state: AppState) -> Router {
    let cors = if let Ok(origins) = std::env::var("SPOOR_ALLOWED_ORIGINS") {
        let parsed_origins: Vec<_> = origins
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .filter_map(|s| s.parse().ok())
            .collect();
        CorsLayer::new()
            .allow_origin(AllowOrigin::list(parsed_origins))
            .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
            .allow_headers([header::CONTENT_TYPE, header::ACCEPT, header::AUTHORIZATION])
    } else {
        CorsLayer::new()
            .allow_origin(AllowOrigin::predicate(|origin, _| {
                let s = origin.to_str().unwrap_or("");
                s.starts_with("http://localhost") || s.starts_with("http://127.0.0.1")
            }))
            .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
            .allow_headers([header::CONTENT_TYPE, header::ACCEPT, header::AUTHORIZATION])
    };

    Router::new()
        .route("/healthz", get(health_handler))
        .route("/v1/lookups", post(create_lookup_handler))
        .route("/v1/lookups/{id}", get(get_lookup_handler))
        .route("/v1/lookups/{id}/events", get(stream_events_handler))
        .route("/v1/lookups/{id}/report", get(get_report_handler))
        .route(
            "/v1/lookups/{id}/export/{format}",
            get(export_report_handler),
        )
        .layer(DefaultBodyLimit::max(64 * 1024)) // 64 KB request limit
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

pub async fn run_server(addr: SocketAddr, state: AppState) -> Result<(), SpoorError> {
    let retention_state = state.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            interval.tick().await;
            let pruned = retention_state.prune_expired_lookups().await;
            if pruned > 0 {
                tracing::debug!("Pruned {pruned} expired lookups from memory");
            }
        }
    });

    let app = create_router(state).into_make_service_with_connect_info::<SocketAddr>();
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| SpoorError::Config(format!("Failed to bind to {addr}: {e}")))?;

    info!("Spoor HTTP service listening on http://{addr}");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(|e| SpoorError::Internal(format!("Server error: {e}")))?;

    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    info!("Shutdown signal received, draining connections...");
}

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
