use std::convert::Infallible;
use std::net::SocketAddr;
use std::time::Duration;

use axum::extract::{ConnectInfo, Path, State};
use axum::http::header::{CONTENT_DISPOSITION, CONTENT_TYPE};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use spoor_core::{AnalysisMode, SpoorEvent, Target};
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
}

#[derive(Serialize)]
pub struct LookupMetaResponse {
    pub id: Uuid,
    pub target: String,
    pub mode: AnalysisMode,
    pub status: LookupStatus,
    pub created_at: chrono::DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Serialize)]
pub struct ApiErrorResponse {
    pub error: String,
    pub code: &'static str,
}

pub async fn create_lookup_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(payload): Json<CreateLookupRequest>,
) -> Result<(StatusCode, Json<LookupMetaResponse>), (StatusCode, Json<ApiErrorResponse>)> {
    // 1. IP rate limiting (extract IP using trusted proxy resolver)
    let client_ip = state.proxy_config.resolve_client_ip(addr.ip(), &headers);

    let allowed = {
        let mut limiter = state.limiter.lock().await;
        limiter.check_and_record(&client_ip.to_string())
    };

    if !allowed {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            Json(ApiErrorResponse {
                error: "Rate limit exceeded. Maximum 30 lookups per minute.".to_string(),
                code: "rate_limit_exceeded",
            }),
        ));
    }

    // 2. Parse target with strict SSRF validation
    let target = Target::parse(&payload.target).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResponse {
                error: e.to_string(),
                code: "invalid_target",
            }),
        )
    })?;

    let mode = payload.mode.unwrap_or_default();
    let id = Uuid::new_v4();
    let created_at = Utc::now();

    let (broadcast_tx, _) = broadcast::channel(128);

    let entry = LookupEntry {
        id,
        target: target.clone(),
        mode,
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

    // 3. Spawn background execution
    let bg_state = state.clone();
    let bg_target = target.clone();
    let bg_id = id;
    let bg_mode = mode;
    let bg_bcast = broadcast_tx;

    tokio::spawn(async move {
        // Mark status as running
        {
            let mut map = bg_state.lookups.write().await;
            if let Some(e) = map.get_mut(&bg_id) {
                e.status = LookupStatus::Running;
            }
        }

        // Channel bridge for progress events
        let (mpsc_tx, mut mpsc_rx) = mpsc::channel::<SpoorEvent>(64);
        let bcast_forward = bg_bcast.clone();
        tokio::spawn(async move {
            while let Some(ev) = mpsc_rx.recv().await {
                let _ = bcast_forward.send(ev);
            }
        });

        // Bounded concurrency
        let _permit = bg_state.concurrency_semaphore.acquire().await;
        let res = bg_state
            .engine
            .run_analysis(&bg_target, bg_mode, Some(mpsc_tx))
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

    Ok((
        StatusCode::ACCEPTED,
        Json(LookupMetaResponse {
            id,
            target: target.display_name(),
            mode,
            status: LookupStatus::Pending,
            created_at,
            error: None,
        }),
    ))
}

pub async fn get_lookup_handler(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<LookupMetaResponse>, (StatusCode, Json<ApiErrorResponse>)> {
    let map = state.lookups.read().await;
    let entry = map.get(&id).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ApiErrorResponse {
                error: format!("Lookup with ID '{id}' was not found"),
                code: "not_found",
            }),
        )
    })?;

    Ok(Json(LookupMetaResponse {
        id: entry.id,
        target: entry.target.display_name(),
        mode: entry.mode,
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
            Json(ApiErrorResponse {
                error: format!("Lookup with ID '{id}' was not found"),
                code: "not_found",
            }),
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
            Json(ApiErrorResponse {
                error: format!("Lookup '{id}' not found"),
                code: "not_found",
            }),
        )
    })?;

    match entry.status {
        LookupStatus::Completed => {
            if let Some(r) = &entry.report {
                Ok(Json(r.clone()))
            } else {
                Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResponse {
                        error: "Report record missing".to_string(),
                        code: "internal_error",
                    }),
                ))
            }
        }
        LookupStatus::Failed => Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ApiErrorResponse {
                error: entry
                    .error
                    .clone()
                    .unwrap_or_else(|| "Lookup failed".into()),
                code: "lookup_failed",
            }),
        )),
        LookupStatus::Pending | LookupStatus::Running => Err((
            StatusCode::ACCEPTED,
            Json(ApiErrorResponse {
                error: "Lookup is currently running. Subscribe to /events or check back shortly."
                    .to_string(),
                code: "in_progress",
            }),
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
            Json(ApiErrorResponse {
                error: e.to_string(),
                code: "unsupported_format",
            }),
        )
    })?;

    let map = state.lookups.read().await;
    let entry = map.get(&id).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ApiErrorResponse {
                error: format!("Lookup '{id}' not found"),
                code: "not_found",
            }),
        )
    })?;

    let report = match &entry.report {
        Some(r) => r,
        None => {
            return Err((
                StatusCode::CONFLICT,
                Json(ApiErrorResponse {
                    error: format!(
                        "Report is not ready yet (current status: {:?})",
                        entry.status
                    ),
                    code: "report_not_ready",
                }),
            ));
        }
    };

    let bytes = export_bytes(report, fmt).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResponse {
                error: e.to_string(),
                code: "export_failed",
            }),
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
