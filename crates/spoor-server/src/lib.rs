pub mod limiter;
pub mod routes;
pub mod state;

use std::net::SocketAddr;

use axum::extract::DefaultBodyLimit;
use axum::routing::{get, post};
use axum::Router;
use spoor_core::SpoorError;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing::info;

pub use limiter::EphemeralIpRateLimiter;
pub use routes::*;
pub use state::{AppState, LookupEntry, LookupStatus};

pub fn create_router(state: AppState) -> Router {
    // Configurable CORS: read SPOOR_ALLOWED_ORIGINS or default to development origins
    let cors = if let Ok(origins) = std::env::var("SPOOR_ALLOWED_ORIGINS") {
        let parsed_origins: Vec<_> = origins
            .split(',')
            .filter_map(|s| s.trim().parse().ok())
            .collect();
        CorsLayer::new()
            .allow_origin(AllowOrigin::list(parsed_origins))
            .allow_methods(tower_http::cors::Any)
            .allow_headers(tower_http::cors::Any)
    } else {
        // Development default: allow localhost and common development ports
        CorsLayer::new()
            .allow_origin(AllowOrigin::predicate(|origin, _| {
                let s = origin.to_str().unwrap_or("");
                s.starts_with("http://localhost") || s.starts_with("http://127.0.0.1")
            }))
            .allow_methods(tower_http::cors::Any)
            .allow_headers(tower_http::cors::Any)
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
