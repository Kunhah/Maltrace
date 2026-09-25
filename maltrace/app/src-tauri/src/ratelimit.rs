//! Per-sidecar rate-limiting / concurrency policy.
//!
//! `run_all_transforms` and the scheduled-machine loop both want to fire
//! many transforms in rapid succession. Without throttling, keyed APIs
//! (Shodan, HIBP, VT, …) reach for the rate-limit hammer and the run
//! log fills with 429s. This module wraps every sidecar call in a
//! tokio `Semaphore` (concurrent-call cap) and a per-sidecar `Mutex<Instant>`
//! (minimum interval between consecutive calls).
//!
//! Policy is persisted in `rate_limits.json` next to `secrets.json` /
//! `proxies.json`. Limits are sidecar-scoped so a slow keyed sidecar
//! doesn't starve the rest.

use anyhow::{Context, Result};
use parking_lot::Mutex as PlMutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Limits {
    /// Maximum number of in-flight calls to one sidecar at once.
    /// Defaults to 4 — generous enough for unkeyed adapters
    /// (sfp_whois, dns) and tight enough to keep keyed adapters from
    /// burning quota on a `Run all`.
    pub max_concurrent: usize,
    /// Minimum wall-clock interval between two consecutive calls,
    /// regardless of concurrency. 200ms is comfortable for almost
    /// everything; bump higher for hostile rate-limiters.
    pub min_interval_ms: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self { max_concurrent: 4, min_interval_ms: 200 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RateLimitConfig {
    #[serde(default)]
    pub default: Option<Limits>,
    /// sidecar_id → override.
    #[serde(default)]
    pub per_sidecar: HashMap<String, Limits>,
}

/// Live state for one sidecar: a semaphore for concurrency caps and a
/// timestamp for the last-call rate limit. Both are wrapped in `Arc` so
/// the registry can hand them out cheaply across the async run path.
#[derive(Debug)]
struct SidecarState {
    sem: Arc<Semaphore>,
    last: Arc<tokio::sync::Mutex<Instant>>,
    limits: Limits,
}

#[derive(Debug)]
pub struct RateLimiter {
    path: PathBuf,
    cfg: PlMutex<RateLimitConfig>,
    /// One state per sidecar id, lazily created on first use. Wrapped
    /// in a `parking_lot::Mutex` because we never hold it across an
    /// `.await`; the inner `Semaphore` / `tokio::Mutex` is what we
    /// actually wait on.
    states: PlMutex<HashMap<String, Arc<SidecarState>>>,
}

impl RateLimiter {
    pub fn open(data_dir: &std::path::Path) -> Self {
        let path = data_dir.join("rate_limits.json");
        let cfg = match Self::read_from(&path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!(
                    "maltrace rate-limits: starting with defaults ({}); file at {} will be created on first write.",
                    e,
                    path.display(),
                );
                RateLimitConfig::default()
            }
        };
        Self {
            path,
            cfg: PlMutex::new(cfg),
            states: PlMutex::new(HashMap::new()),
        }
    }

    fn read_from(path: &std::path::Path) -> Result<RateLimitConfig> {
        if !path.exists() {
            return Ok(RateLimitConfig::default());
        }
        let body = fs::read_to_string(path)
            .with_context(|| format!("reading {}", path.display()))?;
        let parsed: RateLimitConfig = serde_json::from_str(&body)
            .with_context(|| format!("parsing {}", path.display()))?;
        Ok(parsed)
    }

    pub fn snapshot(&self) -> RateLimitConfig {
        self.cfg.lock().clone()
    }

    pub fn set(&self, cfg: RateLimitConfig) -> Result<()> {
        let mut current = self.cfg.lock();
        *current = cfg;
        let to_persist = current.clone();
        // Invalidate cached states so the new limits take effect on the
        // next call. Existing in-flight calls keep their old permits —
        // that's fine; they're already gated.
        self.states.lock().clear();
        self.persist(&to_persist)
    }

    fn limits_for(&self, sidecar_id: &str) -> Limits {
        let cfg = self.cfg.lock();
        cfg.per_sidecar
            .get(sidecar_id)
            .copied()
            .or(cfg.default)
            .unwrap_or_default()
    }

    fn state_for(&self, sidecar_id: &str) -> Arc<SidecarState> {
        let mut states = self.states.lock();
        if let Some(s) = states.get(sidecar_id) {
            return s.clone();
        }
        let limits = self.limits_for(sidecar_id);
        let s = Arc::new(SidecarState {
            sem: Arc::new(Semaphore::new(limits.max_concurrent.max(1))),
            last: Arc::new(tokio::sync::Mutex::new(
                Instant::now() - Duration::from_secs(60),
            )),
            limits,
        });
        states.insert(sidecar_id.to_string(), s.clone());
        s
    }

    /// Acquire one slot's worth of permission to call `sidecar_id`.
    /// Honours the concurrency cap (waits on the semaphore) and the
    /// min-interval policy (sleeps until enough time has passed since
    /// the previous call). The returned guard releases the permit when
    /// dropped; the caller must hold it for the full duration of the
    /// sidecar invocation.
    pub async fn acquire<'a>(&self, sidecar_id: &str) -> RateLimitGuard {
        let state = self.state_for(sidecar_id);
        let permit = state
            .sem
            .clone()
            .acquire_owned()
            .await
            .expect("rate-limit semaphore poisoned");
        // Enforce min-interval after grabbing the concurrency permit so
        // the queue order is preserved.
        let min_interval = Duration::from_millis(state.limits.min_interval_ms);
        if !min_interval.is_zero() {
            let mut last = state.last.lock().await;
            let elapsed = last.elapsed();
            if elapsed < min_interval {
                tokio::time::sleep(min_interval - elapsed).await;
            }
            *last = Instant::now();
        }
        RateLimitGuard { _permit: permit }
    }

    fn persist(&self, cfg: &RateLimitConfig) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).ok();
        }
        let body = serde_json::to_string_pretty(cfg)?;
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, body)
            .with_context(|| format!("writing {}", tmp.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600));
        }
        fs::rename(&tmp, &self.path)
            .with_context(|| format!("renaming {} → {}", tmp.display(), self.path.display()))?;
        Ok(())
    }
}

/// RAII guard that releases the semaphore permit on drop.
pub struct RateLimitGuard {
    _permit: tokio::sync::OwnedSemaphorePermit,
}
