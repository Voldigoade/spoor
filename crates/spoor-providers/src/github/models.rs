use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GhUser {
    pub login: String,
    pub id: u64,
    pub node_id: Option<String>,
    pub avatar_url: String,
    pub html_url: String,
    pub name: Option<String>,
    pub company: Option<String>,
    pub blog: Option<String>,
    pub location: Option<String>,
    pub email: Option<String>,
    pub bio: Option<String>,
    pub twitter_username: Option<String>,
    #[serde(default)]
    pub public_repos: u64,
    #[serde(default)]
    pub public_gists: u64,
    #[serde(default)]
    pub followers: u64,
    #[serde(default)]
    pub following: u64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GhRepo {
    pub name: String,
    pub full_name: String,
    pub description: Option<String>,
    #[serde(default)]
    pub fork: bool,
    pub html_url: String,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
    pub pushed_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub stargazers_count: u64,
    pub watchers_count: Option<u64>,
    pub language: Option<String>,
    #[serde(default)]
    pub forks_count: u64,
    #[serde(default)]
    pub open_issues_count: u64,
    pub license: Option<GhLicense>,
    pub topics: Option<Vec<String>>,
    pub default_branch: Option<String>,
    pub archived: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GhLicense {
    pub key: String,
    pub name: String,
    pub spdx_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GhSimpleUser {
    pub login: String,
    pub id: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GhCommitActor {
    pub name: String,
    pub email: String,
    pub date: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GhVerification {
    pub verified: bool,
    pub reason: String,
    pub signature: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GhCommitDetail {
    pub author: Option<GhCommitActor>,
    pub committer: Option<GhCommitActor>,
    pub message: String,
    pub verification: Option<GhVerification>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GhCommitItem {
    pub sha: String,
    pub html_url: String,
    pub commit: GhCommitDetail,
    pub author: Option<GhSimpleUser>,
    pub committer: Option<GhSimpleUser>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GhEventRepo {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GhEvent {
    pub id: String,
    #[serde(rename = "type")]
    pub event_type: String,
    pub repo: GhEventRepo,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GhEventBatch {
    pub events: Vec<GhEvent>,
    pub is_complete: bool,
    pub warning: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GhGpgEmail {
    pub email: String,
    pub verified: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GhGpgKey {
    pub id: u64,
    pub key_id: Option<String>,
    pub raw_key: Option<String>,
    pub public_key: Option<String>,
    pub emails: Option<Vec<GhGpgEmail>>,
    pub can_sign: Option<bool>,
    pub created_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GhSshKey {
    pub id: u64,
    pub key: String,
    pub title: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct GhCommunityFiles {
    pub readme: Option<serde_json::Value>,
    pub license: Option<serde_json::Value>,
    pub contributing: Option<serde_json::Value>,
    pub code_of_conduct: Option<serde_json::Value>,
    pub security: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct GhCommunityProfile {
    pub health_percentage: Option<u32>,
    pub files: Option<GhCommunityFiles>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GhTreeItem {
    pub path: String,
    #[serde(rename = "type")]
    pub item_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct GhTreeResponse {
    pub sha: Option<String>,
    #[serde(default)]
    pub tree: Vec<GhTreeItem>,
    #[serde(default)]
    pub truncated: bool,
}
