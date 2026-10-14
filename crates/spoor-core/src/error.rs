use thiserror::Error;

#[derive(Error, Debug)]
pub enum SpoorError {
    #[error("Invalid target '{input}': {reason}")]
    InvalidTarget { input: String, reason: String },

    #[error("Target rejected for security (SSRF prevention): {0}")]
    SsrfProtection(String),

    #[error("GitHub user not found: {0}")]
    UserNotFound(String),

    #[error("GitHub repository not found: {0}")]
    RepoNotFound(String),

    #[error("Provider '{provider}' rate limit exceeded. Reset at {reset_at:?}, retry after {retry_after:?}s")]
    RateLimitExceeded {
        provider: String,
        reset_at: Option<chrono::DateTime<chrono::Utc>>,
        retry_after: Option<u64>,
    },

    #[error("Provider '{provider}' error: {message}")]
    ProviderError { provider: String, message: String },

    #[error("Upstream timeout from provider '{provider}' after {timeout_secs}s")]
    Timeout { provider: String, timeout_secs: u64 },

    #[error("Cache error: {0}")]
    Cache(String),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Export error: {0}")]
    Export(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Internal error: {0}")]
    Internal(String),
}
