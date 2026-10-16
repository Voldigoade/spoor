use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Utc};
use spoor_core::SpoorError;
use tracing::{debug, warn};
use uuid::Uuid;

pub const DEFAULT_GIT_TIMEOUT_SECS: u64 = 20;
pub const MAX_REPO_BYTES_BUDGET: u64 = 30 * 1024 * 1024; // 30 MB

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitCommitRecord {
    pub sha: String,
    pub author_name: String,
    pub author_email: String,
    pub author_date: Option<DateTime<Utc>>,
    pub committer_name: String,
    pub committer_email: String,
    pub committer_date: Option<DateTime<Utc>>,
    pub is_verified: bool,
    pub message_subject: String,
    pub repository: String,
}

#[allow(clippy::permissions_set_readonly_false)]
fn make_dir_writable_recursive(path: &Path) {
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let p = entry.path();
            if let Ok(meta) = entry.metadata() {
                if meta.permissions().readonly() {
                    let mut perms = meta.permissions();
                    perms.set_readonly(false);
                    let _ = std::fs::set_permissions(&p, perms);
                }
                if meta.is_dir() {
                    make_dir_writable_recursive(&p);
                }
            }
        }
    }
    if let Ok(meta) = std::fs::metadata(path) {
        if meta.permissions().readonly() {
            let mut perms = meta.permissions();
            perms.set_readonly(false);
            let _ = std::fs::set_permissions(path, perms);
        }
    }
}

pub fn remove_dir_all_robust(path: &Path) -> Result<(), std::io::Error> {
    if !path.exists() {
        return Ok(());
    }

    make_dir_writable_recursive(path);

    match std::fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(_) if !path.exists() => Ok(()),
        Err(e) => {
            let mut last_err = e;
            for delay_ms in [20, 50, 100, 200, 300, 500] {
                std::thread::sleep(Duration::from_millis(delay_ms));
                make_dir_writable_recursive(path);
                match std::fs::remove_dir_all(path) {
                    Ok(()) => return Ok(()),
                    Err(_) if !path.exists() => return Ok(()),
                    Err(err) => last_err = err,
                }
            }
            if !path.exists() {
                Ok(())
            } else {
                Err(last_err)
            }
        }
    }
}

pub struct TempDirGuard {
    path: PathBuf,
}

impl TempDirGuard {
    pub fn new(prefix: &str) -> Result<Self, SpoorError> {
        let temp_dir = std::env::temp_dir().join(format!("{prefix}-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&temp_dir).map_err(|e| {
            SpoorError::Internal(format!(
                "Failed to create temporary directory for git inspection: {e}"
            ))
        })?;
        Ok(Self { path: temp_dir })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn cleanup(&mut self) -> Result<(), std::io::Error> {
        if self.path.exists() {
            remove_dir_all_robust(&self.path)?;
        }
        Ok(())
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        if let Err(e) = self.cleanup() {
            warn!(path = %self.path.display(), error = %e, "Failed to remove temporary directory guard");
        }
    }
}

pub struct GitInspector {
    timeout_duration: Duration,
    max_bytes_budget: u64,
}

impl Default for GitInspector {
    fn default() -> Self {
        Self::new(DEFAULT_GIT_TIMEOUT_SECS, MAX_REPO_BYTES_BUDGET)
    }
}

impl GitInspector {
    pub fn new(timeout_secs: u64, max_bytes_budget: u64) -> Self {
        Self {
            timeout_duration: Duration::from_secs(timeout_secs),
            max_bytes_budget,
        }
    }

    pub fn validate_github_url(url: &str) -> Result<(String, String), SpoorError> {
        let trimmed = url.trim();

        if !trimmed.starts_with("https://github.com/") {
            return Err(SpoorError::SsrfProtection(format!(
                "Invalid git clone target '{trimmed}'. Only https://github.com repositories are allowed."
            )));
        }

        let path = trimmed
            .trim_start_matches("https://github.com/")
            .trim_end_matches(".git");
        let parts: Vec<&str> = path.split('/').collect();
        if parts.len() != 2 {
            return Err(SpoorError::InvalidTarget {
                input: trimmed.to_string(),
                reason: "URL must contain an owner and a repository name".to_string(),
            });
        }

        let owner = parts[0];
        let repo = parts[1];

        let is_valid_char = |c: char| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_';
        if owner.is_empty()
            || repo.is_empty()
            || !owner.chars().all(is_valid_char)
            || !repo.chars().all(is_valid_char)
        {
            return Err(SpoorError::InvalidTarget {
                input: trimmed.to_string(),
                reason: "Owner or repository name contains invalid characters".to_string(),
            });
        }

        Ok((owner.to_string(), repo.to_string()))
    }

    pub async fn is_git_available() -> bool {
        tokio::process::Command::new("git")
            .arg("--version")
            .output()
            .await
            .map(|out| out.status.success())
            .unwrap_or(false)
    }

    fn calculate_dir_size(path: &Path) -> u64 {
        let mut total = 0;
        if let Ok(entries) = std::fs::read_dir(path) {
            for entry in entries.flatten() {
                if let Ok(metadata) = entry.metadata() {
                    if metadata.is_dir() {
                        total += Self::calculate_dir_size(&entry.path());
                    } else {
                        total += metadata.len();
                    }
                }
            }
        }
        total
    }

    pub async fn inspect_repo_history(
        &self,
        owner: &str,
        repo: &str,
        max_commits: usize,
    ) -> Result<Vec<GitCommitRecord>, SpoorError> {
        let clone_url = format!("https://github.com/{owner}/{repo}.git");
        Self::validate_github_url(&clone_url)?;

        if !Self::is_git_available().await {
            return Err(SpoorError::ProviderError {
                provider: "git".to_string(),
                message: "git command line tool is not available in system PATH".to_string(),
            });
        }

        let mut temp_guard = TempDirGuard::new("spoor-git-repo")?;
        let repo_dir = temp_guard.path().to_path_buf();

        debug!(owner = %owner, repo = %repo, "Initiating safe bare clone for history inspection");

        let mut clone_session = crate::process_tree::ProcessTreeSession::new().map_err(|e| {
            SpoorError::ProviderError {
                provider: "git".to_string(),
                message: format!("Failed to create process tree session: {e}"),
            }
        })?;

        let clone_cmd = crate::process_tree::ProcessCommand::new("git")
            .arg("-c")
            .arg("core.hooksPath=/dev/null")
            .arg("-c")
            .arg("credential.helper=")
            .arg("clone")
            .arg("--bare")
            .arg("--filter=blob:none")
            .arg(&clone_url)
            .arg(repo_dir.to_string_lossy().to_string())
            .capture_stdout(false)
            .capture_stderr(true);

        let mut clone_child = clone_session.spawn(clone_cmd).map_err(|e| {
            if let Err(cleanup_err) = temp_guard.cleanup() {
                warn!(
                    path = %temp_guard.path().display(),
                    error = %cleanup_err,
                    "Failed to clean up temporary directory after git clone spawn failure"
                );
            }
            SpoorError::ProviderError {
                provider: "git".to_string(),
                message: format!("Failed to spawn git clone: {e}"),
            }
        })?;

        let max_budget = self.max_bytes_budget;
        let repo_dir_clone = repo_dir.clone();
        let monitor_future = async {
            let mut interval = tokio::time::interval(Duration::from_millis(100));
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        let size = Self::calculate_dir_size(&repo_dir_clone);
                        if size > max_budget {
                            return Err(SpoorError::ProviderError {
                                provider: "git".to_string(),
                                message: format!(
                                    "Repository clone exceeded safety byte budget during transfer (used {} MB, limit {} MB)",
                                    size / (1024 * 1024),
                                    max_budget / (1024 * 1024)
                                ),
                            });
                        }
                    }
                    status = clone_child.wait() => {
                        let exit_status = status.map_err(|e| SpoorError::ProviderError {
                            provider: "git".to_string(),
                            message: format!("Failed to wait for git clone: {e}"),
                        })?;
                        let (_stdout_bytes, stderr_bytes) = clone_child.take_output().await;
                        if !exit_status.success() {
                            let stderr_str = String::from_utf8_lossy(&stderr_bytes);
                            warn!(owner = %owner, repo = %repo, stderr = %stderr_str, "Git clone exited with error");
                            return Err(SpoorError::ProviderError {
                                provider: "git".to_string(),
                                message: format!("git clone failed: {}", stderr_str.trim()),
                            });
                        }
                        return Ok(());
                    }
                }
            }
        };

        let clone_result = tokio::time::timeout(self.timeout_duration, monitor_future).await;

        match clone_result {
            Ok(Ok(())) => {
                clone_session.terminate();
            }
            Ok(Err(e)) => {
                clone_session.terminate();
                let _ = clone_child.kill().await;
                let _ = clone_child.wait().await;
                if let Err(cleanup_err) = temp_guard.cleanup() {
                    warn!(
                        path = %temp_guard.path().display(),
                        error = %cleanup_err,
                        "Failed to clean up temporary directory after git clone failure"
                    );
                }
                return Err(e);
            }
            Err(_) => {
                clone_session.terminate();
                let _ = clone_child.kill().await;
                let _ = clone_child.wait().await;
                if let Err(cleanup_err) = temp_guard.cleanup() {
                    warn!(
                        path = %temp_guard.path().display(),
                        error = %cleanup_err,
                        "Failed to clean up temporary directory after git clone timeout"
                    );
                }
                return Err(SpoorError::Timeout {
                    provider: "git".to_string(),
                    timeout_secs: self.timeout_duration.as_secs(),
                });
            }
        }

        let max_arg = format!("--max-count={max_commits}");
        let format_arg =
            "--format=format:%H%x00%an%x00%ae%x00%aI%x00%cn%x00%ce%x00%cI%x00%G?%x00%s";

        let mut log_session = crate::process_tree::ProcessTreeSession::new().map_err(|e| {
            SpoorError::ProviderError {
                provider: "git".to_string(),
                message: format!("Failed to create process tree session for git log: {e}"),
            }
        })?;

        let log_cmd = crate::process_tree::ProcessCommand::new("git")
            .arg("-C")
            .arg(repo_dir.to_string_lossy().to_string())
            .arg("--no-pager")
            .arg("log")
            .arg("--all")
            .arg(&max_arg)
            .arg(format_arg)
            .capture_stdout(true)
            .capture_stderr(true);

        let mut log_child = log_session.spawn(log_cmd).map_err(|e| {
            if let Err(cleanup_err) = temp_guard.cleanup() {
                warn!(
                    path = %temp_guard.path().display(),
                    error = %cleanup_err,
                    "Failed to clean up temporary directory after git log spawn failure"
                );
            }
            SpoorError::ProviderError {
                provider: "git".to_string(),
                message: format!("Failed to spawn git log: {e}"),
            }
        })?;

        let log_wait_future = async {
            let status = log_child
                .wait()
                .await
                .map_err(|e| SpoorError::ProviderError {
                    provider: "git".to_string(),
                    message: format!("Failed to wait for git log: {e}"),
                })?;
            let (stdout_bytes, stderr_bytes) = log_child.take_output().await;
            Ok((status, stdout_bytes, stderr_bytes))
        };

        let (log_status, stdout_bytes, stderr_bytes) =
            match tokio::time::timeout(self.timeout_duration, log_wait_future).await {
                Ok(res) => res?,
                Err(_) => {
                    log_session.terminate();
                    let _ = log_child.kill().await;
                    let _ = log_child.wait().await;
                    if let Err(cleanup_err) = temp_guard.cleanup() {
                        warn!(
                            path = %temp_guard.path().display(),
                            error = %cleanup_err,
                            "Failed to clean up temporary directory after git log timeout"
                        );
                    }
                    return Err(SpoorError::Timeout {
                        provider: "git".to_string(),
                        timeout_secs: self.timeout_duration.as_secs(),
                    });
                }
            };

        log_session.terminate();

        if !log_status.success() {
            let stderr_str = String::from_utf8_lossy(&stderr_bytes);
            if let Err(cleanup_err) = temp_guard.cleanup() {
                warn!(
                    path = %temp_guard.path().display(),
                    error = %cleanup_err,
                    "Failed to clean up temporary directory after git log failure"
                );
            }
            return Err(SpoorError::ProviderError {
                provider: "git".to_string(),
                message: format!("git log failed: {}", stderr_str.trim()),
            });
        }

        let stdout = String::from_utf8_lossy(&stdout_bytes);
        let mut records = Vec::new();
        let repo_identifier = format!("{owner}/{repo}");

        for line in stdout.lines() {
            let parts: Vec<&str> = line.split('\0').collect();
            if parts.len() < 8 {
                continue;
            }

            let sha = parts[0].trim().to_string();
            let author_name = parts[1].trim().to_string();
            let author_email = parts[2].trim().to_string();
            let author_date = DateTime::parse_from_rfc3339(parts[3].trim())
                .ok()
                .map(|d| d.with_timezone(&Utc));

            let committer_name = parts[4].trim().to_string();
            let committer_email = parts[5].trim().to_string();
            let committer_date = DateTime::parse_from_rfc3339(parts[6].trim())
                .ok()
                .map(|d| d.with_timezone(&Utc));

            let verification_code = parts.get(7).copied().unwrap_or("N");
            let is_verified = verification_code == "G";

            let message_subject = parts.get(8).copied().unwrap_or("").trim().to_string();

            if !sha.is_empty() && (!author_email.is_empty() || !committer_email.is_empty()) {
                records.push(GitCommitRecord {
                    sha,
                    author_name,
                    author_email,
                    author_date,
                    committer_name,
                    committer_email,
                    committer_date,
                    is_verified,
                    message_subject,
                    repository: repo_identifier.clone(),
                });
            }
        }

        temp_guard
            .cleanup()
            .map_err(|e| SpoorError::ProviderError {
                provider: "git".to_string(),
                message: format!(
                    "Failed to clean up temporary directory after successful git inspection: {e}"
                ),
            })?;

        debug!(owner = %owner, repo = %repo, count = records.len(), "Parsed commits from bare git clone");
        Ok(records)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_github_https_urls() {
        assert!(GitInspector::validate_github_url("https://github.com/torvalds/linux").is_ok());
        assert!(
            GitInspector::validate_github_url("https://github.com/Voldigoade/spoor.git").is_ok()
        );
    }

    #[test]
    fn rejects_non_github_urls_and_local_paths() {
        assert!(GitInspector::validate_github_url("file:///etc/passwd").is_err());
        assert!(GitInspector::validate_github_url("https://gitlab.com/user/repo").is_err());
        assert!(GitInspector::validate_github_url("http://github.com/user/repo").is_err());
        assert!(GitInspector::validate_github_url("https://evil.com/user/repo").is_err());
        assert!(GitInspector::validate_github_url("https://github.com/user/repo/extra").is_err());
    }

    #[test]
    fn temp_dir_guard_cleans_up_on_drop() {
        let path = {
            let guard = TempDirGuard::new("test-guard").unwrap();
            let p = guard.path().to_path_buf();
            assert!(p.exists());
            p
        };
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn test_git_inspector_configuration_and_budget() {
        let inspector = GitInspector::new(5, 1024 * 1024);
        assert_eq!(inspector.max_bytes_budget, 1024 * 1024);
        assert_eq!(inspector.timeout_duration, Duration::from_secs(5));

        let res = inspector
            .inspect_repo_history("invalid..user", "repo", 10)
            .await;
        assert!(res.is_err());
    }
}
