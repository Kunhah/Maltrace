//! Plugin manifest schema (the public SDK contract).
//!
//! Every sidecar plugin ships a `plugin.toml` at its root. The registry
//! scans for these on startup instead of using a hardcoded list, so adding
//! a plugin is "drop a directory in, restart" — no Rust edit required.
//!
//! The schema is intentionally small. Today it's just enough to invoke the
//! plugin and label it in the UI; future protocol versions can add fields
//! without breaking old plugins because every entry is `#[serde(default)]`
//! where appropriate.

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Current sidecar JSON protocol version.
///
/// Plugins declare which version they implement via `plugin.protocol_version`.
/// The registry skips any plugin whose declared version doesn't match (with
/// a stderr warning) — so users get a clear "this plugin needs an update"
/// signal instead of mysterious failures when we evolve the protocol.
pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    pub plugin: PluginInfo,
    pub runner: RunnerSpec,
    #[serde(default)]
    pub requirements: Requirements,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginInfo {
    /// Unique id within an install (e.g. "spiderfoot", "face-search").
    /// Used as the `source` field on every transform this plugin contributes.
    pub id: String,
    /// Human-readable name shown in the UI.
    pub name: String,
    /// SemVer string. Cosmetic for now; we don't enforce version policy.
    pub version: String,
    /// Sidecar protocol version this plugin implements. Must equal
    /// `PROTOCOL_VERSION` or the plugin is skipped.
    pub protocol_version: u32,
    /// SPDX license id (e.g. "MIT"). Surfaced in the plugins list.
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    pub description: String,
    /// Optional URL to the plugin's homepage / source.
    #[serde(default)]
    pub homepage: Option<String>,
    /// Optional list of author strings — purely informational.
    #[serde(default)]
    pub authors: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunnerSpec {
    /// Executable to invoke. **Resolved relative to the plugin directory** if
    /// it's a relative path (e.g. `.venv/bin/python`). Absolute paths are
    /// used as-is — but discouraged, since they're machine-specific.
    pub command: String,
    /// Fixed args passed before the maltrace subcommand (`list-transforms` /
    /// `run …`). Typically just the entry script path: `["adapter.py"]`.
    #[serde(default)]
    pub args: Vec<String>,
    /// Extra environment variables set on the subprocess. The plugin can
    /// also read variables from the maltrace process's own env, so this is
    /// mostly for things you want to pin per-plugin (e.g. `PYTHONUNBUFFERED=1`).
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
}

/// Human-readable requirement hints. Not enforced — they're documentation
/// for the user setting up the plugin venv / system deps.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Requirements {
    #[serde(default)]
    pub python: Option<String>,
    #[serde(default)]
    pub system: Vec<String>,
    #[serde(default)]
    pub network: bool,
}

impl PluginManifest {
    /// Load and validate a `plugin.toml` at `manifest_path`. Returns a tuple
    /// of (manifest, plugin_dir) — the latter is the directory the manifest
    /// lives in, used to resolve relative `runner.command` paths.
    pub fn load_from_file(manifest_path: &Path) -> Result<(Self, PathBuf)> {
        let text = std::fs::read_to_string(manifest_path)
            .with_context(|| format!("reading {}", manifest_path.display()))?;
        let manifest: Self = toml::from_str(&text)
            .with_context(|| format!("parsing {}", manifest_path.display()))?;
        let plugin_dir = manifest_path
            .parent()
            .ok_or_else(|| anyhow!("{} has no parent dir", manifest_path.display()))?
            .to_path_buf();
        manifest.validate()?;
        Ok((manifest, plugin_dir))
    }

    fn validate(&self) -> Result<()> {
        if self.plugin.id.is_empty() {
            return Err(anyhow!("plugin.id is required"));
        }
        if !self
            .plugin
            .id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(anyhow!(
                "plugin.id must be ascii-alphanumeric / `-` / `_` (got {:?})",
                self.plugin.id
            ));
        }
        if self.plugin.protocol_version != PROTOCOL_VERSION {
            return Err(anyhow!(
                "plugin.protocol_version = {} but maltrace implements protocol {}",
                self.plugin.protocol_version,
                PROTOCOL_VERSION,
            ));
        }
        if self.runner.command.is_empty() {
            return Err(anyhow!("runner.command is required"));
        }
        Ok(())
    }

    /// Resolve `runner.command` against `plugin_dir`. Absolute paths
    /// pass through; relative paths join under the plugin dir.
    pub fn resolved_command(&self, plugin_dir: &Path) -> PathBuf {
        let cmd = Path::new(&self.runner.command);
        if cmd.is_absolute() {
            cmd.to_path_buf()
        } else {
            plugin_dir.join(cmd)
        }
    }
}

/// Frontend-facing summary returned by `list_plugins`. The transform count
/// is filled in by the registry after a `list_transforms` round-trip.
#[derive(Debug, Clone, Serialize)]
pub struct PluginSummary {
    pub id: String,
    pub name: String,
    pub version: String,
    pub license: String,
    pub description: String,
    pub homepage: Option<String>,
    /// Path to the plugin directory (string for the frontend's sake).
    pub plugin_dir: String,
    /// Transforms this plugin contributed during the last `list_transforms`.
    /// `None` until the catalogue has been fetched at least once.
    pub transform_count: Option<usize>,
    /// True when the plugin's runner command actually exists on disk. False
    /// usually means the venv hasn't been set up yet — UI shows a hint.
    pub runnable: bool,
    /// Trusted-key name that signed this plugin, or `None` for
    /// in-tree plugins (signature exempt) and plugins loaded under a
    /// non-strict trust policy. Surfaced as a "signed by X" badge in
    /// the plugins panel.
    #[serde(default)]
    pub signer: Option<String>,
}
