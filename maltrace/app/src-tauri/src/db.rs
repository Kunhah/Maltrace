//! SQLite persistence for cases, entities, and edges.
//!
//! Schema is intentionally small for the walking skeleton — single default
//! case, entities and edges scoped to it. Schema is versioned via the `meta`
//! table; bump `SCHEMA_VERSION` and add a migration branch when changing it.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// Schema version. Bump when migrations are added.
const SCHEMA_VERSION: i32 = 13;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entity {
    pub id: String,
    pub case_id: i64,
    /// Canonical entity type (see `crate::ontology`), e.g. "Domain", "Email".
    pub entity_type: String,
    /// Original SpiderFoot event type when the entity came from a transform,
    /// e.g. "AFFILIATE_INTERNET_NAME". `None` for manually-added entities.
    pub source_event_type: Option<String>,
    pub label: String,
    /// Optional free-form JSON payload (full WHOIS text, image blob ref, etc.)
    pub data: Option<String>,
    pub pos_x: Option<f64>,
    pub pos_y: Option<f64>,
    pub created_at: DateTime<Utc>,
    /// Investigator notes attached to this entity. Free-form Markdown-ish
    /// text — surfaced in the inspector, persisted forever.
    #[serde(default)]
    pub notes: Option<String>,
    /// Bookmark flag — gives the user a "this matters" pin and a
    /// "show only bookmarked" filter.
    #[serde(default)]
    pub bookmarked: bool,
    /// Pinned-position flag. Locked nodes don't move during layouts and
    /// can't be dragged — exactly Cytoscape's `node.lock()` semantics.
    /// Lets the investigator hand-arrange a cluster and then run fcose
    /// over the rest without losing it.
    #[serde(default)]
    pub locked: bool,
    /// `transform_runs.id` of the run that produced this entity. `None`
    /// for manually-added entities, imported rows, and the sample-graph
    /// seed. Persisting this is what makes the per-entity provenance
    /// view (show me which run created this node) possible.
    #[serde(default)]
    pub created_by_run_id: Option<i64>,
    /// Denormalised copy of the transform id that produced this entity.
    /// Same value lives on `transform_runs.transform_id`; copied here so
    /// the inspector can name the transform without a join. `None` for
    /// manual entities.
    #[serde(default)]
    pub created_by_transform_id: Option<String>,
    /// Source citation: the URL the originating transform claims this
    /// entity comes from. Sidecars surface it under
    /// `ProducedEvent.extra.url`; brazuca uses it for every emitted
    /// resource. Surfaced as a clickable link in the provenance section.
    #[serde(default)]
    pub source_url: Option<String>,
    /// Authenticated user (server build) or `"desktop"` (Tauri build)
    /// that created this row. Stamped at insert time so the audit log
    /// can answer "who added X?" in multi-user mode. `None` only on
    /// pre-v10 rows that existed before the migration.
    #[serde(default)]
    pub created_by_user: Option<String>,
}

/// Free-floating canvas annotation. Like Maltego's sticky notes — text
/// blocks dropped *between* entities to label a cluster or capture a
/// hypothesis. Lives in its own table because semantically these
/// aren't entities: they don't participate in transforms, the
/// ontology, or any export shape; they're investigator commentary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Annotation {
    pub id: String,
    pub case_id: i64,
    pub text: String,
    pub pos_x: f64,
    pub pos_y: f64,
    /// Hex colour for the sticky-note background. Defaulted to a warm
    /// amber when the caller doesn't pick one.
    pub color: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    pub id: String,
    pub case_id: i64,
    pub source_id: String,
    pub target_id: String,
    /// Which transform produced this edge, e.g. "sfp_whois".
    pub label: Option<String>,
    pub created_at: DateTime<Utc>,
    /// `transform_runs.id` of the run that produced this edge. `None` for
    /// manually-created edges (e.g. the sample-graph seed). Pairs with
    /// the matching field on `Entity` so an investigator can trace any
    /// graph element back to a single audited invocation.
    #[serde(default)]
    pub run_id: Option<i64>,
    /// Optional typed relationship label (e.g. `owns`, `resolves_to`,
    /// `abuses_of`). Separate from `label` (the producer transform id):
    /// `label` answers "what made this edge?", `relationship_type`
    /// answers "what does this edge mean?". User-editable via the edge
    /// inspector; `None` for un-classified edges.
    #[serde(default)]
    pub relationship_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Case {
    pub id: i64,
    pub name: String,
    pub created_at: DateTime<Utc>,
    /// True when this case has been "filed" — write paths refuse to
    /// mutate it (no new entities, no transform runs, no notes/labels/
    /// positions). Reads stay open. Investigators flip this at the end
    /// of an investigation to freeze the evidence; flip it back off if
    /// they need to re-open the case.
    #[serde(default)]
    pub locked: bool,
    /// RFC-3339 timestamp of the most recent lock. `None` until first
    /// lock; preserved across unlock/relock cycles so the UI can show
    /// "locked since …".
    #[serde(default)]
    pub locked_at: Option<DateTime<Utc>>,
}

/// User-defined canonical entity type. Returned alongside the built-in
/// `ontology::ALL` so the frontend can show them in the AddEntityForm and
/// colour their nodes uniformly. No SpiderFoot event mapping — custom
/// types are conceptual containers; transforms targeting them won't find
/// anything until someone wires a sidecar.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomEntityType {
    pub id: String,
    pub label: String,
    /// One of `identity`, `asset`, `evidence`, `threat`, `generic` —
    /// validated at insert time. Drives the kind-group in the
    /// AddEntityForm dropdown and not much else.
    pub kind: String,
    /// Hex colour for the node. Defaulted to a slate tone client-side
    /// when the user doesn't pick one.
    pub color: String,
    pub created_at: DateTime<Utc>,
}

/// Audit record for a single transform invocation. Whether the run
/// succeeded or failed, one row lands here so the UI can show "what
/// happened, when, and why."
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransformRun {
    pub id: i64,
    pub case_id: i64,
    pub transform_id: String,
    /// `None` for batch / machine runs where one row aggregates many
    /// per-entity sub-runs — currently always set, kept Optional for
    /// future-proofing.
    pub source_entity_id: Option<String>,
    /// Source entity's label at time of run, captured so the log stays
    /// meaningful after the entity is deleted.
    pub source_label: Option<String>,
    pub started_at: DateTime<Utc>,
    pub duration_ms: i64,
    /// "ok" or "error".
    pub status: String,
    pub new_entity_count: i64,
    /// Free-form error message when `status == "error"`. None on success.
    pub error_message: Option<String>,
    /// Sidecar stderr captured during the run, truncated to a few KB.
    /// Useful for diagnosing rate-limits, missing API keys, exceptions.
    pub stderr_excerpt: Option<String>,
    /// Authenticated user (server build) or `"desktop"` (Tauri build)
    /// that triggered this run. `None` on pre-v10 rows + scheduler
    /// firings (which use `"scheduler"` going forward).
    #[serde(default)]
    pub created_by_user: Option<String>,
}

/// One result row of `search_entities_across_cases`. Carries enough
/// case-attribution that the frontend can render "found in case X" + a
/// jump-to button (switch case + select entity).
#[derive(Debug, Clone, Serialize)]
pub struct CrossCaseMatch {
    pub entity_id: String,
    pub case_id: i64,
    pub entity_type: String,
    pub label: String,
    pub bookmarked: bool,
    pub case_name: String,
}

/// Point-in-time copy of a case's entities + edges, captured by the
/// user (`Cmd+K → "Snapshot current case"`). Lets the investigator
/// compare "state on day 1" vs "state today" and see what's been
/// learned/lost. Distinct from `EntitySnapshot` (per-entity, used by
/// undo) and `transform_runs` / `audit_events` (action history, not
/// graph state).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaseSnapshotMeta {
    pub id: i64,
    pub case_id: i64,
    pub name: String,
    pub created_at: DateTime<Utc>,
    /// Entity count at snapshot time — surfaced in the list UI so the
    /// user can pick "the small one from yesterday" vs "the big one".
    pub entity_count: usize,
    pub edge_count: usize,
}

/// The compact entity / edge shape we persist inside each snapshot's
/// JSON blob. Stripped of position so a diff doesn't churn on the
/// user dragging nodes around, and of bookmarked/locked/notes since
/// those are workflow state, not investigative content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotEntity {
    pub id: String,
    pub entity_type: String,
    pub label: String,
    pub source_event_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotEdge {
    pub id: String,
    pub source_id: String,
    pub target_id: String,
    pub label: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct SnapshotPayload {
    pub entities: Vec<SnapshotEntity>,
    pub edges: Vec<SnapshotEdge>,
}

/// Wire-format result of `diff_case_snapshots`. Lists what's been added,
/// removed, or renamed between two snapshots (or between one snapshot
/// and the current case state). Entity-level changes only — edges
/// inherit from their endpoints.
#[derive(Debug, Serialize)]
pub struct CaseDiff {
    pub added_entities: Vec<SnapshotEntity>,
    pub removed_entities: Vec<SnapshotEntity>,
    /// Same id, different label (rename detected). `before` is the
    /// older state; `after` is the newer.
    pub renamed_entities: Vec<RenamedEntity>,
    pub added_edges: Vec<SnapshotEdge>,
    pub removed_edges: Vec<SnapshotEdge>,
}

#[derive(Debug, Serialize)]
pub struct RenamedEntity {
    pub id: String,
    pub entity_type: String,
    pub before: String,
    pub after: String,
}

/// Append-only record of investigator-significant mutations. Existing
/// `transform_runs` covers transform invocations; this covers everything
/// else — rename, delete, merge, lock toggle, bookmark, notes edits.
/// Together they form an evidence-integrity audit trail.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub id: i64,
    pub case_id: i64,
    /// Stable verb describing the action (e.g. `entity_renamed`,
    /// `entity_deleted`, `entity_merged`, `case_locked`).
    pub kind: String,
    /// When the action targets a specific entity. `None` for case-level
    /// events like `case_locked`.
    pub entity_id: Option<String>,
    /// JSON blob with action-specific detail (old/new label, drop_id on
    /// merges, etc.). Free-form — read by the AuditPanel only.
    pub payload: Option<String>,
    pub created_at: DateTime<Utc>,
    /// Authenticated user (server build) or `"desktop"` (Tauri build)
    /// that took the action. `None` only on pre-v10 rows.
    #[serde(default)]
    pub actor: Option<String>,
}

/// Operationally-significant event the user wants to see in the
/// notifications bell — scheduled-machine firings with new entities,
/// transform errors, plugin install results. Append-only; `read_at`
/// flips when the user acknowledges (bell-click or "mark all read").
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notification {
    pub id: i64,
    pub case_id: i64,
    pub kind: String,
    pub title: String,
    pub detail: Option<String>,
    /// Free-form JSON payload — kind-dependent.
    pub payload: Option<String>,
    pub created_at: DateTime<Utc>,
    pub read_at: Option<DateTime<Utc>>,
}

/// Persisted filter expression. Investigators reach for the same
/// view ("all unresolved domains with HIBP hits"; "everything tagged
/// follow-up") repeatedly within an investigation — the saved-search
/// row stores the predicate so they can recall it with one click.
///
/// Predicate is opaque JSON the frontend interprets. Treating it as a
/// blob keeps the schema small and lets the predicate grammar evolve
/// without migrations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedSearch {
    pub id: i64,
    pub case_id: i64,
    pub name: String,
    /// JSON-encoded `SearchPredicate`. Shape lives entirely on the
    /// frontend (see `types.ts::SearchPredicate`); the backend just
    /// round-trips the string.
    pub predicate: String,
    pub created_at: DateTime<Utc>,
}

/// Recurring-machine schedule. Wakes the background scheduler when
/// `next_run_at <= now`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MachineSchedule {
    pub id: i64,
    pub case_id: i64,
    pub machine_id: String,
    pub entity_id: String,
    /// Seed entity's label at creation time — used by the UI when the
    /// entity has been deleted but the row is still around.
    pub entity_label: String,
    /// Interval between runs. Stored in seconds for arithmetic
    /// simplicity; the UI exposes minutes/hours friendly inputs.
    pub interval_seconds: i64,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub next_run_at: DateTime<Utc>,
    pub last_run_at: Option<DateTime<Utc>>,
}

/// One audit row per scheduled-run firing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MachineScheduleRun {
    pub id: i64,
    pub schedule_id: i64,
    pub started_at: DateTime<Utc>,
    pub status: String,
    pub new_entity_count: i64,
    pub error_message: Option<String>,
}

/// Provenance attribution passed into `upsert_entity` / `insert_entity_force` /
/// `upsert_edge` when a graph element is created by a transform run.
/// Borrows the strings so the call site doesn't have to clone twice (run id
/// + transform id are stable for the duration of one persist loop).
#[derive(Debug, Clone, Copy)]
pub struct Provenance<'a> {
    pub run_id: i64,
    pub transform_id: &'a str,
    /// Source citation extracted from `ProducedEvent.extra.url` when
    /// present. Threaded through to the new entity so the inspector can
    /// render a clickable "source" link.
    pub source_url: Option<&'a str>,
}

pub struct Db {
    conn: Connection,
}

impl Db {
    pub fn open(path: &Path, passphrase: Option<&str>) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let conn = Connection::open(path).with_context(|| format!("opening {path:?}"))?;
        // SQLCipher: PRAGMA key must come before any other query. We
        // escape single-quotes so the SQL stays well-formed; the
        // built-in KDF derives the real key from the string itself.
        if let Some(pp) = passphrase.filter(|p| !p.is_empty()) {
            let escaped = pp.replace('\'', "''");
            conn.execute_batch(&format!("PRAGMA key = '{escaped}';"))
                .context("PRAGMA key failed — non-SQLCipher build or wrong passphrase")?;
        }
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA foreign_keys = ON;",
        )
        .context("initialising connection pragmas (passphrase mismatch?)")?;
        let db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    /// One-shot offline migration: turn the plaintext DB at `path` into a
    /// SQLCipher-encrypted DB at the same path. Atomic — the original
    /// stays intact if the export fails midway, only the final rename
    /// swaps it in. **Not safe while the app is running** — the live
    /// connection must be closed first (the dedicated CLI binary does
    /// this).
    pub fn encrypt_database_in_place(path: &Path, passphrase: &str) -> Result<()> {
        if !path.exists() {
            anyhow::bail!("source database {} does not exist", path.display());
        }
        if passphrase.is_empty() {
            anyhow::bail!("passphrase cannot be empty");
        }
        let tmp = path.with_extension("sqlite.encrypting");
        if tmp.exists() {
            std::fs::remove_file(&tmp).ok();
        }
        let escaped = passphrase.replace('\'', "''");
        let tmp_display = tmp.to_string_lossy().replace('\'', "''");
        let src = Connection::open(path)
            .with_context(|| format!("opening {} for export", path.display()))?;
        src.execute_batch(&format!(
            "ATTACH DATABASE '{tmp_display}' AS encrypted KEY '{escaped}';
             SELECT sqlcipher_export('encrypted');
             DETACH DATABASE encrypted;",
        ))
        .context("sqlcipher_export — is this binary linked against SQLCipher?")?;
        drop(src);
        std::fs::rename(&tmp, path)
            .with_context(|| format!("renaming {} → {}", tmp.display(), path.display()))?;
        Ok(())
    }

    fn migrate(&self) -> Result<()> {
        // 1. Tables that exist in every schema version.
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS cases (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL,
                created_at TEXT NOT NULL
             );",
        )?;

        let current: i32 = self
            .conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'schema_version'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);

        // 2. v0 → v1 (initial entities/edges, no source_event_type column).
        //    v1 → v2 (add source_event_type, canonical entity_type values).
        //    v2 → v3 (additive only: introduce `transform_runs`).
        //    v3 → v4 (additive: introduce `custom_entity_types`).
        //    v4 → v5 (additive: add `notes` + `bookmarked` columns to entities).
        //
        // For pre-v2 schemas we rebuild entities/edges; from v2 onward
        // migrations are additive (the CREATE TABLE IF NOT EXISTS in the
        // next block picks up the new table without dropping anything).
        if current < 2 {
            if current > 0 {
                eprintln!(
                    "maltrace: migrating SQLite schema {} → {}; entities/edges will be rebuilt (cases preserved).",
                    current, SCHEMA_VERSION,
                );
            }
            self.conn.execute_batch(
                "DROP TABLE IF EXISTS edges;
                 DROP TABLE IF EXISTS entities;",
            )?;
        }
        // v4 → v5: ALTER existing entities to add notes/bookmarked.
        // SQLite's `ADD COLUMN` is the safe way — defaults backfill
        // existing rows. We swallow the "duplicate column name" error so
        // a fresh DB (where the columns are already on the CREATE TABLE
        // below) doesn't fail boot.
        if current >= 2 && current < 5 {
            let _ = self
                .conn
                .execute_batch("ALTER TABLE entities ADD COLUMN notes TEXT;");
            let _ = self
                .conn
                .execute_batch("ALTER TABLE entities ADD COLUMN bookmarked INTEGER NOT NULL DEFAULT 0;");
        }
        // v5 → v6: pinning support on entities + free-floating
        // annotations. Same safe-ALTER pattern as above.
        if current >= 2 && current < 6 {
            let _ = self
                .conn
                .execute_batch("ALTER TABLE entities ADD COLUMN locked INTEGER NOT NULL DEFAULT 0;");
        }
        // v6 → v7: provenance + evidence-lock. Additive ALTERs so the
        // existing case data carries over unmodified — older rows get
        // NULL provenance (which the inspector reads as "manually
        // added"), and `cases.locked` defaults to 0 (open). Each ALTER
        // is run separately so partially-applied prior migrations don't
        // hit a "duplicate column" error and short-circuit the rest.
        if current >= 2 && current < 7 {
            let _ = self.conn.execute_batch(
                "ALTER TABLE entities ADD COLUMN created_by_run_id INTEGER;",
            );
            let _ = self.conn.execute_batch(
                "ALTER TABLE entities ADD COLUMN created_by_transform_id TEXT;",
            );
            let _ = self.conn.execute_batch(
                "ALTER TABLE entities ADD COLUMN source_url TEXT;",
            );
            let _ = self.conn.execute_batch(
                "ALTER TABLE edges ADD COLUMN run_id INTEGER;",
            );
            let _ = self.conn.execute_batch(
                "ALTER TABLE cases ADD COLUMN locked INTEGER NOT NULL DEFAULT 0;",
            );
            let _ = self
                .conn
                .execute_batch("ALTER TABLE cases ADD COLUMN locked_at TEXT;");
        }
        // v7 → v8: machine schedules + per-firing audit. Pure additive
        // tables — the CREATE TABLE IF NOT EXISTS below handles fresh
        // DBs; nothing to ALTER on the existing tables.
        // (No-op block; left for migration-cookbook clarity.)

        // v9 → v10: multi-user audit. Stamp `created_by_user` on
        // entities + transform_runs + audit_events so the audit log
        // can answer "who deleted X?" when the server runs with auth
        // enabled. Tauri single-user installs land "desktop" in every
        // row. Typed edges (`relationship_type`) become a first-class
        // edge column, separate from the producer `label`. Also adds
        // the `notifications` table that the scheduler + transform
        // runner write to.
        if current >= 2 && current < 10 {
            let _ = self
                .conn
                .execute_batch("ALTER TABLE entities ADD COLUMN created_by_user TEXT;");
            let _ = self.conn.execute_batch(
                "ALTER TABLE transform_runs ADD COLUMN created_by_user TEXT;",
            );
            let _ = self
                .conn
                .execute_batch("ALTER TABLE audit_events ADD COLUMN actor TEXT;");
            let _ = self
                .conn
                .execute_batch("ALTER TABLE edges ADD COLUMN relationship_type TEXT;");
        }

        // 3. Current-schema tables.
        //
        // The `cases` table created in step 1 is intentionally minimal so the
        // very-first-run path doesn't need lock columns. v6→v7 ALTERs add
        // `locked` + `locked_at` to existing DBs; fresh DBs land here, where
        // we apply the same columns as an idempotent ALTER (silently skipped
        // when they already exist).
        let _ = self
            .conn
            .execute_batch("ALTER TABLE cases ADD COLUMN locked INTEGER NOT NULL DEFAULT 0;");
        let _ = self
            .conn
            .execute_batch("ALTER TABLE cases ADD COLUMN locked_at TEXT;");

        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS entities (
                id TEXT PRIMARY KEY,
                case_id INTEGER NOT NULL REFERENCES cases(id) ON DELETE CASCADE,
                entity_type TEXT NOT NULL,
                source_event_type TEXT,
                label TEXT NOT NULL,
                data TEXT,
                pos_x REAL,
                pos_y REAL,
                created_at TEXT NOT NULL,
                notes TEXT,
                bookmarked INTEGER NOT NULL DEFAULT 0,
                locked INTEGER NOT NULL DEFAULT 0,
                created_by_run_id INTEGER,
                created_by_transform_id TEXT,
                source_url TEXT,
                created_by_user TEXT
             );
             CREATE INDEX IF NOT EXISTS idx_entities_case ON entities(case_id);
             CREATE INDEX IF NOT EXISTS idx_entities_run ON entities(created_by_run_id);
             CREATE UNIQUE INDEX IF NOT EXISTS uniq_entity_per_case
                ON entities(case_id, entity_type, label);
             CREATE TABLE IF NOT EXISTS edges (
                id TEXT PRIMARY KEY,
                case_id INTEGER NOT NULL REFERENCES cases(id) ON DELETE CASCADE,
                source_id TEXT NOT NULL REFERENCES entities(id) ON DELETE CASCADE,
                target_id TEXT NOT NULL REFERENCES entities(id) ON DELETE CASCADE,
                label TEXT,
                created_at TEXT NOT NULL,
                run_id INTEGER,
                relationship_type TEXT
             );
             CREATE INDEX IF NOT EXISTS idx_edges_case ON edges(case_id);
             CREATE UNIQUE INDEX IF NOT EXISTS uniq_edge
                ON edges(case_id, source_id, target_id, label);
             CREATE TABLE IF NOT EXISTS transform_runs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                case_id INTEGER NOT NULL REFERENCES cases(id) ON DELETE CASCADE,
                transform_id TEXT NOT NULL,
                source_entity_id TEXT,
                source_label TEXT,
                started_at TEXT NOT NULL,
                duration_ms INTEGER NOT NULL,
                status TEXT NOT NULL,
                new_entity_count INTEGER NOT NULL,
                error_message TEXT,
                stderr_excerpt TEXT,
                created_by_user TEXT
             );
             CREATE INDEX IF NOT EXISTS idx_runs_case_started
                ON transform_runs(case_id, started_at DESC);
             CREATE TABLE IF NOT EXISTS custom_entity_types (
                id TEXT PRIMARY KEY,
                label TEXT NOT NULL,
                kind TEXT NOT NULL,
                color TEXT NOT NULL,
                created_at TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS annotations (
                id TEXT PRIMARY KEY,
                case_id INTEGER NOT NULL REFERENCES cases(id) ON DELETE CASCADE,
                text TEXT NOT NULL,
                pos_x REAL NOT NULL,
                pos_y REAL NOT NULL,
                color TEXT NOT NULL,
                created_at TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_annotations_case
                ON annotations(case_id);
             CREATE TABLE IF NOT EXISTS audit_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                case_id INTEGER NOT NULL REFERENCES cases(id) ON DELETE CASCADE,
                kind TEXT NOT NULL,
                entity_id TEXT,
                payload TEXT,
                created_at TEXT NOT NULL,
                actor TEXT
             );
             CREATE INDEX IF NOT EXISTS idx_audit_case_created
                ON audit_events(case_id, created_at DESC);
             CREATE TABLE IF NOT EXISTS machine_schedules (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                case_id INTEGER NOT NULL REFERENCES cases(id) ON DELETE CASCADE,
                machine_id TEXT NOT NULL,
                entity_id TEXT NOT NULL REFERENCES entities(id) ON DELETE CASCADE,
                entity_label TEXT NOT NULL,
                interval_seconds INTEGER NOT NULL,
                enabled INTEGER NOT NULL DEFAULT 1,
                created_at TEXT NOT NULL,
                next_run_at TEXT NOT NULL,
                last_run_at TEXT
             );
             CREATE INDEX IF NOT EXISTS idx_schedules_due
                ON machine_schedules(enabled, next_run_at);
             CREATE TABLE IF NOT EXISTS machine_schedule_runs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                schedule_id INTEGER NOT NULL REFERENCES machine_schedules(id) ON DELETE CASCADE,
                started_at TEXT NOT NULL,
                status TEXT NOT NULL,
                new_entity_count INTEGER NOT NULL,
                error_message TEXT
             );
             CREATE INDEX IF NOT EXISTS idx_schedule_runs_sched
                ON machine_schedule_runs(schedule_id, started_at DESC);
             CREATE TABLE IF NOT EXISTS entity_tags (
                entity_id TEXT NOT NULL REFERENCES entities(id) ON DELETE CASCADE,
                tag TEXT NOT NULL,
                PRIMARY KEY (entity_id, tag)
             );
             CREATE INDEX IF NOT EXISTS idx_entity_tags_tag
                ON entity_tags(tag);
             CREATE INDEX IF NOT EXISTS idx_entity_tags_entity
                ON entity_tags(entity_id);
             CREATE TABLE IF NOT EXISTS notifications (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                case_id INTEGER NOT NULL REFERENCES cases(id) ON DELETE CASCADE,
                kind TEXT NOT NULL,
                title TEXT NOT NULL,
                detail TEXT,
                payload TEXT,
                created_at TEXT NOT NULL,
                read_at TEXT
             );
             CREATE INDEX IF NOT EXISTS idx_notifications_case_created
                ON notifications(case_id, created_at DESC);
             CREATE INDEX IF NOT EXISTS idx_notifications_unread
                ON notifications(case_id, read_at);
             CREATE TABLE IF NOT EXISTS saved_searches (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                case_id INTEGER NOT NULL REFERENCES cases(id) ON DELETE CASCADE,
                name TEXT NOT NULL,
                predicate TEXT NOT NULL,
                created_at TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_saved_searches_case
                ON saved_searches(case_id, name);
             CREATE TABLE IF NOT EXISTS case_snapshots (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                case_id INTEGER NOT NULL REFERENCES cases(id) ON DELETE CASCADE,
                name TEXT NOT NULL,
                created_at TEXT NOT NULL,
                /* JSON: { entities: [...minimal entity rows...],
                            edges: [...minimal edge rows...] }
                   stored as a blob so the diff path doesn't fan out
                   into a snapshot_entities / snapshot_edges normalised
                   schema. Walking-skeleton scope; if snapshots grow
                   beyond ~10k entities we'd normalise. */
                payload TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_case_snapshots_case
                ON case_snapshots(case_id, created_at DESC);",
        )?;

        // FTS5 index over entity labels — gives cross-case search a
        // proper inverted index instead of `LIKE %q%` table scans. The
        // virtual-table CREATE can fail on SQLite builds without FTS5
        // (unusual but possible); on failure we log and keep the LIKE
        // fallback. Triggers keep the index in sync with the entities
        // table; on first migration we backfill in one INSERT.
        let fts_setup = self.conn.execute_batch(
            "CREATE VIRTUAL TABLE IF NOT EXISTS entities_fts USING fts5(
                entity_id UNINDEXED,
                label,
                tokenize = 'unicode61 remove_diacritics 1'
             );
             CREATE TRIGGER IF NOT EXISTS entities_fts_ai
                AFTER INSERT ON entities BEGIN
                  INSERT INTO entities_fts(entity_id, label) VALUES (new.id, new.label);
             END;
             CREATE TRIGGER IF NOT EXISTS entities_fts_ad
                AFTER DELETE ON entities BEGIN
                  DELETE FROM entities_fts WHERE entity_id = old.id;
             END;
             CREATE TRIGGER IF NOT EXISTS entities_fts_au
                AFTER UPDATE OF label ON entities BEGIN
                  UPDATE entities_fts SET label = new.label WHERE entity_id = old.id;
             END;",
        );
        if let Err(e) = fts_setup {
            eprintln!(
                "maltrace: FTS5 unavailable ({e}); cross-case search will fall back to LIKE scans"
            );
        } else if current < 12 {
            // Backfill on the schema 11 → 12 bump. The IF NOT EXISTS
            // above means re-runs of CREATE-VIRTUAL-TABLE are no-ops,
            // but the entries table is empty on a fresh DB anyway —
            // this only matters when an existing DB is upgrading.
            let _ = self.conn.execute_batch(
                "INSERT INTO entities_fts(entity_id, label)
                 SELECT id, label FROM entities
                 WHERE id NOT IN (SELECT entity_id FROM entities_fts);",
            );
        }

        self.conn.execute(
            "INSERT OR REPLACE INTO meta(key, value) VALUES ('schema_version', ?1)",
            params![SCHEMA_VERSION.to_string()],
        )?;
        Ok(())
    }

    /// True when this connection has a working `entities_fts` virtual
    /// table — i.e. SQLite was built with FTS5 and CREATE succeeded.
    /// Cached on first call so we don't probe per query.
    fn has_fts5(&self) -> bool {
        self.conn
            .query_row("SELECT 1 FROM entities_fts LIMIT 1", [], |_| Ok(()))
            .is_ok()
            || self
                .conn
                .query_row("SELECT count(*) FROM entities_fts", [], |r| {
                    r.get::<_, i64>(0)
                })
                .is_ok()
    }

    pub fn list_cases(&self) -> Result<Vec<Case>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, created_at, locked, locked_at FROM cases ORDER BY id ASC",
        )?;
        let rows = stmt
            .query_map([], Self::row_to_case)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn create_case(&self, name: &str) -> Result<Case> {
        let now = Utc::now();
        let trimmed = name.trim();
        let final_name = if trimmed.is_empty() {
            "Untitled investigation"
        } else {
            trimmed
        };
        self.conn.execute(
            "INSERT INTO cases(name, created_at) VALUES (?1, ?2)",
            params![final_name, now.to_rfc3339()],
        )?;
        Ok(Case {
            id: self.conn.last_insert_rowid(),
            name: final_name.to_string(),
            created_at: now,
            locked: false,
            locked_at: None,
        })
    }

    /// Look up a case by id. Used by the locked-case write guards so a
    /// mutator can short-circuit on a locked case without a `list_cases`
    /// scan.
    pub fn get_case(&self, id: i64) -> Result<Option<Case>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, name, created_at, locked, locked_at FROM cases WHERE id = ?1",
                params![id],
                Self::row_to_case,
            )
            .optional()?)
    }

    /// Toggle a case's evidence-lock. `locked_at` is updated only when
    /// locking — keeping the original lock timestamp lets the UI show
    /// "locked since …" even across an unlock/relock cycle (when the
    /// caller really wants a fresh stamp, they unlock then relock).
    pub fn set_case_locked(&self, id: i64, locked: bool) -> Result<()> {
        let now = Utc::now();
        if locked {
            self.conn.execute(
                "UPDATE cases SET locked = 1, locked_at = ?2 WHERE id = ?1",
                params![id, now.to_rfc3339()],
            )?;
        } else {
            self.conn.execute(
                "UPDATE cases SET locked = 0 WHERE id = ?1",
                params![id],
            )?;
        }
        Ok(())
    }

    /// Fast path used by every mutator: refuses if the case is locked.
    /// Reads stay open, so list/get methods never call this.
    pub fn ensure_case_writable(&self, case_id: i64) -> Result<()> {
        let locked: Option<i64> = self
            .conn
            .query_row(
                "SELECT locked FROM cases WHERE id = ?1",
                params![case_id],
                |row| row.get(0),
            )
            .optional()?;
        if locked.unwrap_or(0) != 0 {
            anyhow::bail!(
                "case {case_id} is locked (evidence snapshot) — unlock it to make changes"
            );
        }
        Ok(())
    }

    /// Variant for mutators that only know an entity id (most do).
    pub fn ensure_entity_writable(&self, entity_id: &str) -> Result<()> {
        let case_id: Option<i64> = self
            .conn
            .query_row(
                "SELECT case_id FROM entities WHERE id = ?1",
                params![entity_id],
                |row| row.get(0),
            )
            .optional()?;
        match case_id {
            Some(cid) => self.ensure_case_writable(cid),
            // Entity already gone — the caller will hit "not found" anyway.
            None => Ok(()),
        }
    }

    fn row_to_case(row: &rusqlite::Row<'_>) -> rusqlite::Result<Case> {
        let created_at: String = row.get(2)?;
        let locked_int: i64 = row.get::<_, i64>(3).unwrap_or(0);
        let locked_at_str: Option<String> = row.get(4).ok();
        Ok(Case {
            id: row.get(0)?,
            name: row.get(1)?,
            created_at: created_at.parse().unwrap_or_else(|_| Utc::now()),
            locked: locked_int != 0,
            locked_at: locked_at_str.and_then(|s| s.parse().ok()),
        })
    }

    pub fn rename_case(&self, id: i64, name: &str) -> Result<()> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err(anyhow::anyhow!("case name cannot be empty"));
        }
        self.conn.execute(
            "UPDATE cases SET name = ?2 WHERE id = ?1",
            params![id, trimmed],
        )?;
        Ok(())
    }

    /// Delete a case and all its entities/edges (foreign-key cascade).
    /// Refuses to delete the last remaining case so the UI always has
    /// somewhere to land.
    pub fn delete_case(&self, id: i64) -> Result<()> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM cases",
            [],
            |r| r.get(0),
        )?;
        if count <= 1 {
            return Err(anyhow::anyhow!(
                "cannot delete the last remaining case — create another first"
            ));
        }
        self.conn
            .execute("DELETE FROM cases WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// Get the user's default case, creating one if no cases exist yet.
    pub fn default_case(&self) -> Result<Case> {
        let existing: Option<Case> = self
            .conn
            .query_row(
                "SELECT id, name, created_at, locked, locked_at FROM cases ORDER BY id ASC LIMIT 1",
                [],
                Self::row_to_case,
            )
            .optional()?;
        if let Some(c) = existing {
            return Ok(c);
        }
        let now = Utc::now();
        self.conn.execute(
            "INSERT INTO cases(name, created_at) VALUES (?1, ?2)",
            params!["Untitled investigation", now.to_rfc3339()],
        )?;
        Ok(Case {
            id: self.conn.last_insert_rowid(),
            name: "Untitled investigation".into(),
            created_at: now,
            locked: false,
            locked_at: None,
        })
    }

    /// Insert an entity, or return the existing one if (case, canonical_type,
    /// label) already exists. When the existing row has no
    /// `source_event_type` and the caller supplies one, the row is updated —
    /// this lets a manual `Domain` entity get its source labeled if it
    /// later shows up as a transform output.
    ///
    /// `provenance` carries the transform-run attribution. `None` for
    /// manual / imported / seed entities; `Some` when the row came out
    /// of a sidecar transform run.
    pub fn upsert_entity(
        &self,
        case_id: i64,
        entity_type: &str,
        source_event_type: Option<&str>,
        label: &str,
        data: Option<&str>,
        pos: Option<(f64, f64)>,
        provenance: Option<&Provenance<'_>>,
        actor: Option<&str>,
    ) -> Result<Entity> {
        if let Some(existing) = self.find_entity(case_id, entity_type, label)? {
            // Same row already on the graph from a previous (probably
            // manual) add: take this chance to fill in details we now
            // know — the SF event type, and the run-provenance if we
            // have one. Never overwrite existing provenance though, so
            // re-running the same transform doesn't reattribute an
            // entity an earlier run already claimed.
            let mut needs_patch = false;
            let next_source = if existing.source_event_type.is_none() && source_event_type.is_some()
            {
                needs_patch = true;
                source_event_type
            } else {
                existing.source_event_type.as_deref()
            };
            let (next_run_id, next_transform_id, next_source_url) =
                if existing.created_by_run_id.is_none() {
                    if let Some(p) = provenance {
                        needs_patch = true;
                        (Some(p.run_id), Some(p.transform_id), p.source_url)
                    } else {
                        (None, None, None)
                    }
                } else {
                    (
                        existing.created_by_run_id,
                        existing.created_by_transform_id.as_deref(),
                        existing.source_url.as_deref(),
                    )
                };
            if needs_patch {
                self.conn.execute(
                    "UPDATE entities
                       SET source_event_type = COALESCE(?2, source_event_type),
                           created_by_run_id = COALESCE(?3, created_by_run_id),
                           created_by_transform_id = COALESCE(?4, created_by_transform_id),
                           source_url = COALESCE(?5, source_url)
                     WHERE id = ?1",
                    params![
                        existing.id,
                        next_source,
                        next_run_id,
                        next_transform_id,
                        next_source_url,
                    ],
                )?;
                return self
                    .get_entity(&existing.id)?
                    .ok_or_else(|| anyhow::anyhow!("entity vanished after update"));
            }
            return Ok(existing);
        }
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now();
        let (run_id, tid, surl) = match provenance {
            Some(p) => (Some(p.run_id), Some(p.transform_id), p.source_url),
            None => (None, None, None),
        };
        self.conn.execute(
            "INSERT INTO entities(id, case_id, entity_type, source_event_type, label, data, pos_x, pos_y, created_at, created_by_run_id, created_by_transform_id, source_url, created_by_user)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                id,
                case_id,
                entity_type,
                source_event_type,
                label,
                data,
                pos.map(|p| p.0),
                pos.map(|p| p.1),
                now.to_rfc3339(),
                run_id,
                tid,
                surl,
                actor,
            ],
        )?;
        Ok(Entity {
            id,
            case_id,
            entity_type: entity_type.into(),
            source_event_type: source_event_type.map(String::from),
            label: label.into(),
            data: data.map(String::from),
            pos_x: pos.map(|p| p.0),
            pos_y: pos.map(|p| p.1),
            created_at: now,
            notes: None,
            bookmarked: false,
            locked: false,
            created_by_run_id: run_id,
            created_by_transform_id: tid.map(String::from),
            source_url: surl.map(String::from),
            created_by_user: actor.map(String::from),
        })
    }

    /// Insert an entity without any upsert / dedup behaviour, even if a row
    /// with the same `(case_id, entity_type, label)` already exists. Used
    /// for Collection entities, which represent a *batch* and so should
    /// stay distinct across re-runs of the same transform; the caller is
    /// expected to make `label` distinguishing (e.g. by appending a short
    /// hash).
    pub fn insert_entity_force(
        &self,
        case_id: i64,
        entity_type: &str,
        source_event_type: Option<&str>,
        label: &str,
        data: Option<&str>,
        pos: Option<(f64, f64)>,
        provenance: Option<&Provenance<'_>>,
        actor: Option<&str>,
    ) -> Result<Entity> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now();
        let (run_id, tid, surl) = match provenance {
            Some(p) => (Some(p.run_id), Some(p.transform_id), p.source_url),
            None => (None, None, None),
        };
        self.conn.execute(
            "INSERT INTO entities(id, case_id, entity_type, source_event_type, label, data, pos_x, pos_y, created_at, created_by_run_id, created_by_transform_id, source_url, created_by_user)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                id,
                case_id,
                entity_type,
                source_event_type,
                label,
                data,
                pos.map(|p| p.0),
                pos.map(|p| p.1),
                now.to_rfc3339(),
                run_id,
                tid,
                surl,
                actor,
            ],
        )?;
        Ok(Entity {
            id,
            case_id,
            entity_type: entity_type.into(),
            source_event_type: source_event_type.map(String::from),
            label: label.into(),
            data: data.map(String::from),
            pos_x: pos.map(|p| p.0),
            pos_y: pos.map(|p| p.1),
            created_at: now,
            notes: None,
            bookmarked: false,
            locked: false,
            created_by_run_id: run_id,
            created_by_transform_id: tid.map(String::from),
            source_url: surl.map(String::from),
            created_by_user: actor.map(String::from),
        })
    }

    pub fn find_entity(
        &self,
        case_id: i64,
        entity_type: &str,
        label: &str,
    ) -> Result<Option<Entity>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, case_id, entity_type, source_event_type, label, data, pos_x, pos_y, created_at, notes, bookmarked, locked, created_by_run_id, created_by_transform_id, source_url, created_by_user
                 FROM entities WHERE case_id = ?1 AND entity_type = ?2 AND label = ?3",
                params![case_id, entity_type, label],
                Self::row_to_entity,
            )
            .optional()?)
    }

    pub fn get_entity(&self, id: &str) -> Result<Option<Entity>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, case_id, entity_type, source_event_type, label, data, pos_x, pos_y, created_at, notes, bookmarked, locked, created_by_run_id, created_by_transform_id, source_url, created_by_user
                 FROM entities WHERE id = ?1",
                params![id],
                Self::row_to_entity,
            )
            .optional()?)
    }

    pub fn list_entities(&self, case_id: i64) -> Result<Vec<Entity>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, case_id, entity_type, source_event_type, label, data, pos_x, pos_y, created_at, notes, bookmarked, locked, created_by_run_id, created_by_transform_id, source_url, created_by_user
             FROM entities WHERE case_id = ?1 ORDER BY created_at ASC",
        )?;
        let rows = stmt
            .query_map(params![case_id], Self::row_to_entity)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Free-form investigator notes attached to an entity.
    pub fn update_entity_notes(&self, id: &str, notes: Option<&str>) -> Result<()> {
        self.conn.execute(
            "UPDATE entities SET notes = ?2 WHERE id = ?1",
            params![id, notes],
        )?;
        Ok(())
    }

    pub fn update_entity_bookmark(&self, id: &str, bookmarked: bool) -> Result<()> {
        self.conn.execute(
            "UPDATE entities SET bookmarked = ?2 WHERE id = ?1",
            params![id, bookmarked as i64],
        )?;
        Ok(())
    }

    pub fn update_entity_locked(&self, id: &str, locked: bool) -> Result<()> {
        self.conn.execute(
            "UPDATE entities SET locked = ?2 WHERE id = ?1",
            params![id, locked as i64],
        )?;
        Ok(())
    }

    /// List every entity currently bookmarked, across the whole case.
    /// Used by the BookmarksPanel side surface.
    pub fn list_bookmarked(&self, case_id: i64) -> Result<Vec<Entity>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, case_id, entity_type, source_event_type, label, data, pos_x, pos_y, created_at, notes, bookmarked, locked, created_by_run_id, created_by_transform_id, source_url, created_by_user
             FROM entities
             WHERE case_id = ?1 AND bookmarked = 1
             ORDER BY label ASC",
        )?;
        let rows = stmt
            .query_map(params![case_id], Self::row_to_entity)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Filter transform_runs to those involving a specific entity. Used
    /// by the Inspector's per-entity history section.
    /// Substring search across every case's entities. Bookmarked rows
    /// rank first, then by recency. `limit` is capped at 200 to keep the
    /// UI snappy.
    pub fn search_entities_across_cases(
        &self,
        query: &str,
        limit: i64,
    ) -> Result<Vec<CrossCaseMatch>> {
        let q = query.trim();
        if q.is_empty() {
            return Ok(Vec::new());
        }
        let lim = limit.clamp(1, 200);
        // Try the FTS5 index first — proper inverted-index lookup,
        // ranked by `bm25`, much faster than `LIKE` on big DBs. Fall
        // back to `LIKE` when the index isn't available (older
        // migrations / no-FTS5 build) or the query can't be parsed as
        // an FTS5 expression (e.g. it starts with an operator char).
        if self.has_fts5() {
            // Build a "every word as prefix" FTS5 query so users typing
            // partial labels still match. Tokens get quoted to neutralise
            // FTS5 syntax metacharacters; trailing `*` enables prefix.
            let fts_query: String = q
                .split_whitespace()
                .filter(|w| !w.is_empty())
                .map(|w| {
                    let escaped = w.replace('"', "\"\"");
                    format!("\"{escaped}\"*")
                })
                .collect::<Vec<_>>()
                .join(" ");
            if !fts_query.is_empty() {
                let mut stmt = self.conn.prepare(
                    "SELECT e.id, e.case_id, e.entity_type, e.label, e.bookmarked, c.name
                     FROM entities_fts f
                     JOIN entities e ON e.id = f.entity_id
                     JOIN cases c ON c.id = e.case_id
                     WHERE entities_fts MATCH ?1
                     ORDER BY e.bookmarked DESC, bm25(entities_fts), e.created_at DESC
                     LIMIT ?2",
                )?;
                let rows = stmt
                    .query_map(params![fts_query, lim], Self::row_to_cross_case)?
                    .collect::<Result<Vec<_>, _>>()
                    .ok();
                if let Some(r) = rows {
                    return Ok(r);
                }
                // Fall through to LIKE on a malformed FTS query.
            }
        }
        // LIKE fallback — case-insensitive on ASCII (good enough for
        // domains/IPs/emails); slow at scale but always works.
        let pattern = format!("%{q}%");
        let mut stmt = self.conn.prepare(
            "SELECT e.id, e.case_id, e.entity_type, e.label, e.bookmarked, c.name
             FROM entities e
             JOIN cases c ON c.id = e.case_id
             WHERE e.label LIKE ?1
             ORDER BY e.bookmarked DESC, e.created_at DESC
             LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![pattern, lim], Self::row_to_cross_case)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    // ─── Case snapshots ─────────────────────────────────────
    /// Capture the case's current entities + edges as a single JSON
    /// payload row. Returns the snapshot metadata (without the full
    /// payload — the list / diff endpoints fetch payloads on demand).
    pub fn create_case_snapshot(
        &self,
        case_id: i64,
        name: &str,
    ) -> Result<CaseSnapshotMeta> {
        let name = name.trim();
        if name.is_empty() {
            anyhow::bail!("snapshot name cannot be empty");
        }
        let now = Utc::now();
        let entities = self.list_entities(case_id)?;
        let edges = self.list_edges(case_id)?;
        let payload = SnapshotPayload {
            entities: entities
                .iter()
                .map(|e| SnapshotEntity {
                    id: e.id.clone(),
                    entity_type: e.entity_type.clone(),
                    label: e.label.clone(),
                    source_event_type: e.source_event_type.clone(),
                })
                .collect(),
            edges: edges
                .iter()
                .map(|edge| SnapshotEdge {
                    id: edge.id.clone(),
                    source_id: edge.source_id.clone(),
                    target_id: edge.target_id.clone(),
                    label: edge.label.clone(),
                })
                .collect(),
        };
        let json = serde_json::to_string(&payload)?;
        self.conn.execute(
            "INSERT INTO case_snapshots(case_id, name, created_at, payload)
             VALUES (?1, ?2, ?3, ?4)",
            params![case_id, name, now.to_rfc3339(), json],
        )?;
        Ok(CaseSnapshotMeta {
            id: self.conn.last_insert_rowid(),
            case_id,
            name: name.to_string(),
            created_at: now,
            entity_count: payload.entities.len(),
            edge_count: payload.edges.len(),
        })
    }

    pub fn list_case_snapshots(&self, case_id: i64) -> Result<Vec<CaseSnapshotMeta>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, case_id, name, created_at, payload
             FROM case_snapshots
             WHERE case_id = ?1
             ORDER BY created_at DESC",
        )?;
        let rows = stmt
            .query_map(params![case_id], |row| {
                let created_at: String = row.get(3)?;
                let payload_str: String = row.get(4)?;
                let payload: SnapshotPayload =
                    serde_json::from_str(&payload_str).unwrap_or_default();
                Ok(CaseSnapshotMeta {
                    id: row.get(0)?,
                    case_id: row.get(1)?,
                    name: row.get(2)?,
                    created_at: created_at.parse().unwrap_or_else(|_| Utc::now()),
                    entity_count: payload.entities.len(),
                    edge_count: payload.edges.len(),
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn delete_case_snapshot(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM case_snapshots WHERE id = ?1", params![id])?;
        Ok(())
    }

    fn load_snapshot_payload(&self, id: i64) -> Result<SnapshotPayload> {
        let payload_str: String = self.conn.query_row(
            "SELECT payload FROM case_snapshots WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )?;
        Ok(serde_json::from_str(&payload_str)?)
    }

    /// Diff two snapshots, or one snapshot against the case's current
    /// state when `other_id` is None. Renames are detected by id+type
    /// match with a different label.
    pub fn diff_case_snapshots(
        &self,
        snapshot_id: i64,
        other_id: Option<i64>,
    ) -> Result<CaseDiff> {
        let older = self.load_snapshot_payload(snapshot_id)?;
        let newer = match other_id {
            Some(id) => self.load_snapshot_payload(id)?,
            None => {
                // Live case state. Pull case_id from the snapshot row.
                let case_id: i64 = self.conn.query_row(
                    "SELECT case_id FROM case_snapshots WHERE id = ?1",
                    params![snapshot_id],
                    |row| row.get(0),
                )?;
                SnapshotPayload {
                    entities: self
                        .list_entities(case_id)?
                        .iter()
                        .map(|e| SnapshotEntity {
                            id: e.id.clone(),
                            entity_type: e.entity_type.clone(),
                            label: e.label.clone(),
                            source_event_type: e.source_event_type.clone(),
                        })
                        .collect(),
                    edges: self
                        .list_edges(case_id)?
                        .iter()
                        .map(|edge| SnapshotEdge {
                            id: edge.id.clone(),
                            source_id: edge.source_id.clone(),
                            target_id: edge.target_id.clone(),
                            label: edge.label.clone(),
                        })
                        .collect(),
                }
            }
        };
        let older_e: std::collections::HashMap<&str, &SnapshotEntity> =
            older.entities.iter().map(|e| (e.id.as_str(), e)).collect();
        let newer_e: std::collections::HashMap<&str, &SnapshotEntity> =
            newer.entities.iter().map(|e| (e.id.as_str(), e)).collect();
        let mut added_entities = Vec::new();
        let mut removed_entities = Vec::new();
        let mut renamed_entities = Vec::new();
        for (id, after) in &newer_e {
            match older_e.get(id) {
                None => added_entities.push((*after).clone()),
                Some(before) if before.label != after.label => {
                    renamed_entities.push(RenamedEntity {
                        id: id.to_string(),
                        entity_type: after.entity_type.clone(),
                        before: before.label.clone(),
                        after: after.label.clone(),
                    });
                }
                _ => {}
            }
        }
        for (id, before) in &older_e {
            if !newer_e.contains_key(id) {
                removed_entities.push((*before).clone());
            }
        }
        let older_ed: std::collections::HashSet<&str> =
            older.edges.iter().map(|e| e.id.as_str()).collect();
        let newer_ed: std::collections::HashSet<&str> =
            newer.edges.iter().map(|e| e.id.as_str()).collect();
        let added_edges = newer
            .edges
            .iter()
            .filter(|e| !older_ed.contains(e.id.as_str()))
            .cloned()
            .collect();
        let removed_edges = older
            .edges
            .iter()
            .filter(|e| !newer_ed.contains(e.id.as_str()))
            .cloned()
            .collect();
        Ok(CaseDiff {
            added_entities,
            removed_entities,
            renamed_entities,
            added_edges,
            removed_edges,
        })
    }

    fn row_to_cross_case(row: &rusqlite::Row<'_>) -> rusqlite::Result<CrossCaseMatch> {
        Ok(CrossCaseMatch {
            entity_id: row.get(0)?,
            case_id: row.get(1)?,
            entity_type: row.get(2)?,
            label: row.get(3)?,
            bookmarked: row.get::<_, i64>(4).unwrap_or(0) != 0,
            case_name: row.get(5)?,
        })
    }

    /// Append one audit row. Best-effort: if the insert fails (e.g. the
    /// case_id no longer exists), we swallow the error and log to stderr
    /// rather than failing the underlying mutation — audit is observation,
    /// not preconditions.
    pub fn record_audit_event(
        &self,
        case_id: i64,
        kind: &str,
        entity_id: Option<&str>,
        payload: Option<&str>,
        actor: Option<&str>,
    ) {
        let now = Utc::now();
        let result = self.conn.execute(
            "INSERT INTO audit_events(case_id, kind, entity_id, payload, created_at, actor)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![case_id, kind, entity_id, payload, now.to_rfc3339(), actor],
        );
        if let Err(e) = result {
            eprintln!("maltrace audit: failed to record {kind} on case {case_id}: {e}");
        }
    }

    pub fn list_audit_events(&self, case_id: i64, limit: i64) -> Result<Vec<AuditEvent>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, case_id, kind, entity_id, payload, created_at, actor
             FROM audit_events
             WHERE case_id = ?1
             ORDER BY created_at DESC
             LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![case_id, limit.clamp(1, 500)], |row| {
                let created_at: String = row.get(5)?;
                let actor: Option<String> =
                    row.get::<_, Option<String>>(6).unwrap_or(None);
                Ok(AuditEvent {
                    id: row.get(0)?,
                    case_id: row.get(1)?,
                    kind: row.get(2)?,
                    entity_id: row.get(3)?,
                    payload: row.get(4)?,
                    created_at: created_at.parse().unwrap_or_else(|_| Utc::now()),
                    actor,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn list_transform_runs_for_entity(
        &self,
        entity_id: &str,
        limit: i64,
    ) -> Result<Vec<TransformRun>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, case_id, transform_id, source_entity_id, source_label,
                    started_at, duration_ms, status, new_entity_count,
                    error_message, stderr_excerpt, created_by_user
             FROM transform_runs
             WHERE source_entity_id = ?1
             ORDER BY started_at DESC
             LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![entity_id, limit], Self::row_to_transform_run)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Rename an entity. Refuses to rename across the
    /// `(case_id, entity_type, label)` uniqueness boundary — the caller
    /// gets a descriptive error rather than a raw SQLite constraint hit.
    pub fn update_entity_label(&self, id: &str, new_label: &str) -> Result<()> {
        let trimmed = new_label.trim();
        if trimmed.is_empty() {
            anyhow::bail!("entity label cannot be empty");
        }
        // Look up current (case_id, entity_type) so we can check the
        // collision rule upfront with a nicer message.
        let existing: Option<(i64, String)> = self
            .conn
            .query_row(
                "SELECT case_id, entity_type FROM entities WHERE id = ?1",
                params![id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let (case_id, entity_type) = existing.ok_or_else(|| {
            anyhow::anyhow!("entity {id} not found")
        })?;
        let conflict: Option<String> = self
            .conn
            .query_row(
                "SELECT id FROM entities
                 WHERE case_id = ?1 AND entity_type = ?2 AND label = ?3 AND id != ?4",
                params![case_id, entity_type, trimmed, id],
                |row| row.get(0),
            )
            .optional()?;
        if conflict.is_some() {
            anyhow::bail!(
                "another {entity_type} named `{trimmed}` already exists in this case"
            );
        }
        self.conn.execute(
            "UPDATE entities SET label = ?2 WHERE id = ?1",
            params![id, trimmed],
        )?;
        Ok(())
    }

    pub fn update_entity_position(&self, id: &str, x: f64, y: f64) -> Result<()> {
        self.conn.execute(
            "UPDATE entities SET pos_x = ?2, pos_y = ?3 WHERE id = ?1",
            params![id, x, y],
        )?;
        Ok(())
    }

    /// Delete an entity. The `ON DELETE CASCADE` on `edges.source_id` /
    /// `edges.target_id` removes any edges touching it in the same statement.
    pub fn delete_entity(&self, id: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM entities WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn upsert_edge(
        &self,
        case_id: i64,
        source_id: &str,
        target_id: &str,
        label: Option<&str>,
        run_id: Option<i64>,
    ) -> Result<Edge> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now();
        self.conn.execute(
            "INSERT OR IGNORE INTO edges(id, case_id, source_id, target_id, label, created_at, run_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![id, case_id, source_id, target_id, label, now.to_rfc3339(), run_id],
        )?;
        // If a row already existed and we have a run_id but it doesn't,
        // backfill it — same logic as entities: the first run claiming
        // an edge gets attribution; later runs don't reattribute.
        if let Some(rid) = run_id {
            self.conn.execute(
                "UPDATE edges SET run_id = ?5
                 WHERE case_id = ?1 AND source_id = ?2 AND target_id = ?3
                   AND COALESCE(label, '') = COALESCE(?4, '')
                   AND run_id IS NULL",
                params![case_id, source_id, target_id, label, rid],
            )?;
        }
        // Return whichever edge ended up in the table (existing or just-inserted).
        self.conn.query_row(
            "SELECT id, case_id, source_id, target_id, label, created_at, run_id, relationship_type
             FROM edges
             WHERE case_id = ?1 AND source_id = ?2 AND target_id = ?3
               AND COALESCE(label, '') = COALESCE(?4, '')",
            params![case_id, source_id, target_id, label],
            Self::row_to_edge,
        ).map_err(Into::into)
    }

    pub fn list_edges(&self, case_id: i64) -> Result<Vec<Edge>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, case_id, source_id, target_id, label, created_at, run_id, relationship_type
             FROM edges WHERE case_id = ?1 ORDER BY created_at ASC",
        )?;
        let rows = stmt
            .query_map(params![case_id], Self::row_to_edge)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    fn row_to_edge(row: &rusqlite::Row<'_>) -> rusqlite::Result<Edge> {
        let created_at: String = row.get(5)?;
        let run_id: Option<i64> = row.get::<_, Option<i64>>(6).unwrap_or(None);
        let relationship_type: Option<String> =
            row.get::<_, Option<String>>(7).unwrap_or(None);
        Ok(Edge {
            id: row.get(0)?,
            case_id: row.get(1)?,
            source_id: row.get(2)?,
            target_id: row.get(3)?,
            label: row.get(4)?,
            created_at: created_at.parse().unwrap_or_else(|_| Utc::now()),
            run_id,
            relationship_type,
        })
    }

    /// Fetch a single edge by id. Used by the edge inspector to refresh
    /// after a run completes.
    pub fn get_edge(&self, id: &str) -> Result<Option<Edge>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, case_id, source_id, target_id, label, created_at, run_id, relationship_type
                 FROM edges WHERE id = ?1",
                params![id],
                Self::row_to_edge,
            )
            .optional()?)
    }

    /// Lookup of a single transform_run by id — used by provenance reads.
    pub fn get_transform_run(&self, id: i64) -> Result<Option<TransformRun>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, case_id, transform_id, source_entity_id, source_label,
                        started_at, duration_ms, status, new_entity_count,
                        error_message, stderr_excerpt
                 FROM transform_runs WHERE id = ?1",
                params![id],
                Self::row_to_transform_run,
            )
            .optional()?)
    }

    pub fn list_custom_entity_types(&self) -> Result<Vec<CustomEntityType>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, label, kind, color, created_at
             FROM custom_entity_types
             ORDER BY label ASC",
        )?;
        let rows = stmt
            .query_map([], |row| {
                let created_at: String = row.get(4)?;
                Ok(CustomEntityType {
                    id: row.get(0)?,
                    label: row.get(1)?,
                    kind: row.get(2)?,
                    color: row.get(3)?,
                    created_at: created_at.parse().unwrap_or_else(|_| Utc::now()),
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn create_custom_entity_type(
        &self,
        id: &str,
        label: &str,
        kind: &str,
        color: &str,
    ) -> Result<CustomEntityType> {
        let now = Utc::now();
        self.conn.execute(
            "INSERT INTO custom_entity_types(id, label, kind, color, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, label, kind, color, now.to_rfc3339()],
        )?;
        Ok(CustomEntityType {
            id: id.to_string(),
            label: label.to_string(),
            kind: kind.to_string(),
            color: color.to_string(),
            created_at: now,
        })
    }

    pub fn delete_custom_entity_type(&self, id: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM custom_entity_types WHERE id = ?1", params![id])?;
        Ok(())
    }

    // ─── Annotations ─────────────────────────────────────────
    pub fn list_annotations(&self, case_id: i64) -> Result<Vec<Annotation>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, case_id, text, pos_x, pos_y, color, created_at
             FROM annotations WHERE case_id = ?1 ORDER BY created_at ASC",
        )?;
        let rows = stmt
            .query_map(params![case_id], Self::row_to_annotation)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn create_annotation(
        &self,
        case_id: i64,
        text: &str,
        pos_x: f64,
        pos_y: f64,
        color: &str,
    ) -> Result<Annotation> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now();
        self.conn.execute(
            "INSERT INTO annotations(id, case_id, text, pos_x, pos_y, color, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![id, case_id, text, pos_x, pos_y, color, now.to_rfc3339()],
        )?;
        Ok(Annotation {
            id,
            case_id,
            text: text.to_string(),
            pos_x,
            pos_y,
            color: color.to_string(),
            created_at: now,
        })
    }

    /// Patch any subset of an annotation's fields. `None` means "leave
    /// alone" — saves the UI from having to send the full record back on
    /// every position drag.
    pub fn update_annotation(
        &self,
        id: &str,
        text: Option<&str>,
        pos_x: Option<f64>,
        pos_y: Option<f64>,
        color: Option<&str>,
    ) -> Result<()> {
        if let Some(t) = text {
            self.conn.execute(
                "UPDATE annotations SET text = ?2 WHERE id = ?1",
                params![id, t],
            )?;
        }
        if let (Some(x), Some(y)) = (pos_x, pos_y) {
            self.conn.execute(
                "UPDATE annotations SET pos_x = ?2, pos_y = ?3 WHERE id = ?1",
                params![id, x, y],
            )?;
        }
        if let Some(c) = color {
            self.conn.execute(
                "UPDATE annotations SET color = ?2 WHERE id = ?1",
                params![id, c],
            )?;
        }
        Ok(())
    }

    pub fn delete_annotation(&self, id: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM annotations WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// Resolve an annotation id to its owning case_id. Returns `None`
    /// when the annotation doesn't exist (the caller will hit a clean
    /// "no rows updated" downstream).
    pub fn list_annotations_case_id(&self, id: &str) -> Result<Option<i64>> {
        Ok(self
            .conn
            .query_row(
                "SELECT case_id FROM annotations WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .optional()?)
    }

    fn row_to_annotation(row: &rusqlite::Row<'_>) -> rusqlite::Result<Annotation> {
        let created_at: String = row.get(6)?;
        Ok(Annotation {
            id: row.get(0)?,
            case_id: row.get(1)?,
            text: row.get(2)?,
            pos_x: row.get(3)?,
            pos_y: row.get(4)?,
            color: row.get(5)?,
            created_at: created_at.parse().unwrap_or_else(|_| Utc::now()),
        })
    }

    // ─── Entity merge ────────────────────────────────────────
    /// Merge `drop_id` into `keep_id`: re-point every edge incident on
    /// `drop_id` so it touches `keep_id` instead, drop self-loops + any
    /// dupes (existing `uniq_edge` index dedupes via INSERT-or-ignore
    /// via our upsert), then delete `drop_id` (which cascades the
    /// originals).
    ///
    /// Both entities must live in the same case (the caller checks). We
    /// don't try to merge `data` / `notes` / `bookmarked` — the `keep`
    /// side wins on those, which is the simple-and-correct choice when
    /// the user has eyeballed which entity is the canonical one.
    pub fn merge_entities(&self, keep_id: &str, drop_id: &str) -> Result<()> {
        if keep_id == drop_id {
            anyhow::bail!("cannot merge an entity into itself");
        }
        // Resolve both ends so we know their case + can refuse cross-case
        // merges (which would otherwise leave dangling cascade).
        let keep = self
            .get_entity(keep_id)?
            .ok_or_else(|| anyhow::anyhow!("keep entity {keep_id} not found"))?;
        let drop = self
            .get_entity(drop_id)?
            .ok_or_else(|| anyhow::anyhow!("drop entity {drop_id} not found"))?;
        if keep.case_id != drop.case_id {
            anyhow::bail!("cannot merge entities across cases");
        }
        let case_id = keep.case_id;
        // Snapshot incident edges first; iterating while mutating breaks
        // rusqlite's borrow checker and would risk skipping rows anyway.
        let incident: Vec<Edge> = self
            .list_edges(case_id)?
            .into_iter()
            .filter(|e| e.source_id == drop_id || e.target_id == drop_id)
            .collect();
        for e in &incident {
            let (new_src, new_tgt) = (
                if e.source_id == drop_id { keep_id } else { e.source_id.as_str() },
                if e.target_id == drop_id { keep_id } else { e.target_id.as_str() },
            );
            if new_src == new_tgt {
                // Self-loop after re-point — drop silently.
                continue;
            }
            // upsert_edge respects the `uniq_edge` index, so re-pointing
            // an edge that already exists at the keep side is a no-op.
            // Carry the original edge's run_id forward so merge doesn't
            // strip provenance from the survivor.
            self.upsert_edge(case_id, new_src, new_tgt, e.label.as_deref(), e.run_id)?;
        }
        // Cascade deletes the drop entity's original edges; the
        // re-pointed copies we just inserted survive.
        self.delete_entity(drop_id)?;
        Ok(())
    }

    /// Insert a transform_run row up-front with status `"pending"` and
    /// return its id. The row is later completed by
    /// `complete_transform_run`. Splitting it lets us stamp the run id
    /// onto every entity / edge the transform produces — which is what
    /// the inspector's provenance UI reads.
    /// Re-insert a transform_run row, picking a fresh auto-generated id
    /// rather than the original. Returns the new id so the caller can
    /// build an `old_id → new_id` map and patch entities'
    /// `created_by_run_id`. Used only by the backup-restore path.
    #[allow(clippy::too_many_arguments)]
    pub fn restore_transform_run(
        &self,
        case_id: i64,
        transform_id: &str,
        source_entity_id: Option<&str>,
        source_label: Option<&str>,
        started_at: DateTime<Utc>,
        duration_ms: i64,
        status: &str,
        new_entity_count: i64,
        error_message: Option<&str>,
        stderr_excerpt: Option<&str>,
        created_by_user: Option<&str>,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO transform_runs(
                case_id, transform_id, source_entity_id, source_label,
                started_at, duration_ms, status, new_entity_count,
                error_message, stderr_excerpt, created_by_user
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                case_id,
                transform_id,
                source_entity_id,
                source_label,
                started_at.to_rfc3339(),
                duration_ms,
                status,
                new_entity_count,
                error_message,
                stderr_excerpt,
                created_by_user,
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Patch `entities.created_by_run_id` for every row in the case
    /// where it matches `old_id`, swapping in `new_id`. Used by the
    /// backup-restore run-id remapping.
    pub fn remap_entity_run_id(
        &self,
        case_id: i64,
        old_id: i64,
        new_id: i64,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE entities SET created_by_run_id = ?3
             WHERE case_id = ?1 AND created_by_run_id = ?2",
            params![case_id, old_id, new_id],
        )?;
        // Mirror onto edges so the edge inspector keeps showing the
        // same originating run after restore.
        self.conn.execute(
            "UPDATE edges SET run_id = ?3
             WHERE case_id = ?1 AND run_id = ?2",
            params![case_id, old_id, new_id],
        )?;
        Ok(())
    }

    pub fn start_transform_run(
        &self,
        case_id: i64,
        transform_id: &str,
        source_entity_id: Option<&str>,
        source_label: Option<&str>,
        started_at: DateTime<Utc>,
        actor: Option<&str>,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO transform_runs(
                case_id, transform_id, source_entity_id, source_label,
                started_at, duration_ms, status, new_entity_count,
                error_message, stderr_excerpt, created_by_user
             ) VALUES (?1, ?2, ?3, ?4, ?5, 0, 'pending', 0, NULL, NULL, ?6)",
            params![
                case_id,
                transform_id,
                source_entity_id,
                source_label,
                started_at.to_rfc3339(),
                actor,
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Patch a previously-started run with the final outcome. Called once
    /// per started run; no-op (and no error) if the run id doesn't exist
    /// — defensive against double-completion.
    pub fn complete_transform_run(
        &self,
        run_id: i64,
        duration_ms: i64,
        status: &str,
        new_entity_count: i64,
        error_message: Option<&str>,
        stderr_excerpt: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE transform_runs
                SET duration_ms = ?2, status = ?3, new_entity_count = ?4,
                    error_message = ?5, stderr_excerpt = ?6
              WHERE id = ?1",
            params![
                run_id,
                duration_ms,
                status,
                new_entity_count,
                error_message,
                stderr_excerpt,
            ],
        )?;
        Ok(())
    }

    /// Most recent runs first. `limit` is a small ceiling (e.g. 200) so the
    /// UI panel stays snappy on noisy cases.
    pub fn list_transform_runs(&self, case_id: i64, limit: i64) -> Result<Vec<TransformRun>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, case_id, transform_id, source_entity_id, source_label,
                    started_at, duration_ms, status, new_entity_count,
                    error_message, stderr_excerpt, created_by_user
             FROM transform_runs
             WHERE case_id = ?1
             ORDER BY started_at DESC
             LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![case_id, limit], Self::row_to_transform_run)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    fn row_to_transform_run(row: &rusqlite::Row<'_>) -> rusqlite::Result<TransformRun> {
        let started_at: String = row.get(5)?;
        let created_by_user: Option<String> =
            row.get::<_, Option<String>>(11).unwrap_or(None);
        Ok(TransformRun {
            id: row.get(0)?,
            case_id: row.get(1)?,
            transform_id: row.get(2)?,
            source_entity_id: row.get(3)?,
            source_label: row.get(4)?,
            started_at: started_at.parse().unwrap_or_else(|_| Utc::now()),
            duration_ms: row.get(6)?,
            status: row.get(7)?,
            new_entity_count: row.get(8)?,
            error_message: row.get(9)?,
            stderr_excerpt: row.get(10)?,
            created_by_user,
        })
    }

    // ─── Machine schedules ───────────────────────────────────
    pub fn create_machine_schedule(
        &self,
        case_id: i64,
        machine_id: &str,
        entity_id: &str,
        entity_label: &str,
        interval_seconds: i64,
    ) -> Result<MachineSchedule> {
        let interval = interval_seconds.max(60);
        let now = Utc::now();
        let next = now + chrono::Duration::seconds(interval);
        self.conn.execute(
            "INSERT INTO machine_schedules(
                case_id, machine_id, entity_id, entity_label,
                interval_seconds, enabled, created_at, next_run_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6, ?7)",
            params![
                case_id,
                machine_id,
                entity_id,
                entity_label,
                interval,
                now.to_rfc3339(),
                next.to_rfc3339(),
            ],
        )?;
        Ok(MachineSchedule {
            id: self.conn.last_insert_rowid(),
            case_id,
            machine_id: machine_id.to_string(),
            entity_id: entity_id.to_string(),
            entity_label: entity_label.to_string(),
            interval_seconds: interval,
            enabled: true,
            created_at: now,
            next_run_at: next,
            last_run_at: None,
        })
    }

    pub fn list_machine_schedules(&self, case_id: i64) -> Result<Vec<MachineSchedule>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, case_id, machine_id, entity_id, entity_label,
                    interval_seconds, enabled, created_at, next_run_at, last_run_at
             FROM machine_schedules
             WHERE case_id = ?1
             ORDER BY id DESC",
        )?;
        let rows = stmt
            .query_map(params![case_id], Self::row_to_schedule)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn list_due_schedules(&self, now: DateTime<Utc>) -> Result<Vec<MachineSchedule>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, case_id, machine_id, entity_id, entity_label,
                    interval_seconds, enabled, created_at, next_run_at, last_run_at
             FROM machine_schedules
             WHERE enabled = 1 AND next_run_at <= ?1
             ORDER BY next_run_at ASC",
        )?;
        let rows = stmt
            .query_map(params![now.to_rfc3339()], Self::row_to_schedule)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn set_machine_schedule_enabled(&self, id: i64, enabled: bool) -> Result<()> {
        self.conn.execute(
            "UPDATE machine_schedules SET enabled = ?2 WHERE id = ?1",
            params![id, enabled as i64],
        )?;
        Ok(())
    }

    pub fn delete_machine_schedule(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM machine_schedules WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// Patch a schedule with the timestamps after a firing — keeps the
    /// scheduler from immediately re-running the same row.
    pub fn touch_schedule_after_run(
        &self,
        id: i64,
        last_run_at: DateTime<Utc>,
        next_run_at: DateTime<Utc>,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE machine_schedules SET last_run_at = ?2, next_run_at = ?3 WHERE id = ?1",
            params![id, last_run_at.to_rfc3339(), next_run_at.to_rfc3339()],
        )?;
        Ok(())
    }

    pub fn record_schedule_run(
        &self,
        schedule_id: i64,
        started_at: DateTime<Utc>,
        status: &str,
        new_entity_count: i64,
        error_message: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO machine_schedule_runs(schedule_id, started_at, status, new_entity_count, error_message)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                schedule_id,
                started_at.to_rfc3339(),
                status,
                new_entity_count,
                error_message,
            ],
        )?;
        Ok(())
    }

    pub fn list_schedule_runs(
        &self,
        schedule_id: i64,
        limit: i64,
    ) -> Result<Vec<MachineScheduleRun>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, schedule_id, started_at, status, new_entity_count, error_message
             FROM machine_schedule_runs
             WHERE schedule_id = ?1
             ORDER BY started_at DESC
             LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![schedule_id, limit.clamp(1, 500)], |row| {
                let started_at: String = row.get(2)?;
                Ok(MachineScheduleRun {
                    id: row.get(0)?,
                    schedule_id: row.get(1)?,
                    started_at: started_at.parse().unwrap_or_else(|_| Utc::now()),
                    status: row.get(3)?,
                    new_entity_count: row.get(4)?,
                    error_message: row.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    fn row_to_schedule(row: &rusqlite::Row<'_>) -> rusqlite::Result<MachineSchedule> {
        let created_at: String = row.get(7)?;
        let next_run_at: String = row.get(8)?;
        let last_run_at: Option<String> = row.get(9).ok();
        let enabled_int: i64 = row.get::<_, i64>(6).unwrap_or(0);
        Ok(MachineSchedule {
            id: row.get(0)?,
            case_id: row.get(1)?,
            machine_id: row.get(2)?,
            entity_id: row.get(3)?,
            entity_label: row.get(4)?,
            interval_seconds: row.get(5)?,
            enabled: enabled_int != 0,
            created_at: created_at.parse().unwrap_or_else(|_| Utc::now()),
            next_run_at: next_run_at.parse().unwrap_or_else(|_| Utc::now()),
            last_run_at: last_run_at.and_then(|s| s.parse().ok()),
        })
    }

    // ─── Tags ────────────────────────────────────────────────
    pub fn list_tags_for_entity(&self, entity_id: &str) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT tag FROM entity_tags WHERE entity_id = ?1 ORDER BY tag ASC",
        )?;
        let rows = stmt
            .query_map(params![entity_id], |row| row.get(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Snapshot every (entity_id, tags[]) pair for one case in a single
    /// query so the case-load path can colour-code without N round-trips.
    pub fn list_tags_for_case(&self, case_id: i64) -> Result<HashMap<String, Vec<String>>> {
        let mut stmt = self.conn.prepare(
            "SELECT t.entity_id, t.tag
             FROM entity_tags t
             JOIN entities e ON e.id = t.entity_id
             WHERE e.case_id = ?1
             ORDER BY t.entity_id, t.tag",
        )?;
        let mut out: HashMap<String, Vec<String>> = HashMap::new();
        let rows = stmt.query_map(params![case_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for r in rows {
            let (eid, tag) = r?;
            out.entry(eid).or_default().push(tag);
        }
        Ok(out)
    }

    pub fn add_tag(&self, entity_id: &str, tag: &str) -> Result<()> {
        let trimmed = tag.trim();
        if trimmed.is_empty() {
            anyhow::bail!("tag cannot be empty");
        }
        // Normalise to lowercase so the same tag with different casing
        // collapses into one row.
        let lc = trimmed.to_lowercase();
        self.conn.execute(
            "INSERT OR IGNORE INTO entity_tags(entity_id, tag) VALUES (?1, ?2)",
            params![entity_id, lc],
        )?;
        Ok(())
    }

    pub fn remove_tag(&self, entity_id: &str, tag: &str) -> Result<()> {
        let lc = tag.trim().to_lowercase();
        self.conn.execute(
            "DELETE FROM entity_tags WHERE entity_id = ?1 AND tag = ?2",
            params![entity_id, lc],
        )?;
        Ok(())
    }

    /// Insert an entity with a caller-provided id and pre-existing
    /// created_at. Used by the destructive-undo restore path so the
    /// resurrected row keeps the original id (every edge that referenced
    /// it can be restored verbatim) and original timestamp (so
    /// provenance / timeline stays accurate).
    #[allow(clippy::too_many_arguments)]
    pub fn restore_entity(
        &self,
        id: &str,
        case_id: i64,
        entity_type: &str,
        source_event_type: Option<&str>,
        label: &str,
        data: Option<&str>,
        pos_x: Option<f64>,
        pos_y: Option<f64>,
        created_at: DateTime<Utc>,
        notes: Option<&str>,
        bookmarked: bool,
        locked: bool,
        created_by_run_id: Option<i64>,
        created_by_transform_id: Option<&str>,
        source_url: Option<&str>,
        created_by_user: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO entities(
                id, case_id, entity_type, source_event_type, label, data,
                pos_x, pos_y, created_at, notes, bookmarked, locked,
                created_by_run_id, created_by_transform_id, source_url,
                created_by_user
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
             ON CONFLICT(id) DO NOTHING",
            params![
                id,
                case_id,
                entity_type,
                source_event_type,
                label,
                data,
                pos_x,
                pos_y,
                created_at.to_rfc3339(),
                notes,
                bookmarked as i64,
                locked as i64,
                created_by_run_id,
                created_by_transform_id,
                source_url,
                created_by_user,
            ],
        )?;
        Ok(())
    }

    /// Insert an edge with a caller-provided id + timestamp. Mirror of
    /// `restore_entity` for the undo-delete path.
    pub fn restore_edge(
        &self,
        id: &str,
        case_id: i64,
        source_id: &str,
        target_id: &str,
        label: Option<&str>,
        created_at: DateTime<Utc>,
        run_id: Option<i64>,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO edges(id, case_id, source_id, target_id, label, created_at, run_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(id) DO NOTHING",
            params![id, case_id, source_id, target_id, label, created_at.to_rfc3339(), run_id],
        )?;
        Ok(())
    }

    fn row_to_entity(row: &rusqlite::Row<'_>) -> rusqlite::Result<Entity> {
        let created_at: String = row.get(8)?;
        // notes/bookmarked were added in schema v5, locked in v6,
        // provenance cols in v7, `created_by_user` in v10 — read
        // defensively so older rows still surface cleanly.
        let notes: Option<String> = row.get::<_, Option<String>>(9).unwrap_or(None);
        let bookmarked_int: i64 = row.get::<_, i64>(10).unwrap_or(0);
        let locked_int: i64 = row.get::<_, i64>(11).unwrap_or(0);
        let created_by_run_id: Option<i64> =
            row.get::<_, Option<i64>>(12).unwrap_or(None);
        let created_by_transform_id: Option<String> =
            row.get::<_, Option<String>>(13).unwrap_or(None);
        let source_url: Option<String> =
            row.get::<_, Option<String>>(14).unwrap_or(None);
        let created_by_user: Option<String> =
            row.get::<_, Option<String>>(15).unwrap_or(None);
        Ok(Entity {
            id: row.get(0)?,
            case_id: row.get(1)?,
            entity_type: row.get(2)?,
            source_event_type: row.get(3)?,
            label: row.get(4)?,
            data: row.get(5)?,
            pos_x: row.get(6)?,
            pos_y: row.get(7)?,
            created_at: created_at.parse().unwrap_or_else(|_| Utc::now()),
            notes,
            bookmarked: bookmarked_int != 0,
            locked: locked_int != 0,
            created_by_run_id,
            created_by_transform_id,
            source_url,
            created_by_user,
        })
    }

    // ─── Notifications ───────────────────────────────────────
    pub fn create_notification(
        &self,
        case_id: i64,
        kind: &str,
        title: &str,
        detail: Option<&str>,
        payload: Option<&str>,
    ) -> Result<Notification> {
        let now = Utc::now();
        self.conn.execute(
            "INSERT INTO notifications(case_id, kind, title, detail, payload, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![case_id, kind, title, detail, payload, now.to_rfc3339()],
        )?;
        Ok(Notification {
            id: self.conn.last_insert_rowid(),
            case_id,
            kind: kind.to_string(),
            title: title.to_string(),
            detail: detail.map(String::from),
            payload: payload.map(String::from),
            created_at: now,
            read_at: None,
        })
    }

    pub fn list_notifications(
        &self,
        case_id: i64,
        limit: i64,
    ) -> Result<Vec<Notification>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, case_id, kind, title, detail, payload, created_at, read_at
             FROM notifications
             WHERE case_id = ?1
             ORDER BY created_at DESC
             LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![case_id, limit.clamp(1, 500)], Self::row_to_notification)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn unread_notification_count(&self, case_id: i64) -> Result<i64> {
        Ok(self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM notifications WHERE case_id = ?1 AND read_at IS NULL",
                params![case_id],
                |r| r.get(0),
            )
            .unwrap_or(0))
    }

    pub fn mark_notifications_read(&self, case_id: i64) -> Result<()> {
        let now = Utc::now();
        self.conn.execute(
            "UPDATE notifications SET read_at = ?2
             WHERE case_id = ?1 AND read_at IS NULL",
            params![case_id, now.to_rfc3339()],
        )?;
        Ok(())
    }

    fn row_to_notification(row: &rusqlite::Row<'_>) -> rusqlite::Result<Notification> {
        let created_at: String = row.get(6)?;
        let read_at_str: Option<String> = row.get(7).ok();
        Ok(Notification {
            id: row.get(0)?,
            case_id: row.get(1)?,
            kind: row.get(2)?,
            title: row.get(3)?,
            detail: row.get(4)?,
            payload: row.get(5)?,
            created_at: created_at.parse().unwrap_or_else(|_| Utc::now()),
            read_at: read_at_str.and_then(|s| s.parse().ok()),
        })
    }

    // ─── Saved searches ──────────────────────────────────────
    pub fn create_saved_search(
        &self,
        case_id: i64,
        name: &str,
        predicate: &str,
    ) -> Result<SavedSearch> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            anyhow::bail!("saved-search name cannot be empty");
        }
        let now = Utc::now();
        self.conn.execute(
            "INSERT INTO saved_searches(case_id, name, predicate, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![case_id, trimmed, predicate, now.to_rfc3339()],
        )?;
        Ok(SavedSearch {
            id: self.conn.last_insert_rowid(),
            case_id,
            name: trimmed.to_string(),
            predicate: predicate.to_string(),
            created_at: now,
        })
    }

    pub fn list_saved_searches(&self, case_id: i64) -> Result<Vec<SavedSearch>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, case_id, name, predicate, created_at
             FROM saved_searches
             WHERE case_id = ?1
             ORDER BY name ASC",
        )?;
        let rows = stmt
            .query_map(params![case_id], |row| {
                let created_at: String = row.get(4)?;
                Ok(SavedSearch {
                    id: row.get(0)?,
                    case_id: row.get(1)?,
                    name: row.get(2)?,
                    predicate: row.get(3)?,
                    created_at: created_at.parse().unwrap_or_else(|_| Utc::now()),
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn update_saved_search(
        &self,
        id: i64,
        name: &str,
        predicate: &str,
    ) -> Result<()> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            anyhow::bail!("saved-search name cannot be empty");
        }
        self.conn.execute(
            "UPDATE saved_searches SET name = ?2, predicate = ?3 WHERE id = ?1",
            params![id, trimmed, predicate],
        )?;
        Ok(())
    }

    pub fn delete_saved_search(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM saved_searches WHERE id = ?1", params![id])?;
        Ok(())
    }

    // ─── Typed edges ─────────────────────────────────────────
    pub fn update_edge_relationship(
        &self,
        edge_id: &str,
        relationship_type: Option<&str>,
    ) -> Result<()> {
        let trimmed = relationship_type.map(str::trim).filter(|s| !s.is_empty());
        self.conn.execute(
            "UPDATE edges SET relationship_type = ?2 WHERE id = ?1",
            params![edge_id, trimmed],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Open an in-memory case DB so each test gets a fresh database that
    /// dies with the test thread. SQLCipher's `bundled-sqlcipher-vendored-
    /// openssl` feature still supports `:memory:` URIs.
    fn open_mem() -> Db {
        Db::open(Path::new(":memory:"), None).expect("open in-memory db")
    }

    #[test]
    fn cross_case_search_filters_by_label() {
        let db = open_mem();
        let c1 = db.create_case("Alpha").unwrap();
        let c2 = db.create_case("Beta").unwrap();
        db.upsert_entity(c1.id, "Domain", None, "example.com", None, None, None, None)
            .unwrap();
        db.upsert_entity(c2.id, "Domain", None, "different.org", None, None, None, None)
            .unwrap();
        db.upsert_entity(c2.id, "EmailAddress", None, "bob@example.com", None, None, None, None)
            .unwrap();

        let hits = db.search_entities_across_cases("example", 10).unwrap();
        // Two hits — one in each case — and they carry the case name.
        assert_eq!(hits.len(), 2);
        let case_names: std::collections::HashSet<&str> =
            hits.iter().map(|h| h.case_name.as_str()).collect();
        assert!(case_names.contains("Alpha"));
        assert!(case_names.contains("Beta"));
    }

    #[test]
    fn cross_case_search_uses_fts5_prefix_matching() {
        // FTS5 makes "exa" find "example.com"; LIKE %exa% would too,
        // but FTS5 should match even on a multi-word prefix query like
        // "example evi" → matches "example evidence note" etc.
        let db = open_mem();
        let c = db.create_case("FTS").unwrap();
        db.upsert_entity(c.id, "Domain", None, "example.com", None, None, None, None)
            .unwrap();
        db.upsert_entity(c.id, "Person", None, "Alice Example", None, None, None, None)
            .unwrap();
        let hits = db.search_entities_across_cases("exa", 10).unwrap();
        assert_eq!(hits.len(), 2);
        // Multi-word prefix: matches the Person.
        let hits = db.search_entities_across_cases("alice exa", 10).unwrap();
        let labels: Vec<&str> = hits.iter().map(|h| h.label.as_str()).collect();
        assert!(labels.contains(&"Alice Example"));
    }

    #[test]
    fn fts5_index_stays_in_sync_via_triggers() {
        let db = open_mem();
        let c = db.create_case("FTS Triggers").unwrap();
        let e = db
            .upsert_entity(c.id, "Domain", None, "first.example", None, None, None, None)
            .unwrap();
        assert!(!db
            .search_entities_across_cases("first", 10)
            .unwrap()
            .is_empty());
        // Rename — FTS should follow.
        db.update_entity_label(&e.id, "second.example").unwrap();
        assert!(db
            .search_entities_across_cases("first", 10)
            .unwrap()
            .is_empty());
        assert!(!db
            .search_entities_across_cases("second", 10)
            .unwrap()
            .is_empty());
        // Delete — gone.
        db.delete_entity(&e.id).unwrap();
        assert!(db
            .search_entities_across_cases("second", 10)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn cross_case_search_empty_query_returns_nothing() {
        let db = open_mem();
        db.create_case("X").unwrap();
        assert!(db.search_entities_across_cases("", 10).unwrap().is_empty());
        assert!(db.search_entities_across_cases("   ", 10).unwrap().is_empty());
    }

    #[test]
    fn audit_event_round_trip() {
        let db = open_mem();
        let c = db.create_case("Audit Case").unwrap();
        db.record_audit_event(c.id, "entity_renamed", Some("ent-1"), Some(r#"{"from":"a","to":"b"}"#), None);
        db.record_audit_event(c.id, "entity_deleted", Some("ent-1"), None, Some("alice"));
        let events = db.list_audit_events(c.id, 100).unwrap();
        // Newest first.
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].kind, "entity_deleted");
        assert_eq!(events[0].actor.as_deref(), Some("alice"));
        assert_eq!(events[1].kind, "entity_renamed");
    }

    #[test]
    fn merge_repoints_edges_and_deletes_drop() {
        let db = open_mem();
        let c = db.create_case("Merge Case").unwrap();
        let a = db
            .upsert_entity(c.id, "EmailAddress", None, "bob@example.com", None, None, None, None)
            .unwrap();
        let b = db
            .upsert_entity(c.id, "EmailAddress", None, "bob@EXAMPLE.com", None, None, None, None)
            .unwrap();
        let domain = db
            .upsert_entity(c.id, "Domain", None, "example.com", None, None, None, None)
            .unwrap();
        // One edge from each email → the domain.
        db.upsert_edge(c.id, &a.id, &domain.id, Some("manual"), None).unwrap();
        db.upsert_edge(c.id, &b.id, &domain.id, Some("manual"), None).unwrap();

        db.merge_entities(&a.id, &b.id).unwrap();

        // The duplicate email is gone.
        assert!(db.get_entity(&b.id).unwrap().is_none());
        // The kept email still exists.
        assert!(db.get_entity(&a.id).unwrap().is_some());
        // Edges: exactly one (a → domain). Both originals collapsed into
        // the same `(case, source, target, label)` row via upsert dedup.
        let edges = db.list_edges(c.id).unwrap();
        let from_a: Vec<_> = edges
            .iter()
            .filter(|e| e.source_id == a.id && e.target_id == domain.id)
            .collect();
        assert_eq!(from_a.len(), 1);
    }

    #[test]
    fn merge_self_is_rejected() {
        let db = open_mem();
        let c = db.create_case("Self Case").unwrap();
        let a = db
            .upsert_entity(c.id, "Domain", None, "example.com", None, None, None, None)
            .unwrap();
        let err = db.merge_entities(&a.id, &a.id).unwrap_err();
        assert!(err.to_string().contains("itself"));
    }

    #[test]
    fn merge_across_cases_is_rejected() {
        let db = open_mem();
        let c1 = db.create_case("Alpha").unwrap();
        let c2 = db.create_case("Beta").unwrap();
        let a = db
            .upsert_entity(c1.id, "Domain", None, "a.com", None, None, None, None)
            .unwrap();
        let b = db
            .upsert_entity(c2.id, "Domain", None, "b.com", None, None, None, None)
            .unwrap();
        let err = db.merge_entities(&a.id, &b.id).unwrap_err();
        assert!(err.to_string().contains("across cases"));
    }

    #[test]
    fn label_rename_collision_detected() {
        let db = open_mem();
        let c = db.create_case("Rename Case").unwrap();
        db.upsert_entity(c.id, "Domain", None, "first.com", None, None, None, None)
            .unwrap();
        let second = db
            .upsert_entity(c.id, "Domain", None, "second.com", None, None, None, None)
            .unwrap();
        let err = db.update_entity_label(&second.id, "first.com").unwrap_err();
        assert!(err.to_string().contains("already exists"));
    }

    #[test]
    fn snapshot_diff_round_trip() {
        // Capture a snapshot, mutate the live case, then diff: every
        // change kind we surface (add/remove/rename for entities, add/
        // remove for edges) should show up once and only once.
        let db = open_mem();
        let c = db.create_case("Snap").unwrap();
        let a = db
            .upsert_entity(c.id, "Domain", None, "kept.example", None, None, None, None)
            .unwrap();
        let b = db
            .upsert_entity(c.id, "Domain", None, "removed.example", None, None, None, None)
            .unwrap();
        let r = db
            .upsert_entity(c.id, "Domain", None, "rename-me.example", None, None, None, None)
            .unwrap();
        db.upsert_edge(c.id, &a.id, &b.id, Some("links_to"), None)
            .unwrap();

        let snap = db.create_case_snapshot(c.id, "baseline").unwrap();
        assert_eq!(snap.entity_count, 3);
        assert_eq!(snap.edge_count, 1);

        // Mutate the live case: add one entity, remove `b`, rename `r`,
        // add one edge between the new entity and `a`, removing the
        // existing a→b edge as a side-effect of the b-deletion.
        let new_entity = db
            .upsert_entity(c.id, "Person", None, "Alice", None, None, None, None)
            .unwrap();
        db.delete_entity(&b.id).unwrap();
        db.update_entity_label(&r.id, "renamed.example").unwrap();
        db.upsert_edge(c.id, &new_entity.id, &a.id, Some("owns"), None)
            .unwrap();

        let diff = db.diff_case_snapshots(snap.id, None).unwrap();
        // Added: one new Person.
        assert_eq!(diff.added_entities.len(), 1);
        assert_eq!(diff.added_entities[0].label, "Alice");
        // Removed: `b`.
        assert_eq!(diff.removed_entities.len(), 1);
        assert_eq!(diff.removed_entities[0].label, "removed.example");
        // Renamed: `r`.
        assert_eq!(diff.renamed_entities.len(), 1);
        assert_eq!(diff.renamed_entities[0].before, "rename-me.example");
        assert_eq!(diff.renamed_entities[0].after, "renamed.example");
        // Edges: the new (new_entity → a) edge added; the (a → b) edge
        // removed (cascaded when b was deleted).
        assert_eq!(diff.added_edges.len(), 1);
        assert_eq!(diff.removed_edges.len(), 1);

        // Snapshot list reflects what we wrote.
        let list = db.list_case_snapshots(c.id).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "baseline");

        // Delete works.
        db.delete_case_snapshot(snap.id).unwrap();
        assert!(db.list_case_snapshots(c.id).unwrap().is_empty());
    }

    #[test]
    fn notes_and_bookmark_round_trip() {
        let db = open_mem();
        let c = db.create_case("Notes").unwrap();
        let e = db
            .upsert_entity(c.id, "Person", None, "Alice", None, None, None, None)
            .unwrap();
        db.update_entity_notes(&e.id, Some("of interest")).unwrap();
        db.update_entity_bookmark(&e.id, true).unwrap();
        let reloaded = db.get_entity(&e.id).unwrap().unwrap();
        assert_eq!(reloaded.notes.as_deref(), Some("of interest"));
        assert!(reloaded.bookmarked);
        // Clear the note.
        db.update_entity_notes(&e.id, None).unwrap();
        let cleared = db.get_entity(&e.id).unwrap().unwrap();
        assert!(cleared.notes.is_none());
    }
}
