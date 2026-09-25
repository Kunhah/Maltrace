//! Per-user secret store for sidecar API keys / credentials.
//!
//! Sidecars (especially SpiderFoot's 100+ `sfp_*` modules that consume
//! external APIs) need API keys to do anything useful. We don't want
//! those keys in the case database (so users can ship cases without
//! leaking credentials).
//!
//! Two storage backends, picked at open time:
//!   1. **OS keychain** (`keyring` crate). On macOS this is the system
//!      Keychain; on Windows, Credential Manager; on Linux,
//!      Secret-Service via DBus (kwallet / gnome-keyring). Used by
//!      default when accessible.
//!   2. **File** (`secrets.json` in the app data directory, mode 0600 on
//!      Unix). Fallback for headless Linux / containers where the
//!      keychain isn't reachable. Less secure but still scoped to the
//!      user account.
//!
//! On first run with a keychain available, an existing plaintext
//! `secrets.json` is migrated into the keychain and the file is
//! deleted. The migration is one-way.
//!
//! All API methods that list secrets return *names only*. Values only
//! leave the store via `snapshot_for_sidecar()`, which the run path
//! uses to pass them as `--opts` JSON to the sidecar — values never
//! cross the UI/HTTP boundary.

use anyhow::{Context, Result};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize)]
pub struct SecretInfo {
    pub name: String,
    /// Always `true` for entries returned by the store — the field exists
    /// so the UI can render presets (well-known key names with `set=false`)
    /// merged with stored entries uniformly.
    pub set: bool,
}

/// Service / account pair the keyring backend uses. One keychain entry
/// holds the entire JSON blob — simpler than per-secret entries and
/// keeps the `snapshot_for_sidecar` path a single OS-call instead of N.
const KEYRING_SERVICE: &str = "dev.maltrace.app";
const KEYRING_ACCOUNT: &str = "secrets";

#[derive(Debug)]
enum Backend {
    /// OS keychain (preferred). The `keyring::Entry` is opaque to us;
    /// we serialise the whole secrets map as a single string value.
    Keychain(keyring::Entry),
    /// Plaintext JSON file. The path is the existing
    /// `<data_dir>/secrets.json` location.
    File(PathBuf),
}

#[derive(Debug)]
pub struct SecretStore {
    backend: Backend,
    inner: Mutex<HashMap<String, String>>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct OnDisk {
    /// Map of secret name → secret value. Names are free-form strings —
    /// SpiderFoot's `api_key` option per module; users pick names that
    /// match the option key the sidecar expects.
    secrets: HashMap<String, String>,
}

impl SecretStore {
    /// Open the secrets store. Tries the OS keychain first; falls back
    /// to a plaintext file when the keychain isn't reachable (headless
    /// Linux / containers). If a legacy file exists when the keychain
    /// is healthy, its contents are migrated into the keychain and the
    /// file is deleted — one-way upgrade so subsequent runs use the
    /// keychain unconditionally.
    pub fn open(data_dir: &std::path::Path) -> Self {
        let file_path = data_dir.join("secrets.json");
        // 1. Try the keychain. We don't fail boot if it's unavailable —
        //    fall through to file in that case.
        if let Some(entry) = Self::try_open_keychain() {
            let mut inner = Self::read_keychain(&entry).unwrap_or_default();
            // 2. One-shot migration: if the keychain is empty but a
            //    legacy plaintext file has data, fold it in and remove
            //    the file. Idempotent — the keychain is the canonical
            //    store after this.
            if inner.is_empty() {
                if let Ok(file_secrets) = Self::read_from(&file_path) {
                    if !file_secrets.is_empty() {
                        inner = file_secrets;
                        if let Err(e) = Self::write_keychain(&entry, &inner) {
                            eprintln!(
                                "maltrace secrets: migration to keychain failed ({e}); keeping the file backend"
                            );
                            return Self {
                                backend: Backend::File(file_path),
                                inner: Mutex::new(inner),
                            };
                        }
                        // Migration succeeded — scrub the plaintext file.
                        let _ = fs::remove_file(&file_path);
                        eprintln!(
                            "maltrace secrets: migrated {} secret(s) from {} into the OS keychain; file removed.",
                            inner.len(),
                            file_path.display()
                        );
                    }
                }
            }
            return Self {
                backend: Backend::Keychain(entry),
                inner: Mutex::new(inner),
            };
        }
        // 3. No keychain — file fallback. Missing file = empty store.
        eprintln!(
            "maltrace secrets: OS keychain unavailable; using file backend at {} (mode 0600)",
            file_path.display()
        );
        let inner = Self::read_from(&file_path).unwrap_or_else(|e| {
            eprintln!(
                "maltrace secrets: starting with empty store ({}). File at {} will be (re)created on first write.",
                e,
                file_path.display()
            );
            HashMap::new()
        });
        Self {
            backend: Backend::File(file_path),
            inner: Mutex::new(inner),
        }
    }

    fn try_open_keychain() -> Option<keyring::Entry> {
        match keyring::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT) {
            Ok(entry) => {
                // Probe with a read so we surface "no DBus" / "no
                // Secret-Service" failures right here rather than on the
                // first set_password call.
                match entry.get_password() {
                    Ok(_) | Err(keyring::Error::NoEntry) => Some(entry),
                    Err(e) => {
                        eprintln!("maltrace secrets: keychain probe failed: {e}");
                        None
                    }
                }
            }
            Err(e) => {
                eprintln!("maltrace secrets: keychain init failed: {e}");
                None
            }
        }
    }

    fn read_keychain(entry: &keyring::Entry) -> Result<HashMap<String, String>> {
        match entry.get_password() {
            Ok(s) => {
                let parsed: OnDisk = serde_json::from_str(&s)
                    .context("parsing JSON from keychain entry")?;
                Ok(parsed.secrets)
            }
            Err(keyring::Error::NoEntry) => Ok(HashMap::new()),
            Err(e) => Err(anyhow::anyhow!("keychain read: {e}")),
        }
    }

    fn write_keychain(
        entry: &keyring::Entry,
        secrets: &HashMap<String, String>,
    ) -> Result<()> {
        let body = serde_json::to_string(&OnDisk {
            secrets: secrets.clone(),
        })?;
        entry
            .set_password(&body)
            .map_err(|e| anyhow::anyhow!("keychain write: {e}"))?;
        Ok(())
    }

    fn read_from(path: &std::path::Path) -> Result<HashMap<String, String>> {
        if !path.exists() {
            return Ok(HashMap::new());
        }
        let body = fs::read_to_string(path)
            .with_context(|| format!("reading {}", path.display()))?;
        let parsed: OnDisk = serde_json::from_str(&body)
            .with_context(|| format!("parsing {}", path.display()))?;
        Ok(parsed.secrets)
    }

    /// Names of all stored secrets, alphabetised so the UI is stable.
    pub fn list_names(&self) -> Vec<SecretInfo> {
        let inner = self.inner.lock();
        let mut names: Vec<String> = inner.keys().cloned().collect();
        names.sort();
        names
            .into_iter()
            .map(|name| SecretInfo { name, set: true })
            .collect()
    }

    pub fn set(&self, name: &str, value: &str) -> Result<()> {
        if name.trim().is_empty() {
            anyhow::bail!("secret name cannot be empty");
        }
        let mut inner = self.inner.lock();
        inner.insert(name.trim().to_string(), value.to_string());
        self.persist(&inner)
    }

    pub fn delete(&self, name: &str) -> Result<()> {
        let mut inner = self.inner.lock();
        inner.remove(name);
        self.persist(&inner)
    }

    /// Snapshot of every stored (name, value) for handing to a sidecar.
    /// Returned as an owned map so the caller doesn't hold the lock while
    /// awaiting a subprocess.
    pub fn snapshot_for_sidecar(&self) -> HashMap<String, String> {
        self.inner.lock().clone()
    }

    fn persist(&self, inner: &HashMap<String, String>) -> Result<()> {
        match &self.backend {
            Backend::Keychain(entry) => Self::write_keychain(entry, inner),
            Backend::File(path) => Self::persist_file(path, inner),
        }
    }

    fn persist_file(path: &std::path::Path, inner: &HashMap<String, String>) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).ok();
        }
        let body = serde_json::to_string_pretty(&OnDisk {
            secrets: inner.clone(),
        })?;
        // Atomic write: stage to a sibling temp file, then rename.
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, body)
            .with_context(|| format!("writing {}", tmp.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600));
        }
        fs::rename(&tmp, path)
            .with_context(|| format!("renaming {} → {}", tmp.display(), path.display()))?;
        Ok(())
    }

    /// Backend identifier (`"keychain"` or `"file"`) — used by the
    /// Settings panel to show the user where their secrets live.
    pub fn backend_id(&self) -> &'static str {
        match self.backend {
            Backend::Keychain(_) => "keychain",
            Backend::File(_) => "file",
        }
    }
}
