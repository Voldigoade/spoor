use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, TimeZone, Utc};
use reqwest::header::{HeaderMap, HeaderValue, RETRY_AFTER, USER_AGENT};
use reqwest::{Client, Method, RequestBuilder, Response, StatusCode};
use spoor_core::SpoorError;
use tracing::{debug, warn};

pub const DEFAULT_USER_AGENT: &str = "Spoor/0.1.0 (+https://github.com/Voldigoade/spoor)";
pub const MAX_RESPONSE_BYTES: usize = 10 * 1024 * 1024; // 10 MB

#[derive(Debug, Clone, Default)]
pub struct RateLimitState {
    pub limit: Arc<AtomicU64>,
    pub remaining: Arc<AtomicU64>,
    pub reset_epoch: Arc<AtomicU64>,
}

impl RateLimitState {
    pub fn new() -> Self {
        Self {
            limit: Arc::new(AtomicU64::new(0)),
            remaining: Arc::new(AtomicU64::new(u64::MAX)),
            reset_epoch: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn update_from_headers(&self, headers: &HeaderMap) {
        if let Some(val) = headers.get("x-ratelimit-limit") {
            if let Ok(s) = val.to_str() {
                if let Ok(num) = s.parse::<u64>() {
                    self.limit.store(num, Ordering::Relaxed);
                }
            }
        }
        if let Some(val) = headers.get("x-ratelimit-remaining") {
            if let Ok(s) = val.to_str() {
                if let Ok(num) = s.parse::<u64>() {
                    self.remaining.store(num, Ordering::Relaxed);
                }
            }
        }
        if let Some(val) = headers.get("x-ratelimit-reset") {
            if let Ok(s) = val.to_str() {
                if let Ok(num) = s.parse::<u64>() {
                    self.reset_epoch.store(num, Ordering::Relaxed);
                }
            }
        }
    }

    pub fn remaining(&self) -> u64 {
        self.remaining.load(Ordering::Relaxed)
    }

    pub fn reset_at(&self) -> Option<DateTime<Utc>> {
        let epoch = self.reset_epoch.load(Ordering::Relaxed);
        if epoch > 0 {
            Utc.timestamp_opt(epoch as i64, 0).single()
        } else {
            None
        }
    }
}

#[derive(Clone)]
pub struct HttpClient {
    inner: Client,
    pub rate_limit: RateLimitState,
}

impl Default for HttpClient {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpClient {
    pub fn new() -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(12))
            .pool_idle_timeout(Duration::from_secs(30))
            .pool_max_idle_per_host(10)
            .build()
            .unwrap_or_else(|_| Client::new());

        Self {
            inner: client,
            rate_limit: RateLimitState::new(),
        }
    }

    pub fn request(&self, method: Method, url: &str) -> RequestBuilder {
        self.inner
            .request(method, url)
            .header(USER_AGENT, HeaderValue::from_static(DEFAULT_USER_AGENT))
    }

    pub async fn send_with_retry(
        &self,
        mut req_builder_factory: impl FnMut() -> RequestBuilder,
        max_retries: usize,
    ) -> Result<Response, SpoorError> {
        let mut attempts = 0;
        loop {
            attempts += 1;
            let req = req_builder_factory();
            match req.send().await {
                Ok(resp) => {
                    self.rate_limit.update_from_headers(resp.headers());

                    let status = resp.status();
                    if status.is_success() {
                        return Ok(resp);
                    }

                    // Handle rate limits or temporary service unavailability
                    if (status == StatusCode::TOO_MANY_REQUESTS
                        || status == StatusCode::SERVICE_UNAVAILABLE)
                        && attempts <= max_retries
                    {
                        let retry_after = parse_retry_after(resp.headers());
                        let backoff = retry_after
                            .map(Duration::from_secs)
                            .unwrap_or_else(|| Duration::from_millis(500 * (1 << attempts)));

                        warn!(
                            "Received status {}, backing off for {:?}",
                            status.as_u16(),
                            backoff
                        );
                        tokio::time::sleep(backoff).await;
                        continue;
                    }

                    return Ok(resp);
                }
                Err(e) if attempts <= max_retries && (e.is_timeout() || e.is_connect()) => {
                    let backoff = Duration::from_millis(500 * (1 << attempts));
                    debug!("Request failed with {}, retrying in {:?}", e, backoff);
                    tokio::time::sleep(backoff).await;
                    continue;
                }
                Err(e) => {
                    return Err(SpoorError::ProviderError {
                        provider: "HTTP".to_string(),
                        message: e.to_string(),
                    });
                }
            }
        }
    }

    pub async fn read_body_limited(resp: Response) -> Result<String, SpoorError> {
        if let Some(len) = resp.content_length() {
            if len as usize > MAX_RESPONSE_BYTES {
                return Err(SpoorError::ProviderError {
                    provider: "HTTP".to_string(),
                    message: format!(
                        "Response size {len} bytes exceeds maximum limit of {MAX_RESPONSE_BYTES} bytes"
                    ),
                });
            }
        }

        let bytes = resp.bytes().await.map_err(|e| SpoorError::ProviderError {
            provider: "HTTP".to_string(),
            message: format!("Failed reading response bytes: {e}"),
        })?;

        if bytes.len() > MAX_RESPONSE_BYTES {
            return Err(SpoorError::ProviderError {
                provider: "HTTP".to_string(),
                message: format!(
                    "Read {} bytes which exceeds max limit of {MAX_RESPONSE_BYTES}",
                    bytes.len()
                ),
            });
        }

        String::from_utf8(bytes.to_vec()).map_err(|e| SpoorError::ProviderError {
            provider: "HTTP".to_string(),
            message: format!("Response was not valid UTF-8: {e}"),
        })
    }
}

fn parse_retry_after(headers: &HeaderMap) -> Option<u64> {
    if let Some(val) = headers.get(RETRY_AFTER) {
        if let Ok(s) = val.to_str() {
            if let Ok(seconds) = s.parse::<u64>() {
                return Some(seconds);
            }
        }
    }
    None
}
