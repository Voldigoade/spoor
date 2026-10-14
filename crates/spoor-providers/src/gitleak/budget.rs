use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

#[derive(Debug, Clone)]
pub struct GitLeakBudget {
    state: Arc<Mutex<BudgetState>>,
}

#[derive(Debug)]
struct BudgetState {
    max_requests: usize,
    window_duration: Duration,
    timestamps: Vec<Instant>,
}

impl GitLeakBudget {
    pub fn new(max_requests: usize, window_secs: u64) -> Self {
        Self {
            state: Arc::new(Mutex::new(BudgetState {
                max_requests,
                window_duration: Duration::from_secs(window_secs),
                timestamps: Vec::new(),
            })),
        }
    }

    /// Read configuration from environment variables with safe production defaults:
    /// - `SPOOR_GITLEAK_BUDGET`: 40 requests
    /// - `SPOOR_GITLEAK_BUDGET_WINDOW_SECS`: 3600 seconds (1 hour)
    pub fn from_env() -> Self {
        let max_requests = std::env::var("SPOOR_GITLEAK_BUDGET")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(40);
        let window_secs = std::env::var("SPOOR_GITLEAK_BUDGET_WINDOW_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(3600);
        Self::new(max_requests, window_secs)
    }

    /// Attempt to acquire 1 request slot.
    ///
    /// If available, records the timestamp and returns `Ok(())`.
    /// If exhausted, returns `Err(retry_after_secs)`.
    pub async fn try_acquire(&self) -> Result<(), u64> {
        let mut state = self.state.lock().await;
        let now = Instant::now();
        let window = state.window_duration;

        state.timestamps.retain(|&t| now.duration_since(t) < window);

        if state.timestamps.len() >= state.max_requests {
            let earliest = state.timestamps.first().copied().unwrap_or(now);
            let elapsed = now.duration_since(earliest);
            let retry_after = if elapsed < window {
                (window - elapsed).as_secs().max(1)
            } else {
                1
            };
            Err(retry_after)
        } else {
            state.timestamps.push(now);
            Ok(())
        }
    }

    /// Returns the number of remaining requests available in the current window.
    pub async fn remaining(&self) -> usize {
        let mut state = self.state.lock().await;
        let now = Instant::now();
        let window = state.window_duration;
        state.timestamps.retain(|&t| now.duration_since(t) < window);
        state.max_requests.saturating_sub(state.timestamps.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn budget_permits_up_to_max() {
        let budget = GitLeakBudget::new(2, 60);
        assert_eq!(budget.remaining().await, 2);

        assert!(budget.try_acquire().await.is_ok());
        assert_eq!(budget.remaining().await, 1);

        assert!(budget.try_acquire().await.is_ok());
        assert_eq!(budget.remaining().await, 0);

        let err = budget.try_acquire().await;
        assert!(err.is_err());
        assert!(err.unwrap_err() <= 60);
    }
}
