use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::error::SpoorError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AnalysisMode {
    #[default]
    Fast,
    Deep,
}

impl AnalysisMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            AnalysisMode::Fast => "fast",
            AnalysisMode::Deep => "deep",
        }
    }
}

impl std::fmt::Display for AnalysisMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AnalysisOptions {
    pub mode: AnalysisMode,
    #[serde(default)]
    pub enrich: bool,
}

impl Default for AnalysisOptions {
    fn default() -> Self {
        Self {
            mode: AnalysisMode::Fast,
            enrich: false,
        }
    }
}

impl AnalysisOptions {
    pub fn fast() -> Self {
        Self {
            mode: AnalysisMode::Fast,
            enrich: false,
        }
    }

    pub fn fast_enriched() -> Self {
        Self {
            mode: AnalysisMode::Fast,
            enrich: true,
        }
    }

    pub fn deep() -> Self {
        Self {
            mode: AnalysisMode::Deep,
            enrich: false,
        }
    }

    pub fn deep_enriched() -> Self {
        Self {
            mode: AnalysisMode::Deep,
            enrich: true,
        }
    }

    pub fn new(mode: AnalysisMode, enrich: Option<bool>) -> Self {
        let enrich = enrich.unwrap_or(false);
        Self { mode, enrich }
    }
}

impl From<AnalysisMode> for AnalysisOptions {
    fn from(mode: AnalysisMode) -> Self {
        Self {
            mode,
            enrich: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Target {
    User { username: String },
    Repo { owner: String, repo: String },
}

const RESERVED_USERNAMES: &[&str] = &[
    "about",
    "apps",
    "contact",
    "customer-stories",
    "explore",
    "features",
    "issues",
    "join",
    "login",
    "marketplace",
    "notifications",
    "orgs",
    "pricing",
    "pulls",
    "search",
    "security",
    "settings",
    "site",
    "stars",
    "topics",
    "trending",
];

impl Target {
    pub fn parse(raw: &str) -> Result<Self, SpoorError> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(SpoorError::InvalidTarget {
                input: raw.to_string(),
                reason: "target cannot be empty".to_string(),
            });
        }

        if trimmed.chars().any(|c| c.is_control() || c == '\0') {
            return Err(SpoorError::InvalidTarget {
                input: raw.to_string(),
                reason: "target contains invalid control characters".to_string(),
            });
        }

        if trimmed.starts_with("http://")
            || trimmed.starts_with("https://")
            || trimmed.starts_with("github.com/")
            || trimmed.starts_with("www.github.com/")
        {
            return Self::parse_url(trimmed);
        }

        if trimmed.contains("://") || trimmed.contains(".com/") || trimmed.contains(".org/") {
            return Err(SpoorError::SsrfProtection(format!(
                "Non-GitHub URL rejected: '{raw}'"
            )));
        }

        if let Some((owner, repo_part)) = trimmed.split_once('/') {
            if repo_part.contains('/') {
                return Err(SpoorError::InvalidTarget {
                    input: raw.to_string(),
                    reason: "repository shorthand cannot have more than two path segments"
                        .to_string(),
                });
            }
            let repo = repo_part.strip_suffix(".git").unwrap_or(repo_part);
            Self::validate_username(owner)?;
            Self::validate_repo_name(repo)?;
            Ok(Target::Repo {
                owner: owner.to_string(),
                repo: repo.to_string(),
            })
        } else {
            Self::validate_username(trimmed)?;
            Ok(Target::User {
                username: trimmed.to_string(),
            })
        }
    }

    fn parse_url(url_str: &str) -> Result<Self, SpoorError> {
        let normalized_url_str = if !url_str.contains("://") {
            format!("https://{url_str}")
        } else {
            url_str.to_string()
        };

        let parsed = Url::parse(&normalized_url_str).map_err(|e| SpoorError::InvalidTarget {
            input: url_str.to_string(),
            reason: format!("malformed URL: {e}"),
        })?;

        // Strict SSRF check: host must strictly be github.com or www.github.com
        let host = parsed.host_str().unwrap_or("").to_lowercase();
        if host != "github.com" && host != "www.github.com" {
            return Err(SpoorError::SsrfProtection(format!(
                "Only github.com targets are permitted, received host: '{host}'"
            )));
        }

        // Reject userinfo, non-standard ports, query params, fragments that could confuse target extraction
        if !parsed.username().is_empty() || parsed.password().is_some() {
            return Err(SpoorError::SsrfProtection(
                "Credentials in target URL are prohibited".to_string(),
            ));
        }

        let segments: Vec<&str> = parsed
            .path_segments()
            .map(|s| s.filter(|seg| !seg.is_empty()).collect())
            .unwrap_or_default();

        match segments.as_slice() {
            [user] => {
                Self::validate_username(user)?;
                Ok(Target::User {
                    username: (*user).to_string(),
                })
            }
            [owner, repo_part] => {
                let repo = repo_part.strip_suffix(".git").unwrap_or(repo_part);
                Self::validate_username(owner)?;
                Self::validate_repo_name(repo)?;
                Ok(Target::Repo {
                    owner: (*owner).to_string(),
                    repo: repo.to_string(),
                })
            }
            [] => Err(SpoorError::InvalidTarget {
                input: url_str.to_string(),
                reason: "URL does not point to a GitHub user or repository".to_string(),
            }),
            _ => Err(SpoorError::InvalidTarget {
                input: url_str.to_string(),
                reason: "deep URLs (commits, issues, pulls) are not supported as root targets; provide a user or repo"
                    .to_string(),
            }),
        }
    }

    fn validate_username(name: &str) -> Result<(), SpoorError> {
        if name.is_empty() {
            return Err(SpoorError::InvalidTarget {
                input: name.to_string(),
                reason: "username cannot be empty".to_string(),
            });
        }
        if name.len() > 39 {
            return Err(SpoorError::InvalidTarget {
                input: name.to_string(),
                reason: "username exceeds maximum length of 39 characters".to_string(),
            });
        }
        if name.starts_with('-') || name.ends_with('-') {
            return Err(SpoorError::InvalidTarget {
                input: name.to_string(),
                reason: "username cannot begin or end with a hyphen".to_string(),
            });
        }
        for c in name.chars() {
            if !c.is_ascii_alphanumeric() && c != '-' {
                return Err(SpoorError::InvalidTarget {
                    input: name.to_string(),
                    reason: format!("username contains invalid character '{c}'"),
                });
            }
        }
        let lower = name.to_lowercase();
        if RESERVED_USERNAMES.contains(&lower.as_str()) {
            return Err(SpoorError::InvalidTarget {
                input: name.to_string(),
                reason: format!("'{name}' is a reserved GitHub system path, not a user profile"),
            });
        }
        Ok(())
    }

    fn validate_repo_name(name: &str) -> Result<(), SpoorError> {
        if name.is_empty() {
            return Err(SpoorError::InvalidTarget {
                input: name.to_string(),
                reason: "repository name cannot be empty".to_string(),
            });
        }
        if name == "." || name == ".." {
            return Err(SpoorError::InvalidTarget {
                input: name.to_string(),
                reason: "repository name cannot be '.' or '..'".to_string(),
            });
        }
        if name.len() > 100 {
            return Err(SpoorError::InvalidTarget {
                input: name.to_string(),
                reason: "repository name exceeds maximum length of 100 characters".to_string(),
            });
        }
        for c in name.chars() {
            if !c.is_ascii_alphanumeric() && c != '-' && c != '_' && c != '.' {
                return Err(SpoorError::InvalidTarget {
                    input: name.to_string(),
                    reason: format!("repository name contains invalid character '{c}'"),
                });
            }
        }
        Ok(())
    }

    pub fn display_name(&self) -> String {
        match self {
            Target::User { username } => username.clone(),
            Target::Repo { owner, repo } => format!("{owner}/{repo}"),
        }
    }

    pub fn canonical_url(&self) -> String {
        match self {
            Target::User { username } => format!("https://github.com/{username}"),
            Target::Repo { owner, repo } => format!("https://github.com/{owner}/{repo}"),
        }
    }

    pub fn is_user(&self) -> bool {
        matches!(self, Target::User { .. })
    }

    pub fn is_repo(&self) -> bool {
        matches!(self, Target::Repo { .. })
    }

    pub fn username(&self) -> &str {
        match self {
            Target::User { username } => username,
            Target::Repo { owner, .. } => owner,
        }
    }

    pub fn safe_filename_stem(&self, _mode: AnalysisMode, timestamp: &DateTime<Utc>) -> String {
        let ts = timestamp.format("%Y%m%dT%H%M%SZ");
        match self {
            Target::User { username } => {
                let sanitized: String = username
                    .chars()
                    .map(|c| {
                        if c.is_ascii_alphanumeric() || c == '-' {
                            c
                        } else {
                            '_'
                        }
                    })
                    .collect();
                format!("spoor-{sanitized}-{ts}")
            }
            Target::Repo { owner, repo } => {
                let s_owner: String = owner
                    .chars()
                    .map(|c| {
                        if c.is_ascii_alphanumeric() || c == '-' {
                            c
                        } else {
                            '_'
                        }
                    })
                    .collect();
                let s_repo: String = repo
                    .chars()
                    .map(|c| {
                        if c.is_ascii_alphanumeric() || c == '-' {
                            c
                        } else {
                            '_'
                        }
                    })
                    .collect();
                format!("spoor-{s_owner}_{s_repo}-{ts}")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_username() {
        let target = Target::parse("torvalds").unwrap();
        assert_eq!(
            target,
            Target::User {
                username: "torvalds".into()
            }
        );
        assert_eq!(target.canonical_url(), "https://github.com/torvalds");
    }

    #[test]
    fn parses_profile_urls() {
        let urls = [
            "https://github.com/torvalds",
            "http://github.com/torvalds/",
            "github.com/torvalds",
            "www.github.com/torvalds/",
        ];
        for u in urls {
            let target = Target::parse(u).unwrap();
            assert_eq!(
                target,
                Target::User {
                    username: "torvalds".into()
                }
            );
        }
    }

    #[test]
    fn parses_repo_shorthand() {
        let target = Target::parse("torvalds/linux").unwrap();
        assert_eq!(
            target,
            Target::Repo {
                owner: "torvalds".into(),
                repo: "linux".into()
            }
        );
        assert_eq!(target.canonical_url(), "https://github.com/torvalds/linux");
    }

    #[test]
    fn parses_repo_urls() {
        let urls = [
            "https://github.com/torvalds/linux",
            "https://github.com/torvalds/linux.git",
            "http://github.com/torvalds/linux/",
            "github.com/torvalds/linux.git",
        ];
        for u in urls {
            let target = Target::parse(u).unwrap();
            assert_eq!(
                target,
                Target::Repo {
                    owner: "torvalds".into(),
                    repo: "linux".into()
                }
            );
        }
    }

    #[test]
    fn rejects_reserved_usernames() {
        assert!(Target::parse("settings").is_err());
        assert!(Target::parse("https://github.com/search").is_err());
    }

    #[test]
    fn rejects_external_domains_ssrf() {
        assert!(Target::parse("https://gitlab.com/torvalds").is_err());
        assert!(Target::parse("http://127.0.0.1/evil").is_err());
        assert!(Target::parse("https://attacker.com/torvalds/linux").is_err());
        assert!(Target::parse("https://github.com.attacker.com/torvalds").is_err());
    }

    #[test]
    fn rejects_invalid_usernames() {
        assert!(Target::parse("-invalid").is_err());
        assert!(Target::parse("invalid-").is_err());
        assert!(Target::parse("user with spaces").is_err());
        assert!(Target::parse("user$name").is_err());
        assert!(Target::parse("").is_err());
    }

    #[test]
    fn safe_filename_generation() {
        let target = Target::parse("torvalds").unwrap();
        let dt = DateTime::parse_from_rfc3339("2026-10-14T17:42:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(
            target.safe_filename_stem(AnalysisMode::Fast, &dt),
            "spoor-torvalds-20261014T174200Z"
        );
    }
}
