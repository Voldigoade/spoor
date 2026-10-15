use chrono::Datelike;
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

#[derive(Deserialize, Debug, Clone)]
pub struct GhGraphQLUserContributions {
    pub user: Option<GhGraphQLUserContributionsNode>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct GhGraphQLUserContributionsNode {
    #[serde(rename = "contributionsCollection")]
    pub contributions_collection: GhGraphQLContributionsCollection,
}

#[derive(Deserialize, Debug, Clone)]
pub struct GhGraphQLContributionsCollection {
    #[serde(rename = "contributionYears", default)]
    pub contribution_years: Vec<i32>,
    #[serde(rename = "totalRepositoriesWithContributedCommits", default)]
    pub total_repositories_with_contributed_commits: usize,
    #[serde(rename = "commitContributionsByRepository", default)]
    pub commit_contributions_by_repository: Vec<GhGraphQLContributionByRepo>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct GhGraphQLContributionByRepo {
    pub repository: GhGraphQLRepoNode,
    pub contributions: Option<GhGraphQLContributionCount>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct GhGraphQLContributionCount {
    #[serde(rename = "totalCount")]
    pub total_count: u64,
}

#[derive(Deserialize, Debug, Clone)]
pub struct GhGraphQLRepoNode {
    #[serde(rename = "nameWithOwner")]
    pub name_with_owner: String,
    pub name: String,
    pub owner: GhGraphQLOwnerNode,
    #[serde(rename = "isPrivate", default)]
    pub is_private: bool,
    #[serde(rename = "isFork", default)]
    pub is_fork: bool,
    pub description: Option<String>,
    pub url: String,
}

#[derive(Deserialize, Debug, Clone)]
pub struct GhGraphQLOwnerNode {
    pub login: String,
}

#[derive(Debug, Clone)]
pub struct HistoricalContributionRepo {
    pub repository: GhGraphQLRepoNode,
    pub years: Vec<i32>,
    pub total_commits: u64,
}

#[derive(Debug, Clone)]
pub struct HistoricalContributionsResult {
    pub contributions: Vec<HistoricalContributionRepo>,
    pub years_discovered: Vec<i32>,
    pub years_queried: Vec<i32>,
    pub fully_traversed: bool,
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

    pub async fn get_user_commit_contributions(
        &self,
        login: &str,
    ) -> Result<Vec<GhGraphQLContributionByRepo>, SpoorError> {
        let query = r#"
            query($login: String!) {
              user(login: $login) {
                contributionsCollection {
                  contributionYears
                  commitContributionsByRepository(maxRepositories: 50) {
                    repository {
                      nameWithOwner
                      name
                      owner { login }
                      isPrivate
                      isFork
                      description
                      url
                    }
                    contributions {
                      totalCount
                    }
                  }
                }
              }
            }
        "#;

        let variables = serde_json::json!({ "login": login });
        let res: GhGraphQLUserContributions = self.query(query, variables).await?;
        Ok(res
            .user
            .map(|u| {
                u.contributions_collection
                    .commit_contributions_by_repository
            })
            .unwrap_or_default())
    }

    pub async fn get_user_commit_contributions_historical(
        &self,
        login: &str,
        max_historical_years: usize,
    ) -> Result<HistoricalContributionsResult, SpoorError> {
        let years_query = r#"
            query($login: String!) {
              user(login: $login) {
                contributionsCollection {
                  contributionYears
                }
              }
            }
        "#;

        let variables = serde_json::json!({ "login": login });
        let res: GhGraphQLUserContributions = self.query(years_query, variables).await?;

        let user_node = match res.user {
            Some(u) => u,
            None => {
                return Ok(HistoricalContributionsResult {
                    contributions: Vec::new(),
                    years_discovered: Vec::new(),
                    years_queried: Vec::new(),
                    fully_traversed: true,
                });
            }
        };

        let years_discovered = user_node
            .contributions_collection
            .contribution_years
            .clone();

        if years_discovered.is_empty() {
            return Ok(HistoricalContributionsResult {
                contributions: Vec::new(),
                years_discovered: Vec::new(),
                years_queried: Vec::new(),
                fully_traversed: true,
            });
        }

        let current_year = chrono::Utc::now().date_naive().year();
        let mut years_queried = Vec::new();
        let mut map: std::collections::HashMap<String, HistoricalContributionRepo> =
            std::collections::HashMap::new();
        let mut any_repository_truncated = false;

        let calendar_query = r#"
            query($login: String!, $from: DateTime!, $to: DateTime!) {
              user(login: $login) {
                contributionsCollection(from: $from, to: $to) {
                  totalRepositoriesWithContributedCommits
                  commitContributionsByRepository(maxRepositories: 100) {
                    repository {
                      nameWithOwner
                      name
                      owner { login }
                      isPrivate
                      isFork
                      description
                      url
                    }
                    contributions {
                      totalCount
                    }
                  }
                }
              }
            }
        "#;

        let years_to_process = years_discovered
            .iter()
            .copied()
            .take(max_historical_years.max(1))
            .collect::<Vec<_>>();

        for year in &years_to_process {
            let (from, to) = if *year >= current_year {
                (
                    format!("{year}-01-01T00:00:00Z"),
                    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
                )
            } else {
                (
                    format!("{year}-01-01T00:00:00Z"),
                    format!("{year}-12-31T23:59:59Z"),
                )
            };

            let cal_vars = serde_json::json!({
                "login": login,
                "from": from,
                "to": to,
            });

            match self
                .query::<GhGraphQLUserContributions>(calendar_query, cal_vars)
                .await
            {
                Ok(cal_res) => {
                    years_queried.push(*year);
                    if let Some(cal_user) = cal_res.user {
                        let coll = cal_user.contributions_collection;
                        if coll.total_repositories_with_contributed_commits
                            > coll.commit_contributions_by_repository.len()
                        {
                            any_repository_truncated = true;
                        }
                        for item in coll.commit_contributions_by_repository {
                            let commits = item.contributions.map(|c| c.total_count).unwrap_or(1);
                            let entry = map
                                .entry(item.repository.name_with_owner.clone())
                                .or_insert_with(|| HistoricalContributionRepo {
                                    repository: item.repository,
                                    years: Vec::new(),
                                    total_commits: 0,
                                });
                            if !entry.years.contains(year) {
                                entry.years.push(*year);
                            }
                            entry.total_commits += commits;
                        }
                    }
                }
                Err(_) => {
                    break;
                }
            }
        }

        let all_years_done = !years_discovered.is_empty()
            && years_discovered.iter().all(|y| years_queried.contains(y));
        let fully_traversed = all_years_done && !any_repository_truncated;

        Ok(HistoricalContributionsResult {
            contributions: map.into_values().collect(),
            years_discovered,
            years_queried,
            fully_traversed,
        })
    }
}
