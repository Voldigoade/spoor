use std::convert::Infallible;
use std::net::SocketAddr;
use std::time::Duration;

use axum::extract::{ConnectInfo, Path, State};
use axum::http::header::{CONTENT_DISPOSITION, CONTENT_TYPE, RETRY_AFTER};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use spoor_core::{AnalysisMode, AnalysisOptions, SpoorEvent, Target};
use spoor_export::{export_bytes, ExportFormat};
use tokio::sync::{broadcast, mpsc};
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::Stream;
use tokio_stream::StreamExt;
use uuid::Uuid;

use crate::state::{AppState, LookupEntry, LookupStatus};

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub version: &'static str,
}

pub async fn health_handler() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
    })
}

#[derive(Deserialize)]
pub struct CreateLookupRequest {
    pub target: String,
    #[serde(default)]
    pub mode: Option<AnalysisMode>,
    #[serde(default)]
    pub enrich: Option<bool>,
}

#[derive(Serialize)]
pub struct LookupMetaResponse {
    pub id: Uuid,
    pub target: String,
    pub mode: AnalysisMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enrich: Option<bool>,
    pub status: LookupStatus,
    pub created_at: chrono::DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Serialize)]
pub struct ApiErrorResponse {
    pub error: String,
    pub code: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after: Option<u64>,
}

impl ApiErrorResponse {
    pub fn new(code: &'static str, error: impl Into<String>) -> Self {
        Self {
            error: error.into(),
            code,
            retry_after: None,
        }
    }

    pub fn with_retry(code: &'static str, error: impl Into<String>, retry_after: u64) -> Self {
        Self {
            error: error.into(),
            code,
            retry_after: Some(retry_after),
        }
    }
}

pub async fn create_lookup_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(payload): Json<CreateLookupRequest>,
) -> Response {
    let _ = state.prune_expired_lookups().await;

    let mode = payload.mode.unwrap_or(AnalysisMode::Fast);
    let enrich = payload.enrich.unwrap_or(false);

    if mode == AnalysisMode::Deep && !state.http_deep_enabled {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiErrorResponse::new(
                "mode_disabled",
                "Full DEEP analysis mode is disabled on this public instance. Use mode=fast with optional enrich=true.",
            )),
        )
            .into_response();
    }

    // Validate target syntax before charging any rate-limit quota
    let target = match Target::parse(&payload.target) {
        Ok(t) => t,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResponse::new("invalid_target", e.to_string())),
            )
                .into_response();
        }
    };

    let client_ip = state.proxy_config.resolve_client_ip(addr.ip(), &headers);

    let rate_limit_res = {
        let mut limiter = state.limiter.lock().await;
        limiter.check_and_record(&client_ip.to_string())
    };

    if let Some(retry_after) = rate_limit_res.retry_after() {
        let mut resp_headers = HeaderMap::new();
        if let Ok(val) = retry_after.to_string().parse() {
            resp_headers.insert(RETRY_AFTER, val);
        }
        return (
            StatusCode::TOO_MANY_REQUESTS,
            resp_headers,
            Json(ApiErrorResponse::with_retry(
                "rate_limited",
                format!("Rate limit exceeded. Please retry after {retry_after} seconds."),
                retry_after,
            )),
        )
            .into_response();
    }

    let id = Uuid::new_v4();
    let created_at = Utc::now();

    let (broadcast_tx, _) = broadcast::channel(128);

    let entry = LookupEntry {
        id,
        target: target.clone(),
        mode,
        enrich,
        status: LookupStatus::Pending,
        created_at,
        report: None,
        error: None,
        event_tx: broadcast_tx.clone(),
    };

    {
        let mut map = state.lookups.write().await;
        map.insert(id, entry);
    }

    let bg_state = state.clone();
    let bg_target = target.clone();
    let bg_id = id;
    let bg_options = AnalysisOptions { mode, enrich };
    let bg_bcast = broadcast_tx;

    tokio::spawn(async move {
        let (mpsc_tx, mut mpsc_rx) = mpsc::channel::<SpoorEvent>(64);
        let bcast_forward = bg_bcast.clone();
        tokio::spawn(async move {
            while let Some(ev) = mpsc_rx.recv().await {
                let _ = bcast_forward.send(ev);
            }
        });

        // Wait for concurrency permit before transitioning to Running
        let _permit = bg_state.concurrency_semaphore.acquire().await;
        {
            let mut map = bg_state.lookups.write().await;
            if let Some(e) = map.get_mut(&bg_id) {
                e.status = LookupStatus::Running;
            }
        }

        let res = bg_state
            .engine
            .run_analysis(&bg_target, bg_options, Some(mpsc_tx))
            .await;

        let mut map = bg_state.lookups.write().await;
        if let Some(e) = map.get_mut(&bg_id) {
            match res {
                Ok(report) => {
                    e.status = LookupStatus::Completed;
                    e.report = Some(report);
                }
                Err(err) => {
                    e.status = LookupStatus::Failed;
                    e.error = Some(err.to_string());
                    let _ = bg_bcast.send(SpoorEvent::LookupFailed {
                        error: err.to_string(),
                    });
                }
            }
        }
    });

    (
        StatusCode::ACCEPTED,
        Json(LookupMetaResponse {
            id,
            target: target.display_name(),
            mode,
            enrich: Some(enrich),
            status: LookupStatus::Pending,
            created_at,
            error: None,
        }),
    )
        .into_response()
}

pub async fn get_lookup_handler(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<LookupMetaResponse>, (StatusCode, Json<ApiErrorResponse>)> {
    let map = state.lookups.read().await;
    let entry = map.get(&id).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ApiErrorResponse::new(
                "not_found",
                format!("Lookup with ID '{id}' was not found"),
            )),
        )
    })?;

    Ok(Json(LookupMetaResponse {
        id: entry.id,
        target: entry.target.display_name(),
        mode: entry.mode,
        enrich: Some(entry.enrich),
        status: entry.status,
        created_at: entry.created_at,
        error: entry.error.clone(),
    }))
}

pub async fn stream_events_handler(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, (StatusCode, Json<ApiErrorResponse>)>
{
    let map = state.lookups.read().await;
    let entry = map.get(&id).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ApiErrorResponse::new(
                "not_found",
                format!("Lookup with ID '{id}' was not found"),
            )),
        )
    })?;

    let rx = entry.event_tx.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|res| match res {
        Ok(event) => {
            let event_name = event.event_type();
            let json = serde_json::to_string(&event).unwrap_or_default();
            Some(Ok(Event::default().event(event_name).data(json)))
        }
        Err(_) => None,
    });

    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15))))
}

pub async fn get_report_handler(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<spoor_core::Report>, (StatusCode, Json<ApiErrorResponse>)> {
    let map = state.lookups.read().await;
    let entry = map.get(&id).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ApiErrorResponse::new(
                "not_found",
                format!("Lookup '{id}' not found"),
            )),
        )
    })?;

    match entry.status {
        LookupStatus::Completed => {
            if let Some(r) = &entry.report {
                Ok(Json(r.clone()))
            } else {
                Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResponse::new(
                        "internal_error",
                        "Report record missing",
                    )),
                ))
            }
        }
        LookupStatus::Failed => Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ApiErrorResponse::new(
                "lookup_failed",
                entry
                    .error
                    .clone()
                    .unwrap_or_else(|| "Lookup failed".into()),
            )),
        )),
        LookupStatus::Pending | LookupStatus::Running => Err((
            StatusCode::ACCEPTED,
            Json(ApiErrorResponse::new(
                "in_progress",
                "Lookup is currently running. Subscribe to /events or check back shortly.",
            )),
        )),
    }
}

pub async fn export_report_handler(
    State(state): State<AppState>,
    Path((id, format_str)): Path<(Uuid, String)>,
) -> Result<Response, (StatusCode, Json<ApiErrorResponse>)> {
    let fmt = ExportFormat::from_str_loose(&format_str).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResponse::new("unsupported_format", e.to_string())),
        )
    })?;

    let map = state.lookups.read().await;
    let entry = map.get(&id).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ApiErrorResponse::new(
                "not_found",
                format!("Lookup '{id}' not found"),
            )),
        )
    })?;

    let report = match &entry.report {
        Some(r) => r,
        None => {
            return Err((
                StatusCode::CONFLICT,
                Json(ApiErrorResponse::new(
                    "report_not_ready",
                    format!(
                        "Report is not ready yet (current status: {:?})",
                        entry.status
                    ),
                )),
            ));
        }
    };

    let bytes = export_bytes(report, fmt).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResponse::new("export_failed", e.to_string())),
        )
    })?;

    let filename = format!(
        "{}.{}",
        report
            .target
            .safe_filename_stem(report.mode, &report.generated_at),
        fmt.file_extension()
    );

    let mut headers = HeaderMap::new();
    headers.insert(
        CONTENT_TYPE,
        fmt.mime_type()
            .parse()
            .unwrap_or(axum::http::HeaderValue::from_static(
                "application/octet-stream",
            )),
    );
    headers.insert(
        CONTENT_DISPOSITION,
        format!("inline; filename=\"{filename}\"").parse().unwrap(),
    );

    Ok((StatusCode::OK, headers, bytes).into_response())
}
