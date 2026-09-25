//! Per-case file storage.
//!
//! When the user drags an image or file onto the canvas (or browses for
//! one), the bytes land under `<data_dir>/case-<case_id>/files/<uuid>.<ext>`
//! and a matching `Image` or `File` entity gets created. The entity's
//! `label` is the user-facing original filename; `entity.data` JSON
//! carries `{file_path, original_name, mime_type, size_bytes}`. The
//! `source_event_type` is `IMAGE` for image MIMEs so the face-search
//! sidecar treats it as a real `IMAGE` input automatically.
//!
//! Run-path integration: `core::run_transform` checks
//! `entity.data.file_path` first when invoking a sidecar on an Image/File
//! entity — the sidecar gets a real on-disk path, not the display label.
//!
//! Lifecycle: file is removed from disk when its entity is deleted (see
//! `core::delete_entity`). When a whole case is deleted, the cascade
//! removes the entity rows but the file directory is also wiped to keep
//! the on-disk store coherent.

use anyhow::{Context, Result};
use base64::engine::{general_purpose::STANDARD as BASE64, Engine as _};
use std::path::{Path, PathBuf};

/// Hard ceiling on a single uploaded file's size. Generous enough for
/// high-res photos and small PDFs, low enough that a misclick can't
/// fill the user's disk. Documented at the UI boundary too.
pub const MAX_UPLOAD_BYTES: usize = 32 * 1024 * 1024;

/// Cap on what `read_as_data_url` returns. Inspector previews don't need
/// the full 30 MB blob; the user can re-export the file if they want
/// pixel-perfect.
pub const MAX_PREVIEW_BYTES: usize = 8 * 1024 * 1024;

/// Where files for a single case live.
pub fn files_dir(data_dir: &Path, case_id: i64) -> PathBuf {
    data_dir.join(format!("case-{case_id}")).join("files")
}

/// Stored on `Entity.data` as JSON for both Image and File entities.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct FilePayload {
    pub file_path: String,
    pub original_name: String,
    pub mime_type: String,
    pub size_bytes: u64,
}

#[derive(Debug)]
pub struct StoredFile {
    pub path: PathBuf,
    pub mime_type: String,
    pub size_bytes: u64,
}

/// Write the bytes under `<files_dir>/<uuid>.<ext>` and return both the
/// resolved path and the inferred MIME type. The caller wraps this in an
/// entity row.
pub fn write_file(
    data_dir: &Path,
    case_id: i64,
    original_name: &str,
    bytes: &[u8],
) -> Result<StoredFile> {
    if bytes.is_empty() {
        anyhow::bail!("upload: empty file");
    }
    if bytes.len() > MAX_UPLOAD_BYTES {
        anyhow::bail!(
            "upload: file is {} bytes, exceeds {} byte ceiling",
            bytes.len(),
            MAX_UPLOAD_BYTES
        );
    }
    let dir = files_dir(data_dir, case_id);
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("creating {}", dir.display()))?;
    let ext = Path::new(original_name)
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase())
        // Strip anything weird; we already control the rest of the filename.
        .filter(|s| {
            !s.is_empty()
                && s.len() <= 8
                && s.chars().all(|c| c.is_ascii_alphanumeric())
        })
        .unwrap_or_else(|| "bin".to_string());
    let stem = uuid::Uuid::new_v4().simple().to_string();
    let path = dir.join(format!("{stem}.{ext}"));
    std::fs::write(&path, bytes)
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(StoredFile {
        path,
        mime_type: guess_mime(&ext),
        size_bytes: bytes.len() as u64,
    })
}

/// Read a file and return its RAW base64 (no `data:` prefix) — for the
/// Anthropic vision API, which wants the bare base64 + a separate
/// media_type. Same MAX_PREVIEW_BYTES ceiling as `read_as_data_url`.
pub fn read_as_base64(path: &Path) -> Result<String> {
    let meta = std::fs::metadata(path)
        .with_context(|| format!("stat {}", path.display()))?;
    if meta.len() as usize > MAX_PREVIEW_BYTES {
        anyhow::bail!(
            "image: {} is {} bytes, exceeds {} byte ceiling",
            path.display(),
            meta.len(),
            MAX_PREVIEW_BYTES
        );
    }
    let bytes = std::fs::read(path)
        .with_context(|| format!("reading {}", path.display()))?;
    Ok(BASE64.encode(&bytes))
}

/// Read a file from disk and return a `data:<mime>;base64,...` URL the
/// frontend can stuff into an `<img src>` or `<a href>`. Caps the read at
/// MAX_PREVIEW_BYTES.
pub fn read_as_data_url(path: &Path, mime_type: &str) -> Result<String> {
    let meta = std::fs::metadata(path)
        .with_context(|| format!("stat {}", path.display()))?;
    if meta.len() as usize > MAX_PREVIEW_BYTES {
        anyhow::bail!(
            "preview: {} is {} bytes, exceeds {} byte preview ceiling",
            path.display(),
            meta.len(),
            MAX_PREVIEW_BYTES
        );
    }
    let bytes = std::fs::read(path)
        .with_context(|| format!("reading {}", path.display()))?;
    let encoded = BASE64.encode(&bytes);
    Ok(format!("data:{};base64,{}", mime_type, encoded))
}

/// Decode a base64 payload sent over IPC/HTTP. Accepts both standard and
/// "data URL" encodings; we strip the `data:...;base64,` prefix if
/// present so the same call shape works from a browser FileReader and
/// from a CLI hex dump.
pub fn decode_base64(payload: &str) -> Result<Vec<u8>> {
    let stripped = if let Some(idx) = payload.find(";base64,") {
        &payload[idx + ";base64,".len()..]
    } else {
        payload
    };
    BASE64
        .decode(stripped.as_bytes())
        .context("base64 decode failed")
}

/// Best-effort delete: a missing path is fine (entity was never linked to
/// real bytes, or someone moved them by hand).
pub fn try_delete(path: &Path) {
    let _ = std::fs::remove_file(path);
}

/// Wipe a case's files directory wholesale. Called when a case is deleted.
pub fn try_wipe_case(data_dir: &Path, case_id: i64) {
    let dir = files_dir(data_dir, case_id);
    let _ = std::fs::remove_dir_all(dir);
}

/// Very compact extension → MIME table. We don't need full coverage —
/// SpiderFoot and face-search only care that images are tagged `image/*`.
/// Anything unknown falls back to `application/octet-stream`.
fn guess_mime(ext: &str) -> String {
    match ext {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "tif" | "tiff" => "image/tiff",
        "heic" => "image/heic",
        "pdf" => "application/pdf",
        "txt" | "log" => "text/plain",
        "csv" => "text/csv",
        "json" => "application/json",
        "html" | "htm" => "text/html",
        "xml" => "application/xml",
        "zip" => "application/zip",
        _ => "application/octet-stream",
    }
    .to_string()
}

/// Guess a file's MIME from its path extension (case-insensitive). Used when
/// an entity has no stored `FilePayload` (e.g. a manually-typed path).
pub fn mime_for_extension(path: &Path) -> String {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    guess_mime(&ext)
}

/// True when the MIME type starts with `image/`, so the caller can decide
/// to create an `Image` (vs. `File`) entity and tag it with `IMAGE` as
/// `source_event_type` for the face-search sidecar.
pub fn is_image(mime: &str) -> bool {
    mime.starts_with("image/")
}
