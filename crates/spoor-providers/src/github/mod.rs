pub mod graphql;
pub mod models;
pub mod rest;

use std::collections::HashMap;

use spoor_core::SpoorError;

use self::graphql::GitHubGraphQLClient;
use self::models::{
    GhCommitItem, GhCommitSearchResult, GhCommunityProfile, GhEvent, GhGpgKey, GhRepo, GhSshKey,
    GhTreeResponse, GhUser,
};
use self::rest::GitHubRestClient;
use crate::client::HttpClient;

#[derive(Clone)]
pub struct GitHubProvider {
    pub rest: GitHubRestClient,
    pub graphql: GitHubGraphQLClient,
}

impl GitHubProvider {
    pub fn new(
        client: HttpClient,
        token: Option<String>,
        rest_base_url: Option<String>,
        graphql_endpoint: Option<String>,
    ) -> Self {
        Self {
            rest: GitHubRestClient::new(client.clone(), token.clone(), rest_base_url),
            graphql: GitHubGraphQLClient::new(client, token, graphql_endpoint),
        }
    }

    pub fn from_env(client: HttpClient) -> Self {
        let token = std::env::var("GITHUB_TOKEN")
            .ok()
            .filter(|t| !t.trim().is_empty());
        Self::new(client, token, None, None)
    }

    pub fn is_authenticated(&self) -> bool {
        self.graphql.is_authenticated()
    }

    pub async fn fetch_user_profile(&self, username: &str) -> Result<GhUser, SpoorError> {
        self.rest.get_user(username).await
    }

    pub async fn fetch_user_repositories(
        &self,
        username: &str,
        limit: usize,
    ) -> Result<Vec<GhRepo>, SpoorError> {
        self.rest.get_user_repos(username, limit).await
    }

    pub async fn fetch_repository(&self, owner: &str, repo: &str) -> Result<GhRepo, SpoorError> {
        self.rest.get_repo(owner, repo).await
    }

    pub async fn fetch_repository_commits(
        &self,
        owner: &str,
        repo: &str,
        limit: usize,
    ) -> Result<Vec<GhCommitItem>, SpoorError> {
        self.rest.get_repo_commits(owner, repo, limit).await
    }

    pub async fn fetch_user_events(
        &self,
        username: &str,
        limit: usize,
    ) -> Result<Vec<GhEvent>, SpoorError> {
        self.rest.get_user_events(username, limit).await
    }

    pub async fn fetch_user_gpg_keys(&self, username: &str) -> Result<Vec<GhGpgKey>, SpoorError> {
        self.rest.get_user_gpg_keys(username).await
    }

    pub async fn fetch_user_ssh_keys(&self, username: &str) -> Result<Vec<GhSshKey>, SpoorError> {
        self.rest.get_user_ssh_keys(username).await
    }

    pub async fn fetch_repository_languages(
        &self,
        owner: &str,
        repo: &str,
    ) -> Result<HashMap<String, u64>, SpoorError> {
        self.rest.get_repo_languages(owner, repo).await
    }

    pub async fn fetch_repository_community_profile(
        &self,
        owner: &str,
        repo: &str,
    ) -> Result<GhCommunityProfile, SpoorError> {
        self.rest.get_repo_community_profile(owner, repo).await
    }

    pub async fn fetch_repository_tree(
        &self,
        owner: &str,
        repo: &str,
        branch: &str,
    ) -> Result<GhTreeResponse, SpoorError> {
        self.rest.get_repo_tree(owner, repo, branch).await
    }

    pub async fn search_commits(
        &self,
        query: &str,
        page: usize,
        per_page: usize,
    ) -> Result<GhCommitSearchResult, SpoorError> {
        self.rest.search_commits(query, page, per_page).await
    }

    pub async fn fetch_user_contributions(
        &self,
        username: &str,
    ) -> Result<Vec<graphql::GhGraphQLContributionByRepo>, SpoorError> {
        self.graphql.get_user_commit_contributions(username).await
    }
}
