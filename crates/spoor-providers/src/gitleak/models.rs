use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitLeakEmail {
    pub email: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub committer: Option<String>,
    #[serde(rename = "commitUrl", skip_serializing_if = "Option::is_none")]
    pub commit_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct GitLeakHeatmapHour {
    pub hour: u32,
    pub count: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct GitLeakPeakActivity {
    pub hour: u32,
    pub count: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct GitLeakActivityHeatmap {
    #[serde(rename = "heatmapData", default)]
    pub heatmap_data: Vec<GitLeakHeatmapHour>,
    #[serde(rename = "totalCommits", default)]
    pub total_commits: u64,
    #[serde(rename = "peakActivity", skip_serializing_if = "Option::is_none")]
    pub peak_activity: Option<GitLeakPeakActivity>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct GitLeakMetadata {
    #[serde(rename = "totalCommitsProcessed", default)]
    pub total_commits_processed: Option<u64>,
    #[serde(rename = "emailsFound", default)]
    pub emails_found: Option<u64>,
    #[serde(rename = "executionTimeMs", default)]
    pub execution_time_ms: Option<u64>,
    #[serde(rename = "discoveredUsernames", default)]
    pub discovered_usernames: Vec<String>,
    #[serde(rename = "activityHeatmap", skip_serializing_if = "Option::is_none")]
    pub activity_heatmap: Option<GitLeakActivityHeatmap>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct GitLeakResponse {
    #[serde(default)]
    pub emails: Vec<GitLeakEmail>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<GitLeakMetadata>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}
