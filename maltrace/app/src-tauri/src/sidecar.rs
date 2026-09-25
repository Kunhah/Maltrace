//! Python sidecar invocation, multi-sidecar version.
//!
//! Each sidecar is a directory under `maltrace/sidecars/` containing
//! `adapter.py` and a `.venv/`. The Rust core owns a [`SidecarRegistry`]
//! that knows about every sidecar and routes transform invocations to the
//! right one.
//!
//! Transforms carry a `source` field identifying which sidecar contributed
//! them. The registry caches a transform_id → sidecar_id index so
//! `run_transform` is a O(1) lookup, with a self-refresh on cache miss
//! (so a sidecar being added/removed at runtime is recoverable).
//!
//! Sidecars are isolated from each other: if one's venv is broken or its
//! adapter throws on `list-transforms`, the others still work — we log a
//! warning to stderr and drop the failed sidecar from the catalogue for
//! that listing. `run_transform` against the broken one still surfaces the
//! error verbatim, which is the right place to see it.

use anyhow::{anyhow, Context, Result};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::process::Command;

/// Metadata describing a single transform exposed by some sidecar. The
/// `source` field is filled in by the registry, not by the sidecar itself.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransformMeta {
    pub id: String,
    pub name: String,
    pub description: String,
    pub categories: Vec<String>,
    pub group: Vec<String>,
    pub consumes: Vec<String>,
    pub produces: Vec<String>,
    /// Which sidecar contributed this transform (e.g. "spiderfoot",
    /// "face-search"). Defaulted on deserialize so sidecars that don't set
    /// it (current behavior) still parse cleanly.
    #[serde(default)]
    pub source: String,
    /// Runtime-overridable knobs. When non-empty the UI renders a form
    /// before invoking the transform and ships the user's choices to
    /// the sidecar as the `MALTRACE_PARAMS` env var (JSON object).
    /// Adapters that don't know about parameters get the env var and
    /// can ignore it harmlessly.
    #[serde(default)]
    pub parameters: Vec<ParameterSpec>,
    /// OPSEC classification: does this transform touch the target
    /// directly (`active`), query third-party indices only (`passive`),
    /// or is it unclassified (`unknown`)? Set by the registry, not by
    /// the sidecar — looked up via `crate::passivity::classify`.
    #[serde(default)]
    pub passivity: String,
}

/// One user-facing parameter for a transform. Keep the spec narrow so
/// the UI form stays trivial.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParameterSpec {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: ParameterType,
    #[serde(default)]
    pub description: String,
    /// Default value, encoded as JSON. Strings are bare strings;
    /// numbers/booleans use their native JSON shape.
    #[serde(default)]
    pub default: Option<serde_json::Value>,
    /// Closed enum of allowed values when `ty == ParameterType::Choice`.
    #[serde(default)]
    pub choices: Vec<String>,
    /// Required vs optional. Defaults to required.
    #[serde(default = "default_true")]
    pub required: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ParameterType {
    String,
    Number,
    Boolean,
    Choice,
}

#[derive(Debug, Deserialize)]
struct ListResponse {
    transforms: Vec<TransformMeta>,
    #[allow(dead_code)]
    #[serde(default)]
    skipped: Vec<SkippedModule>,
}

#[derive(Debug, Deserialize)]
struct SkippedModule {
    #[allow(dead_code)]
    id: String,
    #[allow(dead_code)]
    reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProducedEvent {
    pub event_type: String,
    pub data: String,
    pub module: Option<String>,
    pub confidence: Option<i64>,
    pub source_event_type: Option<String>,
    /// Free-form structured metadata the sidecar wants to surface in the
    /// inspector — e.g. brazuca emits `{title, description, category, url}`,
    /// face_detect emits `{bbox, det_score, source_image}`. Stored on the
    /// entity as part of `entity.data` JSON. Optional; defaults to None.
    #[serde(default)]
    pub extra: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct RunResponse {
    ok: bool,
    #[serde(default)]
    events: Vec<ProducedEvent>,
    #[serde(default)]
    error: Option<String>,
}

/// One sidecar — a plugin that exposes transforms via the maltrace
/// subprocess JSON protocol. Constructed from a `plugin.toml` manifest;
/// see `crate::manifest::PluginManifest` for the schema.
#[derive(Debug, Clone)]
pub struct Sidecar {
    pub id: String,
    pub manifest: crate::manifest::PluginManifest,
    pub plugin_dir: PathBuf,
    /// Resolved absolute path to the runner command.
    pub command: PathBuf,
    /// Fixed args from `[runner].args` — passed before the maltrace
    /// subcommand on every invocation.
    pub runner_args: Vec<String>,
    /// Friendly name of the trusted key that signed this plugin's
    /// `plugin.toml`, or `None` for in-tree plugins (signature
    /// skipped) and for plugins loaded under a relaxed trust policy.
    /// Surfaced in the plugin list so the UI can show a "signed by X"
    /// badge.
    pub signer: Option<String>,
}

impl Sidecar {
    pub fn from_manifest(manifest: crate::manifest::PluginManifest, plugin_dir: PathBuf) -> Self {
        let command = manifest.resolved_command(&plugin_dir);
        let runner_args = manifest.runner.args.clone();
        Self {
            id: manifest.plugin.id.clone(),
            manifest,
            plugin_dir,
            command,
            runner_args,
            signer: None,
        }
    }

    fn validate(&self) -> Result<()> {
        if !self.command.exists() {
            return Err(anyhow!(
                "plugin {}: runner command not found at {} (typically means the venv hasn't been set up; see the plugin's README)",
                self.id,
                self.command.display(),
            ));
        }
        Ok(())
    }

    /// Spawn the configured runner with extra env vars from the manifest.
    fn spawn(&self) -> Command {
        let mut cmd = Command::new(&self.command);
        // Run from the plugin's own directory. Runner args (e.g. `adapter.py`)
        // and relative paths inside the adapter are written relative to the
        // plugin dir, not to wherever the maltrace binary happens to be
        // launched from (the Tauri build runs from `app/src-tauri`). Without
        // this, every sidecar fails `list-transforms` with "can't open file
        // adapter.py" and gets dropped from the catalogue.
        cmd.current_dir(&self.plugin_dir);
        for a in &self.runner_args {
            cmd.arg(a);
        }
        for (k, v) in &self.manifest.runner.env {
            cmd.env(k, v);
        }
        cmd
    }

    pub async fn list_transforms(&self) -> Result<Vec<TransformMeta>> {
        self.validate()?;
        let output = self
            .spawn()
            .arg("list-transforms")
            .output()
            .await
            .context("spawning sidecar for list-transforms")?;
        if !output.status.success() {
            return Err(anyhow!(
                "sidecar {} list-transforms exited {}: {}",
                self.id,
                output.status,
                String::from_utf8_lossy(&output.stderr).chars().take(500).collect::<String>(),
            ));
        }
        let parsed: ListResponse =
            serde_json::from_slice(&output.stdout).with_context(|| {
                format!(
                    "parsing sidecar {} list-transforms output: {}",
                    self.id,
                    String::from_utf8_lossy(&output.stdout).chars().take(500).collect::<String>(),
                )
            })?;
        Ok(parsed.transforms)
    }

    pub async fn run_transform(
        &self,
        module: &str,
        event_type: &str,
        data: &str,
        opts_json: &str,
        params_json: Option<&str>,
        proxy: Option<&str>,
    ) -> Result<TransformOutput> {
        self.validate()?;
        let mut cmd = self.spawn();
        cmd.arg("run")
            .arg("--module")
            .arg(module)
            .arg("--event-type")
            .arg(event_type)
            .arg("--data")
            .arg(data);
        if !opts_json.is_empty() && opts_json != "{}" {
            cmd.arg("--opts").arg(opts_json);
        }
        // Parameters + proxy travel as env vars rather than argv
        // entries so adapters that don't know about them keep working.
        // Adapters that want them read `MALTRACE_PARAMS` /
        // `MALTRACE_PROXY` directly; standard `HTTP_PROXY` /
        // `HTTPS_PROXY` / `ALL_PROXY` cover the common Python clients
        // (requests, httpx, urllib) without sidecar code changes.
        if let Some(p) = params_json {
            cmd.env("MALTRACE_PARAMS", p);
        }
        if let Some(url) = proxy {
            cmd.env("MALTRACE_PROXY", url);
            cmd.env("HTTP_PROXY", url);
            cmd.env("HTTPS_PROXY", url);
            cmd.env("ALL_PROXY", url);
        }
        let output = cmd.output().await.context("spawning sidecar for run")?;
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

        // Exit-3 = module raised an exception, but stdout is still valid JSON.
        let parsed: RunResponse =
            serde_json::from_slice(&output.stdout).with_context(|| {
                format!(
                    "parsing sidecar {} run output (exit {}): stdout={:?} stderr={:?}",
                    self.id,
                    output.status,
                    String::from_utf8_lossy(&output.stdout).chars().take(500).collect::<String>(),
                    stderr.chars().take(500).collect::<String>(),
                )
            })?;

        if !parsed.ok {
            return Err(anyhow!(
                "transform {module} failed in sidecar {}: {}\n--stderr--\n{}",
                self.id,
                parsed.error.unwrap_or_else(|| "unknown error".into()),
                stderr.chars().take(2000).collect::<String>(),
            ));
        }
        Ok(TransformOutput { events: parsed.events, stderr })
    }
}

/// Successful sidecar run output. Stderr is kept around so the run log can
/// surface diagnostics (rate-limits, warnings) even when the transform
/// itself succeeded.
#[derive(Debug, Clone)]
pub struct TransformOutput {
    pub events: Vec<ProducedEvent>,
    pub stderr: String,
}

/// Per-sidecar load result, captured the last time
/// [`SidecarRegistry::list_all_transforms`] ran. Surfaced to the UI so a
/// sidecar that fails to list its transforms (broken venv, adapter
/// exception, …) is no longer silently dropped — the frontend can warn
/// the user instead.
#[derive(Debug, Clone, Serialize)]
pub struct SidecarStatus {
    pub id: String,
    pub ok: bool,
    pub transform_count: usize,
    pub error: Option<String>,
}

/// A registry of all sidecars maltrace knows about. Constructed once at
/// startup; cloned cheaply through Arc. `sidecars` itself sits behind a
/// Mutex so the in-app plugin installer can swap in a fresh discovery
/// without restarting the process.
#[derive(Debug, Clone)]
pub struct SidecarRegistry {
    sidecars: Arc<Mutex<Vec<Arc<Sidecar>>>>,
    /// transform_id → sidecar_id. Populated lazily by `list_all_transforms`.
    transform_index: Arc<Mutex<HashMap<String, String>>>,
    /// transform_id → full metadata. Populated alongside `transform_index`
    /// by `list_all_transforms`. Lets callers consult a transform's
    /// `consumes` list (e.g. to pick the right input event type) without
    /// re-spawning the owning sidecar.
    transform_meta: Arc<Mutex<HashMap<String, TransformMeta>>>,
    /// Per-sidecar load status from the last `list_all_transforms` run.
    /// Lets the `list_sidecar_status` command surface sidecars that
    /// failed to list (which are otherwise dropped from the catalogue).
    status: Arc<Mutex<Vec<SidecarStatus>>>,
    /// Trust store used at scan time to verify `plugin.toml.sig`.
    /// Wrapped in Arc so cloning the registry stays cheap.
    trust: Arc<crate::plugin_trust::TrustStore>,
}

impl SidecarRegistry {
    /// Discover plugins by scanning the conventional locations for
    /// `plugin.toml` manifests:
    ///   1. `<repo>/sidecars/*/plugin.toml`               (in-tree plugins)
    ///   2. `<XDG_DATA_HOME>/dev.maltrace.app/plugins/*/plugin.toml`
    ///      (user-installed plugins — same data dir as the SQLite db)
    ///
    /// Adding a plugin = drop the directory in, restart the app. No Rust
    /// edit required. Plugins with malformed manifests are skipped with a
    /// stderr warning so one bad plugin doesn't break the others.
    pub fn from_repo_layout(trust: Arc<crate::plugin_trust::TrustStore>) -> Self {
        let me = Self {
            sidecars: Arc::new(Mutex::new(Vec::new())),
            transform_index: Arc::new(Mutex::new(HashMap::new())),
            transform_meta: Arc::new(Mutex::new(HashMap::new())),
            status: Arc::new(Mutex::new(Vec::new())),
            trust,
        };
        me.reload();
        if me.sidecars.lock().is_empty() {
            eprintln!(
                "maltrace: no plugins found. Looked in: {:?}",
                discovery_paths()
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>(),
            );
        }
        me
    }

    /// Re-scan the conventional plugin discovery paths and replace the
    /// in-memory plugin list in place. Called after a successful plugin
    /// install / uninstall so freshly-added plugins show up without an
    /// app restart. Also drops the transform index so the next
    /// `list_all_transforms` populates it from scratch.
    pub fn reload(&self) {
        let mut sidecars = Vec::new();
        let mut seen_ids = std::collections::HashSet::new();
        let policy = self.trust.policy;
        for dir in discovery_paths() {
            for (sc, manifest_path) in scan_plugins_dir(&dir) {
                let signer = match self.trust.verify_plugin(&manifest_path) {
                    Ok(s) => s,
                    Err(e) => match policy {
                        crate::plugin_trust::TrustPolicy::Signed => {
                            eprintln!(
                                "maltrace plugin-trust: rejecting {} — {}",
                                manifest_path.display(),
                                e
                            );
                            continue;
                        }
                        crate::plugin_trust::TrustPolicy::Warn => {
                            eprintln!(
                                "maltrace plugin-trust: WARNING {} — {} (loading anyway, MALTRACE_PLUGIN_TRUST=warn)",
                                manifest_path.display(),
                                e
                            );
                            None
                        }
                        crate::plugin_trust::TrustPolicy::Off => None,
                    },
                };
                let mut sc = sc;
                sc.signer = signer;
                if seen_ids.insert(sc.id.clone()) {
                    sidecars.push(Arc::new(sc));
                } else {
                    eprintln!(
                        "maltrace: plugin id {:?} already registered; skipping duplicate at {}",
                        sc.id,
                        sc.plugin_dir.display(),
                    );
                }
            }
        }
        *self.sidecars.lock() = sidecars;
        self.transform_index.lock().clear();
        self.transform_meta.lock().clear();
    }

    /// Snapshot the loaded plugins for the `list_plugins` command.
    pub fn list_plugins(&self) -> Vec<crate::manifest::PluginSummary> {
        let index = self.transform_index.lock();
        let mut counts: HashMap<String, usize> = HashMap::new();
        for sidecar_id in index.values() {
            *counts.entry(sidecar_id.clone()).or_insert(0) += 1;
        }
        let sidecars = self.sidecars.lock();
        sidecars
            .iter()
            .map(|sc| crate::manifest::PluginSummary {
                id: sc.id.clone(),
                name: sc.manifest.plugin.name.clone(),
                version: sc.manifest.plugin.version.clone(),
                license: sc.manifest.plugin.license.clone(),
                description: sc.manifest.plugin.description.clone(),
                homepage: sc.manifest.plugin.homepage.clone(),
                plugin_dir: sc.plugin_dir.display().to_string(),
                transform_count: counts.get(&sc.id).copied(),
                runnable: sc.command.exists(),
                signer: sc.signer.clone(),
            })
            .collect()
    }

    /// Aggregate every sidecar's transforms. Sidecars that fail to list
    /// (missing venv, adapter exception, …) are dropped with a warning to
    /// stderr; the rest still come through so the UI is usable in a
    /// partially-set-up environment.
    pub async fn list_all_transforms(&self) -> Result<Vec<TransformMeta>> {
        let mut all = Vec::with_capacity(64);
        // Build the new index in a local map first, then swap in under the
        // lock so we never hold it across an .await.
        let mut new_index: HashMap<String, String> = HashMap::new();
        let mut new_meta: HashMap<String, TransformMeta> = HashMap::new();
        // Clone the Arc<Sidecar> list so we don't hold the
        // `sidecars` Mutex across the .await for each list call.
        let sidecars: Vec<Arc<Sidecar>> = self.sidecars.lock().clone();
        // Per-sidecar load status, captured as we iterate so the UI can
        // warn about sidecars that fail (rather than dropping them silently).
        let mut new_status: Vec<SidecarStatus> = Vec::with_capacity(sidecars.len());

        for sc in &sidecars {
            match sc.list_transforms().await {
                Ok(tfs) => {
                    let count = tfs.len();
                    for mut t in tfs {
                        t.source = sc.id.clone();
                        // Enrich with OPSEC classification — same place as
                        // `source`. Serialised as a lowercase string the
                        // frontend can match on directly.
                        let p = crate::passivity::classify(&t.id);
                        t.passivity = match p {
                            crate::passivity::Passivity::Active => "active",
                            crate::passivity::Passivity::Passive => "passive",
                            crate::passivity::Passivity::Unknown => "unknown",
                        }
                        .to_string();
                        new_index.insert(t.id.clone(), sc.id.clone());
                        new_meta.insert(t.id.clone(), t.clone());
                        all.push(t);
                    }
                    new_status.push(SidecarStatus {
                        id: sc.id.clone(),
                        ok: true,
                        transform_count: count,
                        error: None,
                    });
                }
                Err(e) => {
                    eprintln!("maltrace: sidecar {} skipped — {}", sc.id, e);
                    new_status.push(SidecarStatus {
                        id: sc.id.clone(),
                        ok: false,
                        transform_count: 0,
                        error: Some(e.to_string()),
                    });
                }
            }
        }

        *self.transform_index.lock() = new_index;
        *self.transform_meta.lock() = new_meta;
        *self.status.lock() = new_status;
        Ok(all)
    }

    /// Snapshot the per-sidecar load status captured by the last
    /// `list_all_transforms` run. Empty until that has run at least once.
    pub fn sidecar_status(&self) -> Vec<SidecarStatus> {
        self.status.lock().clone()
    }

    /// The set of event types a transform accepts (`consumes`), if known.
    /// Returns `None` when the meta cache hasn't been warmed yet — callers
    /// can warm it via `list_all_transforms`.
    pub fn consumes_for(&self, transform_id: &str) -> Option<Vec<String>> {
        self.transform_meta
            .lock()
            .get(transform_id)
            .map(|m| m.consumes.clone())
    }

    /// Look up where a given transform lives. Used to resolve the
    /// per-sidecar proxy + rate-limit policy. Returns `None` when the
    /// index hasn't been populated yet — callers can warm it via
    /// `list_all_transforms` (which is what `run_transform` does).
    pub fn sidecar_for_transform(&self, transform_id: &str) -> Option<String> {
        self.transform_index.lock().get(transform_id).cloned()
    }

    /// Dispatch a transform invocation to the sidecar that owns it.
    /// `params_json` is the JSON-encoded user parameter map (or None);
    /// `proxy` is the resolved proxy URL for this sidecar (or None).
    /// `rate_limit` is acquired and held for the duration of the call.
    pub async fn run_transform(
        &self,
        transform_id: &str,
        event_type: &str,
        data: &str,
        opts_json: &str,
        params_json: Option<&str>,
        proxy: Option<&str>,
        rate_limiter: &crate::ratelimit::RateLimiter,
    ) -> Result<TransformOutput> {
        // Fast path: index hit.
        let sidecar_id = {
            let idx = self.transform_index.lock();
            idx.get(transform_id).cloned()
        };
        let sidecar_id = match sidecar_id {
            Some(id) => id,
            None => {
                // Cold start or new transform — refresh once, then retry.
                self.list_all_transforms().await?;
                let idx = self.transform_index.lock();
                idx.get(transform_id).cloned().ok_or_else(|| {
                    anyhow!(
                        "unknown transform: {transform_id} (not contributed by any sidecar)"
                    )
                })?
            }
        };
        let sidecar = {
            let sidecars = self.sidecars.lock();
            sidecars
                .iter()
                .find(|s| s.id == sidecar_id)
                .ok_or_else(|| {
                    anyhow!("internal: sidecar {sidecar_id} disappeared from registry")
                })?
                .clone()
        };
        // Hold the rate-limit guard for the whole invocation. The
        // Semaphore caps concurrency per sidecar; the min-interval is
        // enforced inside acquire().
        let _guard = rate_limiter.acquire(&sidecar_id).await;
        sidecar
            .run_transform(transform_id, event_type, data, opts_json, params_json, proxy)
            .await
    }
}

/// Directories scanned for `plugin.toml` at startup, in priority order
/// (earlier wins on id conflict).
fn discovery_paths() -> Vec<PathBuf> {
    let mut paths = Vec::with_capacity(2);

    // 1. In-tree: <repo>/sidecars/
    //    CARGO_MANIFEST_DIR == .../maltrace/app/src-tauri
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    if let Some(in_tree) = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("sidecars"))
    {
        paths.push(in_tree);
    }

    // 2. User-installed: <XDG_DATA_HOME>/dev.maltrace.app/plugins/
    //    Mirrors crate::lib::server_data_dir's resolution.
    if let Some(data) = dirs::data_dir() {
        paths.push(data.join(crate::BUNDLE_IDENTIFIER).join("plugins"));
    }

    // 3. Explicit override for development / CI.
    if let Ok(extra) = std::env::var("MALTRACE_PLUGIN_PATH") {
        for entry in std::env::split_paths(&extra) {
            paths.push(entry);
        }
    }

    paths
}

/// Scan a directory for `*/plugin.toml`. Returns `(sidecar,
/// manifest_path)` pairs so the caller can run the signature check
/// against the original manifest bytes. Bad ones are skipped with a
/// stderr warning.
fn scan_plugins_dir(dir: &Path) -> Vec<(Sidecar, PathBuf)> {
    let mut out = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return out, // dir doesn't exist or isn't readable — fine
    };
    for entry in entries.flatten() {
        let manifest_path = entry.path().join("plugin.toml");
        if !manifest_path.exists() {
            continue;
        }
        match crate::manifest::PluginManifest::load_from_file(&manifest_path) {
            Ok((m, plugin_dir)) => {
                out.push((Sidecar::from_manifest(m, plugin_dir), manifest_path));
            }
            Err(e) => {
                eprintln!(
                    "maltrace: plugin manifest at {} skipped — {}",
                    manifest_path.display(),
                    e
                );
            }
        }
    }
    out
}
