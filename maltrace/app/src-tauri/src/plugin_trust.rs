//! Plugin signature verification (ed25519).
//!
//! Trust model:
//!   - The user-installed plugins directory may contain `plugin.toml.sig`
//!     beside each plugin's `plugin.toml`. The sig file is a single
//!     base64 line: the ed25519 detached signature of the toml bytes.
//!   - `trusted_keys.json` in the app data dir lists every public key
//!     that is allowed to sign plugins: `{keys: [{name, key_b64}]}`.
//!   - Policy at registry-load time is read from `MALTRACE_PLUGIN_TRUST`:
//!       - `signed` (default): plugin is loaded only when the sig
//!         verifies against a trusted key. In-tree plugins (under
//!         `<repo>/sidecars/`) are exempt — they ship with the
//!         maltrace checkout itself.
//!       - `warn`: unsigned/invalid plugins still load but log a
//!         warning.
//!       - `off`: signatures are not checked at all (legacy /
//!         development mode).
//!
//! In-tree exemption: signing every checked-in plugin would be a hassle
//! during development and adds nothing — anyone who can drop a plugin
//! into the repo can also tamper with maltrace itself. The exemption
//! is by *path*: anything under `<CARGO_MANIFEST_DIR>/../../sidecars/`
//! is trusted unconditionally.
//!
//! Signing a plugin (operator workflow, doc only here):
//!   1. Generate a keypair with `maltrace-server --gen-plugin-key NAME`
//!      (or use any ed25519 tool — openssl, sshkeygen-ed25519, etc.).
//!   2. Add the public key to `trusted_keys.json` on every host that
//!      will install the plugin.
//!   3. Sign with `openssl pkeyutl -sign -inkey ed25519.pem -rawin -in plugin.toml | base64 > plugin.toml.sig`
//!      (or the equivalent in your toolchain).
//!   4. Distribute `plugin.toml` + `plugin.toml.sig` + the rest of the
//!      plugin tree.

use anyhow::{anyhow, Context, Result};
use base64::engine::{general_purpose::STANDARD as BASE64, Engine as _};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TrustPolicy {
    /// Reject unsigned/invalid plugins. Default.
    Signed,
    /// Log + load anyway. Useful while bootstrapping a key infra.
    Warn,
    /// Don't check signatures at all. Dev / single-user.
    Off,
}

impl Default for TrustPolicy {
    fn default() -> Self {
        Self::Signed
    }
}

impl TrustPolicy {
    pub fn from_env() -> Self {
        match std::env::var("MALTRACE_PLUGIN_TRUST").as_deref() {
            Ok("warn") => Self::Warn,
            Ok("off") => Self::Off,
            _ => Self::Signed,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustedKey {
    pub name: String,
    /// Base64-encoded raw ed25519 public key (32 bytes after decode).
    pub key_b64: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TrustedKeysFile {
    #[serde(default)]
    pub keys: Vec<TrustedKey>,
}

/// In-memory snapshot of the trusted keys, parsed once at startup.
#[derive(Debug, Clone)]
pub struct TrustStore {
    pub policy: TrustPolicy,
    keys: Vec<(String, VerifyingKey)>,
}

impl TrustStore {
    pub fn open(data_dir: &Path) -> Self {
        let policy = TrustPolicy::from_env();
        let path = data_dir.join("trusted_keys.json");
        let keys = if path.exists() {
            match Self::read_keys(&path) {
                Ok(ks) => ks,
                Err(e) => {
                    eprintln!(
                        "maltrace plugin-trust: failed to read {}: {e}; treating key list as empty",
                        path.display()
                    );
                    Vec::new()
                }
            }
        } else {
            Vec::new()
        };
        if policy == TrustPolicy::Signed && keys.is_empty() {
            eprintln!(
                "maltrace plugin-trust: SIGNED policy active but {} has no keys — only in-tree plugins will load. Set MALTRACE_PLUGIN_TRUST=warn or =off to relax.",
                path.display()
            );
        }
        Self { policy, keys }
    }

    fn read_keys(path: &Path) -> Result<Vec<(String, VerifyingKey)>> {
        let body = fs::read_to_string(path)
            .with_context(|| format!("reading {}", path.display()))?;
        let parsed: TrustedKeysFile = serde_json::from_str(&body)
            .with_context(|| format!("parsing {}", path.display()))?;
        let mut out = Vec::with_capacity(parsed.keys.len());
        for k in parsed.keys {
            let raw = BASE64
                .decode(k.key_b64.as_bytes())
                .with_context(|| format!("base64 key {}", k.name))?;
            let arr: [u8; 32] = raw
                .as_slice()
                .try_into()
                .map_err(|_| anyhow!("trusted key {} is not 32 bytes", k.name))?;
            let vk = VerifyingKey::from_bytes(&arr)
                .with_context(|| format!("ed25519 key {} invalid", k.name))?;
            out.push((k.name, vk));
        }
        Ok(out)
    }

    /// Verify the signature for one plugin manifest path. Returns
    /// `Ok(Some(signer))` on success, `Ok(None)` when in-tree
    /// (signature skipped), `Err(...)` when verification failed.
    pub fn verify_plugin(&self, manifest_path: &Path) -> Result<Option<String>> {
        if self.policy == TrustPolicy::Off {
            return Ok(Some("(policy=off)".into()));
        }
        if is_in_tree(manifest_path) {
            return Ok(None);
        }
        let manifest_bytes = fs::read(manifest_path)
            .with_context(|| format!("reading {}", manifest_path.display()))?;
        let sig_path = manifest_path.with_extension("toml.sig");
        if !sig_path.exists() {
            return Err(anyhow!("no signature file at {}", sig_path.display()));
        }
        let sig_text = fs::read_to_string(&sig_path)
            .with_context(|| format!("reading {}", sig_path.display()))?;
        let sig_bytes = BASE64
            .decode(sig_text.trim().as_bytes())
            .context("base64 decode of signature failed")?;
        let sig_arr: [u8; 64] = sig_bytes
            .as_slice()
            .try_into()
            .map_err(|_| anyhow!("signature is not 64 bytes"))?;
        let sig = Signature::from_bytes(&sig_arr);
        for (name, vk) in &self.keys {
            if vk.verify(&manifest_bytes, &sig).is_ok() {
                return Ok(Some(name.clone()));
            }
        }
        Err(anyhow!(
            "signature does not match any trusted key ({} key{})",
            self.keys.len(),
            if self.keys.len() == 1 { "" } else { "s" }
        ))
    }
}

/// Best-effort check: is this plugin path under the maltrace
/// in-tree `sidecars/` directory? In-tree plugins are exempt from
/// signing — anyone who can edit the repo can also edit the
/// signature-verification logic itself.
fn is_in_tree(manifest_path: &Path) -> bool {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let in_tree_root = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("sidecars"));
    if let Some(root) = in_tree_root {
        if let (Ok(plugin_abs), Ok(root_abs)) = (
            std::fs::canonicalize(manifest_path),
            std::fs::canonicalize(&root),
        ) {
            return plugin_abs.starts_with(&root_abs);
        }
        // Path-component fallback when canonicalize fails (test envs).
        return manifest_path.starts_with(&root);
    }
    false
}
