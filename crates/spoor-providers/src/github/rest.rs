use std::collections::HashMap;

use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION};
use reqwest::{Method, StatusCode};
use spoor_core::SpoorError;
use tracing::debug;

use super::models::{
    GhCommitItem, GhCommitSearchResult, GhCommunityProfile, GhEvent, GhGpgKey, GhRepo, GhSshKey,
    GhTreeResponse, GhUser,
};
use crate::client::HttpClient;

#[derive(Clone)]
pub struct GitHubRestClient {
    client: HttpClient,
    token: Option<String>,
    base_url: String,
}

impl GitHubRestClient {
    pub fn new(client: HttpClient, token: Option<String>, base_url: Option<String>) -> Self {
        Self {
            client,
            token,
            base_url: base_url.unwrap_or_else(|| "https://api.github.com".to_string()),
        }
    }

    pub fn rate_limit_remaining(&self) -> u64 {
        self.client.rate_limit.remaining()
    }

    fn headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            ACCEPT,
            HeaderValue::from_static("application/vnd.github+json"),
        );
        headers.insert(
            "X-GitHub-Api-Version",
            HeaderValue::from_static("2022-11-28"),
        );
        if let Some(token) = &self.token {
            if let Ok(val) = HeaderValue::from_str(&format!("Bearer {token}")) {
                headers.insert(AUTHORIZATION, val);
            }
        }
        headers
    }

    pub async fn get_user(&self, username: &str) -> Result<GhUser, SpoorError> {
        let url = format!("{}/users/{username}", self.base_url);
        let headers = self.headers();

        let resp = self
            .client
            .send_with_retry(
                || {
                    self.client
                        .request(Method::GET, &url)
                        .headers(headers.clone())
                },
                2,
            )
            .await?;

        let status = resp.status();
        if status == StatusCode::NOT_FOUND {
            return Err(SpoorError::UserNotFound(username.to_string()));
        }
        if status == StatusCode::FORBIDDEN || status == StatusCode::TOO_MANY_REQUESTS {
            return Err(SpoorError::RateLimitExceeded {
                provider: "GitHub REST".to_string(),
                reset_at: self.client.rate_limit.reset_at(),
                retry_after: None,
            });
        }
        if !status.is_success() {
            return Err(SpoorError::ProviderError {
                provider: "GitHub REST".to_string(),
                message: format!("User lookup returned HTTP status {status}"),
            });
        }

        let body = HttpClient::read_body_limited(resp).await?;
        serde_json::from_str::<GhUser>(&body).map_err(|e| SpoorError::Serialization(e.to_string()))
    }

    pub async fn get_user_repos(
        &self,
        username: &str,
        max_repos: usize,
    ) -> Result<Vec<GhRepo>, SpoorError> {
        let mut repos = Vec::new();
        let mut page = 1;
        let per_page = 100.min(max_repos);

        while repos.len() < max_repos {
            let url = format!(
                "{}/users/{username}/repos?per_page={per_page}&page={page}&sort=pushed&type=public",
                self.base_url
            );
            let headers = self.headers();

            let resp = self
                .client
                .send_with_retry(
                    || {
                        self.client
                            .request(Method::GET, &url)
                            .headers(headers.clone())
                    },
                    2,
                )
                .await?;

            if !resp.status().is_success() {
                debug!("Failed to fetch user repos page {page}: {}", resp.status());
                break;
            }

            let body = HttpClient::read_body_limited(resp).await?;
            let page_repos: Vec<GhRepo> = serde_json::from_str(&body).unwrap_or_default();
            let page_len = page_repos.len();
            if page_len == 0 {
                break;
            }

            let needed = max_repos - repos.len();
            if page_len <= needed {
                repos.extend(page_repos);
            } else {
                repos.extend(page_repos.into_iter().take(needed));
                break;
            }

            if page_len < per_page {
                break;
            }

            page += 1;
        }

        Ok(repos)
    }

    pub async fn get_repo(&self, owner: &str, repo: &str) -> Result<GhRepo, SpoorError> {
        let url = format!("{}/repos/{owner}/{repo}", self.base_url);
        let headers = self.headers();

        let resp = self
            .client
            .send_with_retry(
                || {
                    self.client
                        .request(Method::GET, &url)
                        .headers(headers.clone())
                },
                2,
            )
            .await?;

        let status = resp.status();
        if status == StatusCode::NOT_FOUND {
            return Err(SpoorError::RepoNotFound(format!("{owner}/{repo}")));
        }
        if status == StatusCode::FORBIDDEN || status == StatusCode::TOO_MANY_REQUESTS {
            return Err(SpoorError::RateLimitExceeded {
                provider: "GitHub REST".to_string(),
                reset_at: self.client.rate_limit.reset_at(),
                retry_after: None,
            });
        }
        if !status.is_success() {
            return Err(SpoorError::ProviderError {
                provider: "GitHub REST".to_string(),
                message: format!("Repository lookup returned HTTP status {status}"),
            });
        }

        let body = HttpClient::read_body_limited(resp).await?;
        serde_json::from_str::<GhRepo>(&body).map_err(|e| SpoorError::Serialization(e.to_string()))
    }

    pub async fn search_commits(
        &self,
        query: &str,
        page: usize,
        per_page: usize,
    ) -> Result<GhCommitSearchResult, SpoorError> {
        let per_page = per_page.clamp(1, 100);
        let encoded_query =
            url::form_urlencoded::byte_serialize(query.as_bytes()).collect::<String>();
        let url = format!(
            "{}/search/commits?q={encoded_query}&page={page}&per_page={per_page}&sort=committer-date&order=desc",
            self.base_url
        );
        let headers = self.headers();

        let resp = self
            .client
            .send_with_retry(
                || {
                    self.client
                        .request(Method::GET, &url)
                        .headers(headers.clone())
                },
                2,
            )
            .await?;

        let status = resp.status();
        if status == StatusCode::FORBIDDEN || status == StatusCode::TOO_MANY_REQUESTS {
            return Err(SpoorError::RateLimitExceeded {
                provider: "GitHub REST (Search)".to_string(),
                reset_at: self.client.rate_limit.reset_at(),
                retry_after: None,
            });
        }
        if !status.is_success() {
            return Err(SpoorError::ProviderError {
                provider: "GitHub REST (Search)".to_string(),
                message: format!("Commit search returned HTTP status {status}"),
            });
        }

        let body = HttpClient::read_body_limited(resp).await?;
        serde_json::from_str::<GhCommitSearchResult>(&body).map_err(|e| {
            SpoorError::Serialization(format!("Failed to parse commit search result: {e}"))
        })
    }

    pub async fn get_repo_commits(
        &self,
        owner: &str,
        repo: &str,
        limit: usize,
    ) -> Result<Vec<GhCommitItem>, SpoorError> {
        let per_page = 100.min(limit);
        let url = format!(
            "{}/repos/{owner}/{repo}/commits?per_page={per_page}",
            self.base_url
        );
        let headers = self.headers();

        let resp = self
            .client
            .send_with_retry(
                || {
                    self.client
                        .request(Method::GET, &url)
                        .headers(headers.clone())
                },
                2,
            )
            .await?;

        let status = resp.status();
        if status == StatusCode::NOT_FOUND || status == StatusCode::CONFLICT {
            return Ok(Vec::new());
        }
        if status == StatusCode::FORBIDDEN || status == StatusCode::TOO_MANY_REQUESTS {
            return Err(SpoorError::RateLimitExceeded {
                provider: "GitHub REST".to_string(),
                reset_at: self.client.rate_limit.reset_at(),
                retry_after: None,
            });
        }
        if !status.is_success() {
            return Err(SpoorError::ProviderError {
                provider: "GitHub REST".to_string(),
                message: format!("Commit lookup for {owner}/{repo} returned HTTP status {status}"),
            });
        }

        let body = HttpClient::read_body_limited(resp).await?;
        let mut commits: Vec<GhCommitItem> = serde_json::from_str(&body).map_err(|e| {
            SpoorError::Serialization(format!("Failed to parse commits for {owner}/{repo}: {e}"))
        })?;
        if commits.len() > limit {
            commits.truncate(limit);
        }
        Ok(commits)
    }

    pub async fn get_user_events(
        &self,
        username: &str,
        limit: usize,
    ) -> Result<Vec<GhEvent>, SpoorError> {
        let per_page = 100.min(limit);
        let url = format!(
            "{}/users/{username}/events/public?per_page={per_page}",
            self.base_url
        );
        let headers = self.headers();

        let resp = self
            .client
            .send_with_retry(
                || {
                    self.client
                        .request(Method::GET, &url)
                        .headers(headers.clone())
                },
                2,
            )
            .await?;

        let status = resp.status();
        if status == StatusCode::NOT_FOUND {
            return Ok(Vec::new());
        }
        if status == StatusCode::FORBIDDEN || status == StatusCode::TOO_MANY_REQUESTS {
            return Err(SpoorError::RateLimitExceeded {
                provider: "GitHub REST".to_string(),
                reset_at: self.client.rate_limit.reset_at(),
                retry_after: None,
            });
        }
        if !status.is_success() {
            return Err(SpoorError::ProviderError {
                provider: "GitHub REST".to_string(),
                message: format!("Events lookup for {username} returned HTTP status {status}"),
            });
        }

        let body = HttpClient::read_body_limited(resp).await?;
        let mut events: Vec<GhEvent> = serde_json::from_str(&body).map_err(|e| {
            SpoorError::Serialization(format!("Failed to parse events for {username}: {e}"))
        })?;
        if events.len() > limit {
            events.truncate(limit);
        }
        Ok(events)
    }

    pub async fn get_user_gpg_keys(&self, username: &str) -> Result<Vec<GhGpgKey>, SpoorError> {
        let url = format!("{}/users/{username}/gpg_keys", self.base_url);
        let headers = self.headers();

        let resp = self
            .client
            .send_with_retry(
                || {
                    self.client
                        .request(Method::GET, &url)
                        .headers(headers.clone())
                },
                2,
            )
            .await?;

        let status = resp.status();
        if status == StatusCode::NOT_FOUND {
            return Ok(Vec::new());
        }
        if status == StatusCode::FORBIDDEN || status == StatusCode::TOO_MANY_REQUESTS {
            return Err(SpoorError::RateLimitExceeded {
                provider: "GitHub REST".to_string(),
                reset_at: self.client.rate_limit.reset_at(),
                retry_after: None,
            });
        }
        if !status.is_success() {
            return Err(SpoorError::ProviderError {
                provider: "GitHub REST".to_string(),
                message: format!("GPG keys lookup for {username} returned HTTP status {status}"),
            });
        }

        let body = HttpClient::read_body_limited(resp).await?;
        serde_json::from_str(&body).map_err(|e| {
            SpoorError::Serialization(format!("Failed to parse GPG keys for {username}: {e}"))
        })
    }

    pub async fn get_user_ssh_keys(&self, username: &str) -> Result<Vec<GhSshKey>, SpoorError> {
        let url = format!("{}/users/{username}/keys", self.base_url);
        let headers = self.headers();

        let resp = self
            .client
            .send_with_retry(
                || {
                    self.client
                        .request(Method::GET, &url)
                        .headers(headers.clone())
                },
                2,
            )
            .await?;

        let status = resp.status();
        if status == StatusCode::NOT_FOUND {
            return Ok(Vec::new());
        }
        if status == StatusCode::FORBIDDEN || status == StatusCode::TOO_MANY_REQUESTS {
            return Err(SpoorError::RateLimitExceeded {
                provider: "GitHub REST".to_string(),
                reset_at: self.client.rate_limit.reset_at(),
                retry_after: None,
            });
        }
        if !status.is_success() {
            return Err(SpoorError::ProviderError {
                provider: "GitHub REST".to_string(),
                message: format!("SSH keys lookup for {username} returned HTTP status {status}"),
            });
        }

        let body = HttpClient::read_body_limited(resp).await?;
        serde_json::from_str(&body).map_err(|e| {
            SpoorError::Serialization(format!("Failed to parse SSH keys for {username}: {e}"))
        })
    }

    pub async fn get_repo_languages(
        &self,
        owner: &str,
        repo: &str,
    ) -> Result<HashMap<String, u64>, SpoorError> {
        let url = format!("{}/repos/{owner}/{repo}/languages", self.base_url);
        let headers = self.headers();

        let resp = self
            .client
            .send_with_retry(
                || {
                    self.client
                        .request(Method::GET, &url)
                        .headers(headers.clone())
                },
                2,
            )
            .await?;

        let status = resp.status();
        if status == StatusCode::NOT_FOUND {
            return Ok(HashMap::new());
        }
        if status == StatusCode::FORBIDDEN || status == StatusCode::TOO_MANY_REQUESTS {
            return Err(SpoorError::RateLimitExceeded {
                provider: "GitHub REST".to_string(),
                reset_at: self.client.rate_limit.reset_at(),
                retry_after: None,
            });
        }
        if !status.is_success() {
            return Err(SpoorError::ProviderError {
                provider: "GitHub REST".to_string(),
                message: format!(
                    "Languages lookup for {owner}/{repo} returned HTTP status {status}"
                ),
            });
        }

        let body = HttpClient::read_body_limited(resp).await?;
        serde_json::from_str(&body).map_err(|e| {
            SpoorError::Serialization(format!("Failed to parse languages for {owner}/{repo}: {e}"))
        })
    }

    pub async fn get_repo_community_profile(
        &self,
        owner: &str,
        repo: &str,
    ) -> Result<GhCommunityProfile, SpoorError> {
        let url = format!("{}/repos/{owner}/{repo}/community/profile", self.base_url);
        let headers = self.headers();

        let resp = self
            .client
            .send_with_retry(
                || {
                    self.client
                        .request(Method::GET, &url)
                        .headers(headers.clone())
                },
                2,
            )
            .await?;

        let status = resp.status();
        if status == StatusCode::NOT_FOUND {
            return Ok(GhCommunityProfile::default());
        }
        if status == StatusCode::FORBIDDEN || status == StatusCode::TOO_MANY_REQUESTS {
            return Err(SpoorError::RateLimitExceeded {
                provider: "GitHub REST".to_string(),
                reset_at: self.client.rate_limit.reset_at(),
                retry_after: None,
            });
        }
        if !status.is_success() {
            return Err(SpoorError::ProviderError {
                provider: "GitHub REST".to_string(),
                message: format!(
                    "Community profile for {owner}/{repo} returned HTTP status {status}"
                ),
            });
        }

        let body = HttpClient::read_body_limited(resp).await?;
        serde_json::from_str(&body).map_err(|e| {
            SpoorError::Serialization(format!(
                "Failed to parse community profile for {owner}/{repo}: {e}"
            ))
        })
    }

    pub async fn get_repo_tree(
        &self,
        owner: &str,
        repo: &str,
        branch: &str,
    ) -> Result<GhTreeResponse, SpoorError> {
        let url = format!(
            "{}/repos/{owner}/{repo}/git/trees/{branch}?recursive=1",
            self.base_url
        );
        let headers = self.headers();

        let resp = self
            .client
            .send_with_retry(
                || {
                    self.client
                        .request(Method::GET, &url)
                        .headers(headers.clone())
                },
                2,
            )
            .await?;

        let status = resp.status();
        if status == StatusCode::NOT_FOUND {
            return Ok(GhTreeResponse::default());
        }
        if status == StatusCode::FORBIDDEN || status == StatusCode::TOO_MANY_REQUESTS {
            return Err(SpoorError::RateLimitExceeded {
                provider: "GitHub REST".to_string(),
                reset_at: self.client.rate_limit.reset_at(),
                retry_after: None,
            });
        }
        if !status.is_success() {
            return Err(SpoorError::ProviderError {
                provider: "GitHub REST".to_string(),
                message: format!("Git tree for {owner}/{repo} returned HTTP status {status}"),
            });
        }

        let body = HttpClient::read_body_limited(resp).await?;
        serde_json::from_str(&body).map_err(|e| {
            SpoorError::Serialization(format!("Failed to parse git tree for {owner}/{repo}: {e}"))
        })
    }
}
