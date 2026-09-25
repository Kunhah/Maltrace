//! Per-sidecar network policy: outbound proxy/Tor.
//!
//! Operations-conscious investigators (and many CTI workflows) need every
//! transform to route via a SOCKS5/HTTP(S) proxy — Tor at
//! `socks5://127.0.0.1:9050` is the canonical example. Storage layout
//! mirrors [`crate::secrets::SecretStore`]: a JSON file in the app data
//! directory, mode 0600 on Unix, atomic-write on persist.
//!
//! Per-sidecar overrides win over the default. At run time the registry
//! sets `HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY`, and `MALTRACE_PROXY` env
//! vars on the subprocess so Python adapters using `requests`/`httpx`
//! pick it up without code changes.

use anyhow::{Context, Result};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct OnDisk {
    /// Applied to every sidecar that doesn't have a per-sidecar override.
    /// Empty string == no proxy.
    #[serde(default)]
    default_proxy: String,
    /// sidecar_id → proxy URL. Empty value means "no proxy, even if a
    /// default is set" (explicit opt-out for one sidecar).
    #[serde(default)]
    per_sidecar: HashMap<String, String>,
}

/// Wire-format snapshot the UI reads + writes via the settings panel.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProxyConfig {
    #[serde(default)]
    pub default_proxy: String,
    #[serde(default)]
    pub per_sidecar: HashMap<String, String>,
}

#[derive(Debug)]
pub struct ProxyStore {
    path: PathBuf,
    inner: Mutex<OnDisk>,
}

impl ProxyStore {
    pub fn open(data_dir: &std::path::Path) -> Self {
        let path = data_dir.join("proxies.json");
        let inner = match Self::read_from(&path) {
            Ok(o) => o,
            Err(e) => {
                eprintln!(
                    "maltrace proxies: starting empty ({}); file at {} will be created on first write.",
                    e,
                    path.display()
                );
                OnDisk::default()
            }
        };
        Self { path, inner: Mutex::new(inner) }
    }

    fn read_from(path: &std::path::Path) -> Result<OnDisk> {
        if !path.exists() {
            return Ok(OnDisk::default());
        }
        let body = fs::read_to_string(path)
            .with_context(|| format!("reading {}", path.display()))?;
        let parsed: OnDisk = serde_json::from_str(&body)
            .with_context(|| format!("parsing {}", path.display()))?;
        Ok(parsed)
    }

    pub fn snapshot(&self) -> ProxyConfig {
        let i = self.inner.lock();
        ProxyConfig {
            default_proxy: i.default_proxy.clone(),
            per_sidecar: i.per_sidecar.clone(),
        }
    }

    pub fn set(&self, cfg: ProxyConfig) -> Result<()> {
        let mut inner = self.inner.lock();
        inner.default_proxy = cfg.default_proxy;
        inner.per_sidecar = cfg.per_sidecar;
        self.persist(&inner)
    }

    /// Resolve the proxy URL to apply for a given sidecar id. Returns
    /// `None` when neither an override nor a default is set (or when an
    /// empty override explicitly opts the sidecar out).
    pub fn resolve(&self, sidecar_id: &str) -> Option<String> {
        let i = self.inner.lock();
        if let Some(v) = i.per_sidecar.get(sidecar_id) {
            return if v.is_empty() { None } else { Some(v.clone()) };
        }
        if i.default_proxy.is_empty() {
            None
        } else {
            Some(i.default_proxy.clone())
        }
    }

    fn persist(&self, inner: &OnDisk) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).ok();
        }
        let body = serde_json::to_string_pretty(inner)?;
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
