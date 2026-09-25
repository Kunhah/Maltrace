//! Case backup + restore as a single `.tar.gz` bundle.
//!
//! Investigator workflow: archive a case at the end of an
//! investigation, hand off a sealed bundle to legal / a colleague,
//! re-import on another machine. The bundle layout is JSON-first so
//! it's diffable and human-inspectable:
//!
//! ```text
//! manifest.json          { case, exported_at, schema_version, file_count }
//! entities.json          [ Entity, … ]   (with tags merged in)
//! edges.json             [ Edge, … ]
//! annotations.json       [ Annotation, … ]
//! transform_runs.json    [ TransformRun, … ]
//! audit_events.json      [ AuditEvent, … ]
//! files/<entity_id>/<original_name>  (any uploaded files attached to entities)
//! ```
//!
//! The whole thing is wrapped in a single gzip-tar so it travels as a
//! single file. Restore re-creates the case (with a fresh id) and all
//! its rows. IDs of restored entities are preserved (UUIDs are safe to
//! re-use across cases since they're globally unique).

use crate::core::AppState;
use crate::db::{Annotation, AuditEvent, Edge, Entity, TransformRun};
use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{Cursor, Read, Write};
use std::path::PathBuf;

pub const BACKUP_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
pub struct Manifest {
    pub schema_version: u32,
    pub case_name: String,
    pub exported_at: String,
    pub entity_count: usize,
    pub edge_count: usize,
    pub run_count: usize,
    pub file_count: usize,
}

#[derive(Debug, Serialize, Deserialize)]
struct EntityWithTags {
    #[serde(flatten)]
    entity: Entity,
    #[serde(default)]
    tags: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ExportResult {
    /// `.tar.gz` payload, base64-encoded so it crosses JSON / IPC
    /// boundaries safely.
    pub bytes_base64: String,
    pub manifest: Manifest,
}

#[derive(Debug, Serialize)]
pub struct RestoreResult {
    pub new_case_id: i64,
    pub new_case_name: String,
    pub entity_count: usize,
    pub edge_count: usize,
}

pub fn export_case(state: &AppState, case_id: i64) -> Result<ExportResult> {
    let (case, entities, edges, annotations, runs, audit_events, tags_by_entity) = {
        let db = state.db.lock();
        let case = db
            .get_case(case_id)?
            .ok_or_else(|| anyhow!("case {case_id} not found"))?;
        let entities = db.list_entities(case_id)?;
        let edges = db.list_edges(case_id)?;
        let annotations = db.list_annotations(case_id)?;
        let runs = db.list_transform_runs(case_id, 5000)?;
        let audit_events = db.list_audit_events(case_id, 5000)?;
        let tags = db.list_tags_for_case(case_id)?;
        (case, entities, edges, annotations, runs, audit_events, tags)
    };

    let entities_with_tags: Vec<EntityWithTags> = entities
        .iter()
        .map(|e| EntityWithTags {
            entity: e.clone(),
            tags: tags_by_entity.get(&e.id).cloned().unwrap_or_default(),
        })
        .collect();

    // Gather any on-disk attachments. Files live under
    // `<data_dir>/case-<case_id>/files/`; we copy them in by their
    // path-relative-to-the-case-files-dir so a clean tar layout falls
    // out.
    let case_files_dir = state.data_dir.join(format!("case-{case_id}/files"));
    let mut file_paths: Vec<PathBuf> = Vec::new();
    if case_files_dir.exists() {
        for entry in walkdir(&case_files_dir) {
            if entry.is_file() {
                file_paths.push(entry);
            }
        }
    }

    let manifest = Manifest {
        schema_version: BACKUP_SCHEMA_VERSION,
        case_name: case.name.clone(),
        exported_at: chrono::Utc::now().to_rfc3339(),
        entity_count: entities.len(),
        edge_count: edges.len(),
        run_count: runs.len(),
        file_count: file_paths.len(),
    };

    // Build the tarball in memory.
    let mut buf = Vec::with_capacity(4096 + entities.len() * 200);
    {
        let gz = flate2::write::GzEncoder::new(&mut buf, flate2::Compression::default());
        let mut tar = tar::Builder::new(gz);
        write_json(&mut tar, "manifest.json", &manifest)?;
        write_json(&mut tar, "entities.json", &entities_with_tags)?;
        write_json(&mut tar, "edges.json", &edges)?;
        write_json(&mut tar, "annotations.json", &annotations)?;
        write_json(&mut tar, "transform_runs.json", &runs)?;
        write_json(&mut tar, "audit_events.json", &audit_events)?;
        for f in &file_paths {
            let Ok(rel) = f.strip_prefix(&case_files_dir) else {
                continue;
            };
            let archive_path = PathBuf::from("files").join(rel);
            tar.append_path_with_name(f, archive_path).with_context(|| {
                format!("appending {} to case backup", f.display())
            })?;
        }
        tar.finish()?;
    }

    use base64::engine::{general_purpose::STANDARD as BASE64, Engine as _};
    Ok(ExportResult {
        bytes_base64: BASE64.encode(&buf),
        manifest,
    })
}

pub fn restore_case(state: &AppState, bytes_base64: &str) -> Result<RestoreResult> {
    use base64::engine::{general_purpose::STANDARD as BASE64, Engine as _};
    let payload = bytes_base64.trim();
    // Accept both bare base64 and a `data:...;base64,...` URL.
    let payload = if let Some(idx) = payload.find(";base64,") {
        &payload[idx + ";base64,".len()..]
    } else {
        payload
    };
    let raw = BASE64
        .decode(payload.as_bytes())
        .context("base64 decode of backup payload failed")?;

    let mut gz = flate2::read::GzDecoder::new(&raw[..]);
    let mut decompressed = Vec::with_capacity(raw.len() * 4);
    gz.read_to_end(&mut decompressed)
        .context("gunzip of backup payload failed")?;

    let mut archive = tar::Archive::new(Cursor::new(decompressed));
    // Two-pass: first read every JSON entry into memory; then create
    // the case + rows; then write files to the new case's data dir.
    let mut manifest: Option<Manifest> = None;
    let mut entities_with_tags: Vec<EntityWithTags> = Vec::new();
    let mut edges: Vec<Edge> = Vec::new();
    let mut annotations: Vec<Annotation> = Vec::new();
    let mut audit_events: Vec<AuditEvent> = Vec::new();
    let mut runs: Vec<TransformRun> = Vec::new();
    let mut files: HashMap<PathBuf, Vec<u8>> = HashMap::new();

    for entry in archive.entries().context("reading backup tar entries")? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        // Refuse traversal.
        if path.is_absolute()
            || path
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err(anyhow!("backup contains unsafe path: {}", path.display()));
        }
        let name = path.to_string_lossy().into_owned();
        if name == "manifest.json" {
            let mut s = String::new();
            entry.read_to_string(&mut s)?;
            manifest = Some(serde_json::from_str(&s).context("manifest.json")?);
        } else if name == "entities.json" {
            let mut s = String::new();
            entry.read_to_string(&mut s)?;
            entities_with_tags = serde_json::from_str(&s).context("entities.json")?;
        } else if name == "edges.json" {
            let mut s = String::new();
            entry.read_to_string(&mut s)?;
            edges = serde_json::from_str(&s).context("edges.json")?;
        } else if name == "annotations.json" {
            let mut s = String::new();
            entry.read_to_string(&mut s)?;
            annotations = serde_json::from_str(&s).context("annotations.json")?;
        } else if name == "transform_runs.json" {
            let mut s = String::new();
            entry.read_to_string(&mut s)?;
            runs = serde_json::from_str(&s).context("transform_runs.json")?;
        } else if name == "audit_events.json" {
            let mut s = String::new();
            entry.read_to_string(&mut s)?;
            audit_events = serde_json::from_str(&s).context("audit_events.json")?;
        } else if name.starts_with("files/") {
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes)?;
            let rel = PathBuf::from(name.strip_prefix("files/").unwrap_or(&name));
            files.insert(rel, bytes);
        }
    }

    let manifest = manifest.ok_or_else(|| anyhow!("backup is missing manifest.json"))?;
    if manifest.schema_version != BACKUP_SCHEMA_VERSION {
        return Err(anyhow!(
            "backup schema_version {} (current: {})",
            manifest.schema_version,
            BACKUP_SCHEMA_VERSION
        ));
    }

    let db = state.db.lock();
    // Use a label like "restored: <original_name>" so two restores of
    // the same archive don't collide.
    let new_name = format!("restored: {}", manifest.case_name);
    let new_case = db.create_case(&new_name)?;

    // Replay entities — keep original ids since UUIDs are unique. We
    // use `restore_entity` for the same byte-for-byte semantics as the
    // undo path. Also re-tag.
    for ewt in &entities_with_tags {
        let e = &ewt.entity;
        db.restore_entity(
            &e.id,
            new_case.id,
            &e.entity_type,
            e.source_event_type.as_deref(),
            &e.label,
            e.data.as_deref(),
            e.pos_x,
            e.pos_y,
            e.created_at,
            e.notes.as_deref(),
            e.bookmarked,
            e.locked,
            e.created_by_run_id,
            e.created_by_transform_id.as_deref(),
            e.source_url.as_deref(),
            e.created_by_user.as_deref(),
        )?;
        for t in &ewt.tags {
            let _ = db.add_tag(&e.id, t);
        }
    }

    // Replay edges.
    for e in &edges {
        db.restore_edge(
            &e.id,
            new_case.id,
            &e.source_id,
            &e.target_id,
            e.label.as_deref(),
            e.created_at,
            e.run_id,
        )?;
    }

    for a in &annotations {
        let _ = db.create_annotation(new_case.id, &a.text, a.pos_x, a.pos_y, &a.color);
    }

    // ─── Run-id remapping ────────────────────────────────────
    // transform_runs.id is AUTOINCREMENT, so we can't pin restored
    // rows to their original ids. Instead: insert each run with a
    // fresh id, build an old→new map, and patch every restored
    // entity's `created_by_run_id` + edge's `run_id` to use the new
    // id. End result: `entity_provenance` resolves cleanly in the
    // restored case.
    let mut run_id_map: std::collections::HashMap<i64, i64> =
        std::collections::HashMap::with_capacity(runs.len());
    for r in &runs {
        let new_id = db.restore_transform_run(
            new_case.id,
            &r.transform_id,
            r.source_entity_id.as_deref(),
            r.source_label.as_deref(),
            r.started_at,
            r.duration_ms,
            &r.status,
            r.new_entity_count,
            r.error_message.as_deref(),
            r.stderr_excerpt.as_deref(),
            r.created_by_user.as_deref(),
        )?;
        run_id_map.insert(r.id, new_id);
    }
    for (old_id, new_id) in &run_id_map {
        db.remap_entity_run_id(new_case.id, *old_id, *new_id)?;
    }

    // Audit events: re-record verbatim so the audit panel reads true
    // history. `actor` is preserved. Order-by-id matters less than
    // order-by-timestamp here — both are honoured by the column
    // values we replay.
    for a in &audit_events {
        db.record_audit_event(
            new_case.id,
            &a.kind,
            a.entity_id.as_deref(),
            a.payload.as_deref(),
            a.actor.as_deref(),
        );
    }

    // Restore files to the new case's files dir, preserving the
    // relative paths (which entities reference via
    // `entity.data.file_path` — but that path will be wrong on the new
    // machine).
    if !files.is_empty() {
        let new_files_dir = state.data_dir.join(format!("case-{}/files", new_case.id));
        std::fs::create_dir_all(&new_files_dir).ok();
        for (rel, bytes) in &files {
            let dst = new_files_dir.join(rel);
            if let Some(parent) = dst.parent() {
                std::fs::create_dir_all(parent).ok();
            }
            if let Err(e) = std::fs::write(&dst, bytes) {
                eprintln!("backup: failed to restore file {}: {e}", dst.display());
            }
        }
    }

    Ok(RestoreResult {
        new_case_id: new_case.id,
        new_case_name: new_case.name,
        entity_count: entities_with_tags.len(),
        edge_count: edges.len(),
    })
}

fn write_json<W: Write, T: Serialize>(
    tar: &mut tar::Builder<W>,
    path: &str,
    value: &T,
) -> Result<()> {
    let body = serde_json::to_vec_pretty(value).with_context(|| format!("serializing {path}"))?;
    let mut header = tar::Header::new_gnu();
    header.set_path(path)?;
    header.set_size(body.len() as u64);
    header.set_mode(0o644);
    header.set_mtime(chrono::Utc::now().timestamp() as u64);
    header.set_cksum();
    tar.append(&header, body.as_slice())?;
    Ok(())
}

/// Tiny inline file-walker so we don't need to pull a walkdir crate
/// for one usage. Yields every regular file under `root`.
fn walkdir(root: &std::path::Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let p = entry.path();
            let kind = match entry.file_type() {
                Ok(k) => k,
                Err(_) => continue,
            };
            if kind.is_dir() {
                stack.push(p);
            } else if kind.is_file() {
                out.push(p);
            }
        }
    }
    out
}
