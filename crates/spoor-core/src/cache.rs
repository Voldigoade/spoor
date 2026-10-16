use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::error::SpoorError;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CacheStats {
    pub total_entries: usize,
    pub active_entries: usize,
    pub expired_entries: usize,
    pub total_bytes: usize,
    pub database_path: String,
}

#[async_trait]
pub trait CacheStore: Send + Sync {
    async fn get(
        &self,
        provider: &str,
        target: &str,
        options_hash: &str,
    ) -> Result<Option<String>, SpoorError>;

    async fn set(
        &self,
        provider: &str,
        target: &str,
        options_hash: &str,
        data: &str,
        ttl_secs: u64,
    ) -> Result<(), SpoorError>;

    async fn stats(&self) -> Result<CacheStats, SpoorError>;

    async fn prune_expired(&self) -> Result<usize, SpoorError>;

    async fn clear(&self) -> Result<usize, SpoorError>;
}

pub struct SqliteCacheStore {
    conn: Mutex<Connection>,
    path_display: String,
}

impl SqliteCacheStore {
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self, SpoorError> {
        let p = path.as_ref();
        if let Some(parent) = p.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    SpoorError::Cache(format!("Failed to create cache directory: {e}"))
                })?;
            }
        }
        let conn = Connection::open(p)
            .map_err(|e| SpoorError::Cache(format!("Failed to open cache database: {e}")))?;
        Self::init_schema(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
            path_display: p.to_string_lossy().to_string(),
        })
    }

    pub fn in_memory() -> Result<Self, SpoorError> {
        let conn = Connection::open_in_memory()
            .map_err(|e| SpoorError::Cache(format!("Failed to open in-memory cache: {e}")))?;
        Self::init_schema(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
            path_display: ":memory:".to_string(),
        })
    }

    pub fn default_path() -> PathBuf {
        if let Ok(env_path) = std::env::var("SPOOR_CACHE_PATH") {
            return PathBuf::from(env_path);
        }
        let mut base = dirs_next().unwrap_or_else(|| PathBuf::from("."));
        base.push("spoor");
        base.push("cache.sqlite");
        base
    }

    fn init_schema(conn: &Connection) -> Result<(), SpoorError> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS spoor_cache (
                cache_key TEXT PRIMARY KEY,
                provider TEXT NOT NULL,
                target TEXT NOT NULL,
                options_hash TEXT NOT NULL,
                data TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                expires_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_cache_expires ON spoor_cache(expires_at);
            CREATE INDEX IF NOT EXISTS idx_cache_lookup ON spoor_cache(provider, target, options_hash);",
        )
        .map_err(|e| SpoorError::Cache(format!("Failed to initialize cache schema: {e}")))?;
        Ok(())
    }

    fn make_key(provider: &str, target: &str, options_hash: &str) -> String {
        format!("{provider}:{target}:{options_hash}")
    }

    fn now_secs() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64
    }
}

fn dirs_next() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("APPDATA").map(PathBuf::from))
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
    }
}

#[async_trait]
impl CacheStore for SqliteCacheStore {
    async fn get(
        &self,
        provider: &str,
        target: &str,
        options_hash: &str,
    ) -> Result<Option<String>, SpoorError> {
        let key = Self::make_key(provider, target, options_hash);
        let now = Self::now_secs();

        let conn = self
            .conn
            .lock()
            .map_err(|_| SpoorError::Cache("Failed to lock cache connection".into()))?;

        let mut stmt = conn
            .prepare("SELECT data, expires_at FROM spoor_cache WHERE cache_key = ?1")
            .map_err(|e| SpoorError::Cache(e.to_string()))?;

        let mut rows = stmt
            .query(params![key])
            .map_err(|e| SpoorError::Cache(e.to_string()))?;

        if let Some(row) = rows.next().map_err(|e| SpoorError::Cache(e.to_string()))? {
            let data: String = row.get(0).map_err(|e| SpoorError::Cache(e.to_string()))?;
            let expires_at: i64 = row.get(1).map_err(|e| SpoorError::Cache(e.to_string()))?;

            if expires_at > now {
                return Ok(Some(data));
            } else {
                let _ = conn.execute("DELETE FROM spoor_cache WHERE cache_key = ?1", params![key]);
            }
        }

        Ok(None)
    }

    async fn set(
        &self,
        provider: &str,
        target: &str,
        options_hash: &str,
        data: &str,
        ttl_secs: u64,
    ) -> Result<(), SpoorError> {
        let key = Self::make_key(provider, target, options_hash);
        let now = Self::now_secs();
        let expires_at = now + ttl_secs as i64;

        let conn = self
            .conn
            .lock()
            .map_err(|_| SpoorError::Cache("Failed to lock cache connection".into()))?;

        conn.execute(
            "INSERT INTO spoor_cache (cache_key, provider, target, options_hash, data, created_at, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(cache_key) DO UPDATE SET
                data = excluded.data,
                created_at = excluded.created_at,
                expires_at = excluded.expires_at",
            params![key, provider, target, options_hash, data, now, expires_at],
        )
        .map_err(|e| SpoorError::Cache(e.to_string()))?;

        // Bounded opportunistic cleanup of expired records on write
        let _ = conn.execute(
            "DELETE FROM spoor_cache WHERE expires_at <= ?1",
            params![now],
        );

        Ok(())
    }

    async fn stats(&self) -> Result<CacheStats, SpoorError> {
        let now = Self::now_secs();
        let conn = self
            .conn
            .lock()
            .map_err(|_| SpoorError::Cache("Failed to lock cache connection".into()))?;

        let total: usize = conn
            .query_row("SELECT COUNT(*) FROM spoor_cache", [], |row| row.get(0))
            .map_err(|e| SpoorError::Cache(e.to_string()))?;

        let active: usize = conn
            .query_row(
                "SELECT COUNT(*) FROM spoor_cache WHERE expires_at > ?1",
                params![now],
                |row| row.get(0),
            )
            .map_err(|e| SpoorError::Cache(e.to_string()))?;

        let total_bytes: usize = conn
            .query_row(
                "SELECT COALESCE(SUM(LENGTH(data)), 0) FROM spoor_cache",
                [],
                |row| row.get(0),
            )
            .map_err(|e| SpoorError::Cache(e.to_string()))?;

        Ok(CacheStats {
            total_entries: total,
            active_entries: active,
            expired_entries: total.saturating_sub(active),
            total_bytes,
            database_path: self.path_display.clone(),
        })
    }

    async fn prune_expired(&self) -> Result<usize, SpoorError> {
        let now = Self::now_secs();
        let conn = self
            .conn
            .lock()
            .map_err(|_| SpoorError::Cache("Failed to lock cache connection".into()))?;

        let deleted = conn
            .execute(
                "DELETE FROM spoor_cache WHERE expires_at <= ?1",
                params![now],
            )
            .map_err(|e| SpoorError::Cache(e.to_string()))?;

        Ok(deleted)
    }

    async fn clear(&self) -> Result<usize, SpoorError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| SpoorError::Cache("Failed to lock cache connection".into()))?;

        let count: usize = conn
            .query_row("SELECT COUNT(*) FROM spoor_cache", [], |row| row.get(0))
            .unwrap_or(0);

        conn.execute("DELETE FROM spoor_cache", [])
            .map_err(|e| SpoorError::Cache(e.to_string()))?;

        Ok(count)
    }
}

pub struct NoopCacheStore;

#[async_trait]
impl CacheStore for NoopCacheStore {
    async fn get(&self, _p: &str, _t: &str, _o: &str) -> Result<Option<String>, SpoorError> {
        Ok(None)
    }
    async fn set(
        &self,
        _p: &str,
        _t: &str,
        _o: &str,
        _d: &str,
        _ttl: u64,
    ) -> Result<(), SpoorError> {
        Ok(())
    }
    async fn stats(&self) -> Result<CacheStats, SpoorError> {
        Ok(CacheStats::default())
    }
    async fn prune_expired(&self) -> Result<usize, SpoorError> {
        Ok(0)
    }
    async fn clear(&self) -> Result<usize, SpoorError> {
        Ok(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn basic_cache_lifecycle() {
        let cache = SqliteCacheStore::in_memory().unwrap();
        assert_eq!(cache.get("github", "torvalds", "fast").await.unwrap(), None);

        cache
            .set("github", "torvalds", "fast", r#"{"name":"Linus"}"#, 3600)
            .await
            .unwrap();

        let val = cache.get("github", "torvalds", "fast").await.unwrap();
        assert_eq!(val, Some(r#"{"name":"Linus"}"#.to_string()));

        let stats = cache.stats().await.unwrap();
        assert_eq!(stats.total_entries, 1);
        assert_eq!(stats.active_entries, 1);

        let cleared = cache.clear().await.unwrap();
        assert_eq!(cleared, 1);
        assert_eq!(cache.get("github", "torvalds", "fast").await.unwrap(), None);
    }

    #[tokio::test]
    async fn prune_expired_removes_stale_records() {
        let cache = SqliteCacheStore::in_memory().unwrap();

        // Insert an active record and an expired record
        cache
            .set("github", "user1", "fast", "active_data", 3600)
            .await
            .unwrap();

        // Insert directly with past timestamp to simulate stale record without waiting
        {
            let conn = cache.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO spoor_cache (cache_key, provider, target, options_hash, data, created_at, expires_at)
                 VALUES ('github:user2:fast', 'github', 'user2', 'fast', 'expired_data', 1000, 1001)",
                [],
            )
            .unwrap();
        }

        let stats = cache.stats().await.unwrap();
        assert_eq!(stats.total_entries, 2);
        assert_eq!(stats.active_entries, 1);
        assert_eq!(stats.expired_entries, 1);

        let pruned = cache.prune_expired().await.unwrap();
        assert_eq!(pruned, 1);

        let stats_after = cache.stats().await.unwrap();
        assert_eq!(stats_after.total_entries, 1);
        assert_eq!(stats_after.active_entries, 1);
        assert_eq!(stats_after.expired_entries, 0);
    }
}
