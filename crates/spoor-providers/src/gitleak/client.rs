use reqwest::{Method, StatusCode};
use spoor_core::SpoorError;
use tracing::{debug, warn};

use super::models::GitLeakResponse;
use crate::client::HttpClient;

#[derive(Debug, Clone, PartialEq)]
pub enum GitLeakOutcome {
    Success(Box<GitLeakResponse>),
    RateLimited {
        retry_after: Option<u64>,
        message: String,
    },
    NoActivity,
    NotFound,
    TimedOut,
    Unavailable(String),
}

#[derive(Clone)]
pub struct GitLeakClient {
    client: HttpClient,
    base_url: String,
}

impl GitLeakClient {
    pub fn new(client: HttpClient, base_url: Option<String>) -> Self {
        Self {
            client,
            base_url: base_url.unwrap_or_else(|| "https://gitleak.io".to_string()),
        }
    }

    pub async fn lookup(
        &self,
        username: &str,
        include_noreply: bool,
        include_timezone: bool,
    ) -> Result<GitLeakOutcome, SpoorError> {
        let url = format!(
            "{}/api/v1/users/{}?includeNoreply={}&includeTimezone={}",
            self.base_url, username, include_noreply, include_timezone
        );

        let resp = match self.client.request(Method::GET, &url).send().await {
            Ok(r) => r,
            Err(e) => {
                warn!("GitLeak network request failed: {e}");
                return Ok(GitLeakOutcome::Unavailable(e.to_string()));
            }
        };

        let status = resp.status();
        let headers = resp.headers().clone();

        match status {
            StatusCode::OK => {
                let body = HttpClient::read_body_limited(resp).await?;
                match serde_json::from_str::<GitLeakResponse>(&body) {
                    Ok(parsed) => {
                        // Check if upstream returned an error message inside 200 (e.g. GitHub rate limit exceeded)
                        if let Some(msg) = &parsed.message {
                            if msg.to_lowercase().contains("rate limit") {
                                return Ok(GitLeakOutcome::RateLimited {
                                    retry_after: None,
                                    message: msg.clone(),
                                });
                            }
                        }
                        Ok(GitLeakOutcome::Success(Box::new(parsed)))
                    }
                    Err(e) => {
                        debug!("Failed deserializing GitLeak payload: {e}");
                        Ok(GitLeakOutcome::Unavailable(format!(
                            "Invalid JSON returned: {e}"
                        )))
                    }
                }
            }
            StatusCode::NOT_FOUND => Ok(GitLeakOutcome::NotFound),
            StatusCode::UNPROCESSABLE_ENTITY => Ok(GitLeakOutcome::NoActivity),
            StatusCode::REQUEST_TIMEOUT => Ok(GitLeakOutcome::TimedOut),
            StatusCode::TOO_MANY_REQUESTS => {
                let retry_after = headers
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.parse::<u64>().ok());
                Ok(GitLeakOutcome::RateLimited {
                    retry_after,
                    message: "GitLeak 50 requests/hour limit exceeded".to_string(),
                })
            }
            StatusCode::UNAUTHORIZED => Ok(GitLeakOutcome::Unavailable(
                "GitLeak server GitHub token not configured (401)".to_string(),
            )),
            StatusCode::BAD_REQUEST => Ok(GitLeakOutcome::Unavailable(
                "GitLeak rejected request as invalid (400)".to_string(),
            )),
            _ => {
                let msg = format!("GitLeak returned unexpected HTTP {status}");
                Ok(GitLeakOutcome::Unavailable(msg))
            }
        }
    }
}
