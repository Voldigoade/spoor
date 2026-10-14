use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION, USER_AGENT};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use spoor_core::SpoorError;

use crate::client::{HttpClient, DEFAULT_USER_AGENT};

#[derive(Clone)]
pub struct GitHubGraphQLClient {
    client: HttpClient,
    token: Option<String>,
    endpoint: String,
}

#[derive(Serialize)]
struct GraphQLRequest<'a> {
    query: &'a str,
    variables: serde_json::Value,
}

#[derive(Deserialize, Debug)]
pub struct GraphQLResponse<T> {
    pub data: Option<T>,
    pub errors: Option<Vec<GraphQLError>>,
}

#[derive(Deserialize, Debug)]
pub struct GraphQLError {
    pub message: String,
}

#[derive(Deserialize, Debug)]
pub struct RateLimitData {
    pub limit: u64,
    pub cost: u64,
    pub remaining: u64,
    #[serde(rename = "resetAt")]
    pub reset_at: String,
}

impl GitHubGraphQLClient {
    pub fn new(client: HttpClient, token: Option<String>, endpoint: Option<String>) -> Self {
        Self {
            client,
            token,
            endpoint: endpoint.unwrap_or_else(|| "https://api.github.com/graphql".to_string()),
        }
    }

    pub fn is_authenticated(&self) -> bool {
        self.token.as_ref().is_some_and(|t| !t.trim().is_empty())
    }

    pub async fn query<T: for<'de> Deserialize<'de>>(
        &self,
        query: &str,
        variables: serde_json::Value,
    ) -> Result<T, SpoorError> {
        let token = self
            .token
            .as_ref()
            .ok_or_else(|| SpoorError::ProviderError {
                provider: "GitHub GraphQL".to_string(),
                message: "No GitHub token provided; GraphQL requires authentication".to_string(),
            })?;

        let mut headers = HeaderMap::new();
        headers.insert(USER_AGENT, HeaderValue::from_static(DEFAULT_USER_AGENT));
        headers.insert(
            ACCEPT,
            HeaderValue::from_static("application/vnd.github+json"),
        );
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {token}"))
                .map_err(|e| SpoorError::Config(format!("Invalid GitHub token formatting: {e}")))?,
        );

        let body = GraphQLRequest { query, variables };

        let resp = self
            .client
            .send_with_retry(
                || {
                    self.client
                        .request(Method::POST, &self.endpoint)
                        .headers(headers.clone())
                        .json(&body)
                },
                2,
            )
            .await?;

        if !resp.status().is_success() {
            return Err(SpoorError::ProviderError {
                provider: "GitHub GraphQL".to_string(),
                message: format!("GraphQL query failed with status {}", resp.status()),
            });
        }

        let resp_body = HttpClient::read_body_limited(resp).await?;
        let parsed: GraphQLResponse<T> = serde_json::from_str(&resp_body).map_err(|e| {
            SpoorError::Serialization(format!("GraphQL response decoding error: {e}"))
        })?;

        if let Some(errors) = parsed.errors {
            if !errors.is_empty() {
                return Err(SpoorError::ProviderError {
                    provider: "GitHub GraphQL".to_string(),
                    message: errors
                        .into_iter()
                        .map(|e| e.message)
                        .collect::<Vec<_>>()
                        .join(", "),
                });
            }
        }

        parsed.data.ok_or_else(|| SpoorError::ProviderError {
            provider: "GitHub GraphQL".to_string(),
            message: "Empty GraphQL data response".to_string(),
        })
    }
}
