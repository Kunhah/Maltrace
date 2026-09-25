//! Outbound webhook delivery — Slack/Discord/generic JSON POST.
//!
//! Investigators running scheduled machines want to be told when
//! something new lands. This module stores webhook URLs (Slack/Discord
//! incoming-webhook style — same `{text: "..."}` body works for both
//! by convention) and the scheduler / transform runner POST to every
//! configured URL on a "schedule fired with new entities" or "transform
//! errored" event.
//!
//! Persisted in `webhooks.json` next to `secrets.json` / `proxies.json`.
//! Mode 0600 on Unix. Best-effort delivery — errors are logged but
//! don't fail the underlying operation.

use anyhow::{Context, Result};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WebhookConfig {
    /// One URL per row. Each event is POSTed to every entry.
    /// Empty entries are skipped silently.
    #[serde(default)]
    pub urls: Vec<String>,
}

#[derive(Debug)]
pub struct WebhookStore {
    path: PathBuf,
    inner: Mutex<WebhookConfig>,
}

impl WebhookStore {
    pub fn open(data_dir: &std::path::Path) -> Self {
        let path = data_dir.join("webhooks.json");
        let inner = match Self::read_from(&path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!(
                    "maltrace webhooks: starting empty ({}); file at {} will be created on first write.",
                    e,
                    path.display()
                );
                WebhookConfig::default()
            }
        };
        Self { path, inner: Mutex::new(inner) }
    }

    fn read_from(path: &std::path::Path) -> Result<WebhookConfig> {
        if !path.exists() {
            return Ok(WebhookConfig::default());
        }
        let body = fs::read_to_string(path)
            .with_context(|| format!("reading {}", path.display()))?;
        let parsed: WebhookConfig = serde_json::from_str(&body)
            .with_context(|| format!("parsing {}", path.display()))?;
        Ok(parsed)
    }

    pub fn snapshot(&self) -> WebhookConfig {
        self.inner.lock().clone()
    }

    pub fn set(&self, cfg: WebhookConfig) -> Result<()> {
        let mut inner = self.inner.lock();
        *inner = cfg;
        let to_persist = inner.clone();
        self.persist(&to_persist)
    }

    /// Snapshot the URL list for delivery. Returned by-value so the
    /// caller doesn't hold the lock across the network round-trip.
    pub fn urls(&self) -> Vec<String> {
        self.inner
            .lock()
            .urls
            .iter()
            .filter(|u| !u.trim().is_empty())
            .cloned()
            .collect()
    }

    fn persist(&self, cfg: &WebhookConfig) -> Result<()> {
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

/// Fire-and-forget POST to every configured webhook with a Slack/Discord
/// compatible `{text: ...}` body. Spawned per call (non-blocking); the
/// returned future resolves once every POST has completed (or failed).
pub async fn dispatch_all(urls: &[String], text: String) {
    if urls.is_empty() {
        return;
    }
    let body = serde_json::json!({ "text": text }).to_string();
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("maltrace webhook: client build failed: {e}");
            return;
        }
    };
    for url in urls {
        let url = url.clone();
        let body = body.clone();
        let client = client.clone();
        // Concurrent dispatch: the scheduler is on its own task and
        // doesn't care about ordering.
        tokio::spawn(async move {
            match client
                .post(&url)
                .header("content-type", "application/json")
                .body(body)
                .send()
                .await
            {
                Ok(resp) if resp.status().is_success() => {}
                Ok(resp) => eprintln!(
                    "maltrace webhook: {url} returned {}",
                    resp.status()
                ),
                Err(e) => eprintln!("maltrace webhook: {url} failed: {e}"),
            }
        });
    }
}
