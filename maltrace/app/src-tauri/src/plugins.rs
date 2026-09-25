//! In-app plugin install + uninstall + venv bootstrap.
//!
//! The plugin SDK already supports drop-in install at
//! `~/.local/share/dev.maltrace.app/plugins/<id>/` — this module adds
//! the operations to do that drop-in *from the running app*:
//!
//!   - **install_from_directory**: copy a local plugin tree (containing
//!     `plugin.toml` at its root) into the user-plugins dir.
//!   - **install_from_tarball**: same, but the source is a base64-
//!     encoded `.tar.gz` from the browser/Tauri picker. Used for
//!     downloaded plugins.
//!   - **uninstall**: remove a previously-installed plugin (refuses to
//!     touch in-tree plugins, which are part of the maltrace install).
//!   - **bootstrap_venv**: if the plugin ships a `requirements.txt`,
//!     create a `.venv/` inside the plugin dir and `pip install -r` it.
//!
//! Each operation validates the `plugin.toml` after staging so a
//! malformed archive can't end up wedged in the user plugins dir.

use crate::manifest::PluginManifest;
use anyhow::{anyhow, Context, Result};
use base64::engine::{general_purpose::STANDARD as BASE64, Engine as _};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Stdio;

/// Where user-installed plugins live. Mirrors the discovery path in
/// `sidecar::discovery_paths` so a freshly-installed plugin is found
/// on the next registry `reload()`.
pub fn user_plugins_dir() -> PathBuf {
    let base = dirs::data_dir().unwrap_or_else(|| {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".local")
            .join("share")
    });
    base.join(crate::BUNDLE_IDENTIFIER).join("plugins")
}

#[derive(Debug, Serialize)]
pub struct InstallResult {
    pub plugin_id: String,
    pub plugin_dir: String,
    /// True when a `requirements.txt` was detected — the UI offers a
    /// one-click "Bootstrap venv" button only when this is set.
    pub needs_venv: bool,
}

/// Copy a local plugin tree into the user-plugins dir. `source_dir`
/// must contain a valid `plugin.toml` at its root. Refuses to overwrite
/// an existing install with the same id.
pub fn install_from_directory(source_dir: &Path) -> Result<InstallResult> {
    let manifest_path = source_dir.join("plugin.toml");
    if !manifest_path.exists() {
        return Err(anyhow!(
            "no plugin.toml at {} — not a valid plugin directory",
            source_dir.display()
        ));
    }
    let (manifest, _) = PluginManifest::load_from_file(&manifest_path)
        .context("validating source plugin.toml")?;
    let dest_root = user_plugins_dir();
    std::fs::create_dir_all(&dest_root).ok();
    let dest = dest_root.join(&manifest.plugin.id);
    if dest.exists() {
        return Err(anyhow!(
            "plugin {:?} already installed at {} — uninstall first",
            manifest.plugin.id,
            dest.display(),
        ));
    }
    copy_tree(source_dir, &dest)?;
    let needs_venv = dest.join("requirements.txt").exists();
    Ok(InstallResult {
        plugin_id: manifest.plugin.id.clone(),
        plugin_dir: dest.display().to_string(),
        needs_venv,
    })
}

/// Decode and extract a `.tar.gz` payload into a temp dir, then run
/// `install_from_directory`. Defends against path traversal by refusing
/// any entry whose normalised path escapes the temp staging area.
pub fn install_from_tarball(bytes_base64: &str) -> Result<InstallResult> {
    let bytes = decode_base64(bytes_base64)?;
    let staging = mkstaging()?;
    extract_tar_gz(&bytes, &staging)?;
    // Some tarballs wrap the plugin in a single top-level directory
    // (e.g. `my-plugin/plugin.toml`); others put plugin.toml at the
    // root. Probe both.
    let plugin_root = if staging.join("plugin.toml").exists() {
        staging.clone()
    } else {
        // Find the single child dir that contains plugin.toml.
        let mut candidates: Vec<PathBuf> = Vec::new();
        for entry in std::fs::read_dir(&staging)? {
            let p = entry?.path();
            if p.is_dir() && p.join("plugin.toml").exists() {
                candidates.push(p);
            }
        }
        if candidates.len() != 1 {
            return Err(anyhow!(
                "tarball must contain plugin.toml (found {} candidate roots)",
                candidates.len()
            ));
        }
        candidates.remove(0)
    };
    let result = install_from_directory(&plugin_root)?;
    // Best-effort cleanup of staging. Leaks on failure are harmless —
    // they're in a tmp dir.
    let _ = std::fs::remove_dir_all(&staging);
    Ok(result)
}

/// Remove a user-installed plugin. Refuses to remove in-tree plugins
/// (they're part of the maltrace checkout and would just come back on
/// the next launch).
pub fn uninstall(plugin_id: &str) -> Result<()> {
    let dest = user_plugins_dir().join(plugin_id);
    if !dest.exists() {
        return Err(anyhow!(
            "plugin {:?} not installed in {}",
            plugin_id,
            user_plugins_dir().display()
        ));
    }
    std::fs::remove_dir_all(&dest)
        .with_context(|| format!("removing {}", dest.display()))?;
    Ok(())
}

#[derive(Debug, Serialize)]
pub struct BootstrapResult {
    pub plugin_id: String,
    pub status: String, // "ok" | "error"
    pub stdout_excerpt: String,
    pub stderr_excerpt: String,
}

/// Create a Python venv inside the plugin dir and `pip install -r
/// requirements.txt`. Synchronous-on-our-side (the user clicks once and
/// the spinner shows until pip finishes); pip itself can take a couple
/// of minutes for heavy deps. Returns stdout/stderr excerpts so the UI
/// can surface diagnostics.
pub fn bootstrap_venv(plugin_id: &str, python_bin: Option<&str>) -> Result<BootstrapResult> {
    let dir = user_plugins_dir().join(plugin_id);
    if !dir.exists() {
        return Err(anyhow!(
            "plugin {:?} not installed in {}",
            plugin_id,
            user_plugins_dir().display()
        ));
    }
    let req = dir.join("requirements.txt");
    if !req.exists() {
        return Err(anyhow!(
            "no requirements.txt in {} — nothing to bootstrap",
            dir.display()
        ));
    }
    let python = python_bin.unwrap_or("python3");
    // 1) Create venv. If `.venv/` already exists, skip — pip-install
    // straight into the existing one.
    let venv = dir.join(".venv");
    let mut stdout = String::new();
    let mut stderr = String::new();
    if !venv.exists() {
        let out = std::process::Command::new(python)
            .arg("-m")
            .arg("venv")
            .arg(&venv)
            .current_dir(&dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .with_context(|| format!("running {python} -m venv"))?;
        push_excerpt(&mut stdout, &out.stdout);
        push_excerpt(&mut stderr, &out.stderr);
        if !out.status.success() {
            return Ok(BootstrapResult {
                plugin_id: plugin_id.to_string(),
                status: "error".into(),
                stdout_excerpt: stdout,
                stderr_excerpt: stderr,
            });
        }
    }
    // 2) pip install -r requirements.txt. The platform-dependent
    // pip path: `.venv/bin/pip` on Unix, `.venv\Scripts\pip.exe` on
    // Windows.
    let pip = if cfg!(windows) {
        venv.join("Scripts").join("pip.exe")
    } else {
        venv.join("bin").join("pip")
    };
    let out = std::process::Command::new(&pip)
        .arg("install")
        .arg("-r")
        .arg(&req)
        .current_dir(&dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .with_context(|| format!("running {}", pip.display()))?;
    push_excerpt(&mut stdout, &out.stdout);
    push_excerpt(&mut stderr, &out.stderr);
    Ok(BootstrapResult {
        plugin_id: plugin_id.to_string(),
        status: if out.status.success() { "ok" } else { "error" }.to_string(),
        stdout_excerpt: stdout,
        stderr_excerpt: stderr,
    })
}

// ─── Helpers ─────────────────────────────────────────────────────

fn decode_base64(payload: &str) -> Result<Vec<u8>> {
    let stripped = if let Some(idx) = payload.find(";base64,") {
        &payload[idx + ";base64,".len()..]
    } else {
        payload
    };
    BASE64
        .decode(stripped.trim().as_bytes())
        .context("base64 decode failed")
}

fn mkstaging() -> Result<PathBuf> {
    let base = std::env::temp_dir().join(format!(
        "maltrace-plugin-stage-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&base)?;
    Ok(base)
}

/// Recursive copy of a directory tree. Skips `.venv/` and `__pycache__/`
/// so a freshly-extracted plugin doesn't drag stale build artefacts
/// from the source machine — the venv is recreated by `bootstrap_venv`
/// on the target.
fn copy_tree(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str == ".venv" || name_str == "__pycache__" || name_str == ".git" {
            continue;
        }
        let from = entry.path();
        let to = dst.join(&name);
        let kind = entry.file_type()?;
        if kind.is_dir() {
            copy_tree(&from, &to)?;
        } else if kind.is_file() {
            std::fs::copy(&from, &to)
                .with_context(|| format!("copying {} → {}", from.display(), to.display()))?;
        }
        // Skip symlinks and special files — plugins shouldn't ship them.
    }
    Ok(())
}

/// Tiny tar.gz extractor that handles the subset of formats produced by
/// modern `tar`/`gtar` — POSIX ustar with regular files + directories.
/// Refuses absolute paths and `..` traversal.
fn extract_tar_gz(bytes: &[u8], dest: &Path) -> Result<()> {
    use std::io::Read;
    let mut gz = flate2::read::GzDecoder::new(bytes);
    let mut decompressed = Vec::with_capacity(bytes.len() * 4);
    gz.read_to_end(&mut decompressed)
        .context("gunzipping tarball")?;
    let mut cursor = std::io::Cursor::new(decompressed);
    let mut archive = tar::Archive::new(&mut cursor);
    for entry in archive.entries().context("opening tar entries")? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        // Defend against path traversal.
        if path.is_absolute()
            || path
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err(anyhow!(
                "tar entry path escapes archive root: {}",
                path.display()
            ));
        }
        let dest_path = dest.join(&path);
        if let Some(parent) = dest_path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        // entry.unpack honours the entry header's permissions.
        entry
            .unpack(&dest_path)
            .with_context(|| format!("unpacking {}", path.display()))?;
    }
    Ok(())
}

fn push_excerpt(into: &mut String, bytes: &[u8]) {
    const MAX: usize = 4000;
    let s = String::from_utf8_lossy(bytes);
    if s.len() <= MAX {
        into.push_str(&s);
    } else {
        into.push_str(&s[..MAX]);
        into.push_str("\n…[truncated]");
    }
}
