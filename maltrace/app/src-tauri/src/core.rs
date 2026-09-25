//! Shared business logic, transport-agnostic.
//!
//! Both the Tauri command shells (`commands.rs`) and the axum HTTP handlers
//! (`http.rs`) call into the functions here so the desktop and web targets
//! cannot drift in behavior.

use crate::ai::{self, AiState, ChatInput, ChatReply};
use crate::db::{Annotation, AuditEvent, Case, CrossCaseMatch, Db, Edge, Entity, TransformRun};
use crate::files::{self, FilePayload};
use crate::machine::{MachineMeta, MachineRegistry, MachineRunResult};
use crate::ontology::{self, Kind, COLLECTION_THRESHOLD};
use crate::proxies::{ProxyConfig, ProxyStore};
use crate::ratelimit::{RateLimitConfig, RateLimiter};
use crate::secrets::{SecretInfo, SecretStore};
use crate::sidecar::{ProducedEvent, SidecarRegistry, SidecarStatus, TransformMeta};
use chrono::Utc;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

pub struct AppState {
    pub db: Mutex<Db>,
    pub sidecars: SidecarRegistry,
    pub machines: MachineRegistry,
    pub ai: AiState,
    pub secrets: SecretStore,
    /// Per-sidecar outbound network policy (proxy URL). Resolved at
    /// every `run_transform` call and applied as env vars on the
    /// sidecar subprocess.
    pub proxies: ProxyStore,
    /// Per-sidecar concurrency cap + min-interval policy. Guards every
    /// sidecar invocation so `Run all` / scheduled machines don't blow
    /// past keyed-API quotas.
    pub rate_limiter: RateLimiter,
    /// Token-based authentication store (`users.json`). Only consulted
    /// by the axum middleware on the server build; the Tauri command
    /// shells run un-authed (single-user desktop).
    pub users: crate::auth::UsersStore,
    /// Outbound webhook URLs (Slack/Discord/generic JSON). Scheduler
    /// firings with new entities + transform errors POST to every URL.
    pub webhooks: crate::webhooks::WebhookStore,
    /// Live-update event bus. Mutating core fns publish a
    /// `GraphChanged` event after success; the SSE handler in
    /// `http.rs` streams them to subscribed web clients so other
    /// users see updates without a page reload.
    pub event_bus: crate::event_bus::EventBus,
    /// Root path for per-case file storage. Same dir as the SQLite DB.
    pub data_dir: PathBuf,
}

pub type SharedState = Arc<AppState>;

/// Wire-format canonical-type record. Built-in entries (from
/// `ontology::ALL`) and user-defined ones (from `custom_entity_types`)
/// both land in the same vector so the frontend doesn't have to merge.
/// Owned strings let runtime entries coexist with static ones.
#[derive(Debug, Serialize)]
pub struct CanonicalTypeDto {
    pub id: String,
    pub label: String,
    pub kind: Kind,
    pub color: String,
    pub sf_inputs: Vec<String>,
    pub default_sf_event: String,
    /// True for user-defined types; lets the UI mark them in the dropdown
    /// and offer a delete affordance.
    pub custom: bool,
}

/// Wire-format result of seeding a sample graph: just the new entities
/// and edges, so the frontend can splice them in without a full reload.
#[derive(Debug, Serialize)]
pub struct SeedResult {
    pub entities: Vec<Entity>,
    pub edges: Vec<Edge>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TransformRunResult {
    pub source_entity: Entity,
    pub new_entities: Vec<Entity>,
    pub new_edges: Vec<Edge>,
    pub transform_id: String,
    pub skipped: Vec<String>,
    /// Entities that the caller should drop from its local graph state
    /// (e.g. a Collection that was just expanded into its children).
    /// Empty for normal transform runs.
    #[serde(default)]
    pub removed_entity_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct AddEntityInput {
    pub case_id: i64,
    pub entity_type: String,
    pub label: String,
    #[serde(default)]
    pub pos_x: Option<f64>,
    #[serde(default)]
    pub pos_y: Option<f64>,
}

#[derive(Debug, Deserialize)]
pub struct PositionInput {
    pub x: f64,
    pub y: f64,
}

pub fn current_case(state: &AppState) -> Result<Case, String> {
    state.db.lock().default_case().map_err(stringify)
}

/// Per-sidecar load status from the last transform-listing pass. Thin wrapper
/// over the registry so the Tauri command and any other frontend share it.
pub async fn sidecar_status(state: &AppState) -> Vec<SidecarStatus> {
    // The status table is only filled as a side effect of a full sidecar
    // scan. On a cold start `list_transforms` may not have run yet (or may
    // be racing this call), which would make us report an empty list and
    // silently miss failed sidecars. Warm the catalogue first when empty so
    // the result always reflects a real scan, regardless of call ordering.
    if state.sidecars.sidecar_status().is_empty() {
        let _ = state.sidecars.list_all_transforms().await;
    }
    state.sidecars.sidecar_status()
}

pub fn list_cases(state: &AppState) -> Result<Vec<Case>, String> {
    state.db.lock().list_cases().map_err(stringify)
}

pub fn create_case(state: &AppState, name: &str) -> Result<Case, String> {
    state.db.lock().create_case(name).map_err(stringify)
}

/// Seed an existing case with a small example graph so a new user can
/// see what entities, edges, and pivots look like without having to set
/// up sidecars first. Idempotent in the *de-dup* sense — runs against
/// the existing `(case_id, entity_type, label)` upsert, so calling it
/// twice doesn't create duplicates.
///
/// Returns the freshly-touched entities + edges so the frontend can
/// splice them into local state without a full reload.
pub fn seed_sample_graph(
    state: &AppState,
    case_id: i64,
) -> Result<(Vec<Entity>, Vec<Edge>), String> {
    let db = state.db.lock();
    // The sample is a tiny "what you'd find investigating a domain" graph:
    // a Domain root with WHOIS + a couple of resolved IPs + the org that
    // owns them. Picked so every node has a meaningfully different colour
    // (Domain green, IP amber, WHOIS slate, Organization pink, …).
    let nodes: &[(&str, &str, Option<&str>, &str, (f64, f64))] = &[
        // (canonical, label, source_event_type, payload_json, (x, y))
        ("Domain", "example.com", Some("DOMAIN_NAME"), "{}", (0.0, 0.0)),
        (
            "WhoisRecord",
            "Registrar: ICANN",
            Some("DOMAIN_REGISTRAR"),
            "{\"full_data\":\"Sample WHOIS — registrar entry, created 1995-08-14.\"}",
            (-180.0, -120.0),
        ),
        (
            "IpAddress",
            "93.184.216.34",
            Some("IP_ADDRESS"),
            "{\"full_data\":\"Sample IPv4 — A record.\"}",
            (180.0, -120.0),
        ),
        (
            "IpAddress",
            "2606:2800:220:1:248:1893:25c8:1946",
            Some("IPV6_ADDRESS"),
            "{\"full_data\":\"Sample IPv6 — AAAA record.\"}",
            (250.0, 60.0),
        ),
        (
            "Organization",
            "Internet Assigned Numbers Authority",
            Some("COMPANY_NAME"),
            "{\"full_data\":\"Sample IANA — owner of the example.com block.\"}",
            (220.0, 180.0),
        ),
        (
            "EmailAddress",
            "abuse@iana.org",
            Some("EMAILADDR"),
            "{\"full_data\":\"Sample abuse contact harvested from the WHOIS record.\"}",
            (-120.0, 180.0),
        ),
    ];
    let mut created = Vec::with_capacity(nodes.len());
    for (canonical, label, sf, data, pos) in nodes {
        let e = db
            .upsert_entity(case_id, canonical, *sf, label, Some(data), Some(*pos), None, None)
            .map_err(stringify)?;
        created.push(e);
    }
    // Walk the freshly-inserted (or already-existing) nodes by index so
    // edges line up with the layout above.
    let edges: &[(usize, usize, &str)] = &[
        (0, 1, "sample"),
        (0, 2, "sample"),
        (0, 3, "sample"),
        (2, 4, "sample"),
        (3, 4, "sample"),
        (1, 5, "sample"),
    ];
    let mut new_edges = Vec::with_capacity(edges.len());
    for (s_idx, t_idx, label) in edges {
        let s = &created[*s_idx];
        let t = &created[*t_idx];
        let edge = db
            .upsert_edge(case_id, &s.id, &t.id, Some(label), None)
            .map_err(stringify)?;
        new_edges.push(edge);
    }
    Ok((created, new_edges))
}

pub fn rename_case(state: &AppState, id: i64, name: &str) -> Result<(), String> {
    state.db.lock().rename_case(id, name).map_err(stringify)
}

pub fn delete_case(state: &AppState, id: i64) -> Result<(), String> {
    state.db.lock().delete_case(id).map_err(stringify)?;
    files::try_wipe_case(&state.data_dir, id);
    Ok(())
}

/// Render a case as a GraphML document. GraphML is the closest thing to
/// a lingua-franca for graph interchange — Gephi, Cytoscape Desktop,
/// Maltego, NetworkX, yEd all read it. We emit only the bits anyone
/// reading us cares about: id, label, entity_type on nodes; label on
/// edges. Position is included as `x`/`y` keys so the receiver can
/// preserve layout if they want.
pub fn export_case_graphml(state: &AppState, case_id: i64) -> Result<String, String> {
    let db = state.db.lock();
    let entities = db.list_entities(case_id).map_err(stringify)?;
    let edges = db.list_edges(case_id).map_err(stringify)?;
    drop(db);

    let mut out = String::with_capacity(entities.len() * 200);
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str("<graphml xmlns=\"http://graphml.graphdrawing.org/xmlns\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:schemaLocation=\"http://graphml.graphdrawing.org/xmlns http://graphml.graphdrawing.org/xmlns/1.1/graphml.xsd\">\n");
    // Attribute declarations.
    out.push_str("  <key id=\"label\" for=\"node\" attr.name=\"label\" attr.type=\"string\"/>\n");
    out.push_str("  <key id=\"type\" for=\"node\" attr.name=\"type\" attr.type=\"string\"/>\n");
    out.push_str("  <key id=\"source_event_type\" for=\"node\" attr.name=\"source_event_type\" attr.type=\"string\"/>\n");
    out.push_str("  <key id=\"x\" for=\"node\" attr.name=\"x\" attr.type=\"double\"/>\n");
    out.push_str("  <key id=\"y\" for=\"node\" attr.name=\"y\" attr.type=\"double\"/>\n");
    out.push_str("  <key id=\"edge_label\" for=\"edge\" attr.name=\"label\" attr.type=\"string\"/>\n");
    out.push_str(&format!(
        "  <graph id=\"case-{}\" edgedefault=\"directed\">\n",
        case_id
    ));
    for e in &entities {
        out.push_str(&format!("    <node id=\"{}\">\n", xml_escape(&e.id)));
        out.push_str(&format!(
            "      <data key=\"label\">{}</data>\n",
            xml_escape(&e.label)
        ));
        out.push_str(&format!(
            "      <data key=\"type\">{}</data>\n",
            xml_escape(&e.entity_type)
        ));
        if let Some(ref s) = e.source_event_type {
            out.push_str(&format!(
                "      <data key=\"source_event_type\">{}</data>\n",
                xml_escape(s)
            ));
        }
        if let Some(x) = e.pos_x {
            out.push_str(&format!("      <data key=\"x\">{}</data>\n", x));
        }
        if let Some(y) = e.pos_y {
            out.push_str(&format!("      <data key=\"y\">{}</data>\n", y));
        }
        out.push_str("    </node>\n");
    }
    for edge in &edges {
        out.push_str(&format!(
            "    <edge id=\"{}\" source=\"{}\" target=\"{}\">\n",
            xml_escape(&edge.id),
            xml_escape(&edge.source_id),
            xml_escape(&edge.target_id),
        ));
        if let Some(ref l) = edge.label {
            out.push_str(&format!(
                "      <data key=\"edge_label\">{}</data>\n",
                xml_escape(l)
            ));
        }
        out.push_str("    </edge>\n");
    }
    out.push_str("  </graph>\n");
    out.push_str("</graphml>\n");
    Ok(out)
}

fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

pub fn list_entities(state: &AppState, case_id: i64) -> Result<Vec<Entity>, String> {
    state.db.lock().list_entities(case_id).map_err(stringify)
}

pub fn list_edges(state: &AppState, case_id: i64) -> Result<Vec<Edge>, String> {
    state.db.lock().list_edges(case_id).map_err(stringify)
}

pub fn list_canonical_types(state: &AppState) -> Vec<CanonicalTypeDto> {
    let mut out: Vec<CanonicalTypeDto> = ontology::ALL
        .iter()
        .map(|c| CanonicalTypeDto {
            id: c.id.to_string(),
            label: c.label.to_string(),
            kind: c.kind,
            color: c.color.to_string(),
            sf_inputs: ontology::sf_inputs_for(c.id)
                .into_iter()
                .map(String::from)
                .collect(),
            default_sf_event: ontology::default_sf_event_for(c.id),
            custom: false,
        })
        .collect();
    if let Ok(rows) = state.db.lock().list_custom_entity_types() {
        for r in rows {
            out.push(CanonicalTypeDto {
                id: r.id.clone(),
                label: r.label,
                // Map the stored string back to the Kind enum; unknown kinds
                // fall back to Generic so the UI still has a colour bucket.
                kind: parse_kind(&r.kind),
                color: r.color,
                sf_inputs: Vec::new(),
                default_sf_event: r.id,
                custom: true,
            });
        }
    }
    out
}

pub fn create_custom_entity_type(
    state: &AppState,
    label: &str,
    kind: &str,
    color: &str,
) -> Result<crate::db::CustomEntityType, String> {
    let label_trim = label.trim();
    if label_trim.is_empty() {
        return Err("entity type label cannot be empty".into());
    }
    // Slugify the label so the id is stable and SQL-safe.
    let id = slugify(label_trim);
    if id.is_empty() {
        return Err("entity type label must contain at least one alphanumeric character".into());
    }
    // Refuse collisions with built-in ids so a user can't accidentally
    // shadow `Domain`, `EmailAddress`, etc.
    if ontology::ALL.iter().any(|c| c.id.eq_ignore_ascii_case(&id)) {
        return Err(format!(
            "entity type id `{id}` collides with a built-in type; pick a different name"
        ));
    }
    let kind_ok = matches!(kind, "identity" | "asset" | "evidence" | "threat" | "generic");
    if !kind_ok {
        return Err(format!("unknown kind `{kind}` (expected identity/asset/evidence/threat/generic)"));
    }
    let color = if color.starts_with('#') && (color.len() == 7 || color.len() == 4) {
        color
    } else {
        // Fall back to a neutral slate when the colour is malformed —
        // safer than erroring out on a UX detail.
        "#64748b"
    };
    state
        .db
        .lock()
        .create_custom_entity_type(&id, label_trim, kind, color)
        .map_err(stringify)
}

pub fn delete_custom_entity_type(state: &AppState, id: &str) -> Result<(), String> {
    state.db.lock().delete_custom_entity_type(id).map_err(stringify)
}

fn slugify(s: &str) -> String {
    // PascalCase to match the built-in id convention (e.g. "Person",
    // "EmailAddress"). Splits on whitespace + non-alphanum, capitalises
    // each chunk's first ASCII letter, drops anything else.
    let mut out = String::with_capacity(s.len());
    let mut next_upper = true;
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            if next_upper {
                for u in c.to_uppercase() {
                    out.push(u);
                }
            } else {
                out.push(c);
            }
            next_upper = false;
        } else {
            next_upper = true;
        }
    }
    out
}

fn parse_kind(s: &str) -> Kind {
    match s {
        "identity" => Kind::Identity,
        "asset" => Kind::Asset,
        "evidence" => Kind::Evidence,
        "threat" => Kind::Threat,
        _ => Kind::Generic,
    }
}

pub fn add_entity(
    state: &AppState,
    input: AddEntityInput,
    actor: Option<&str>,
) -> Result<Entity, String> {
    let pos = match (input.pos_x, input.pos_y) {
        (Some(x), Some(y)) => Some((x, y)),
        _ => None,
    };
    // If the caller hands us something already in the canonical table, take it
    // as-is. Otherwise treat it as a SpiderFoot event type and resolve.
    let canonical = if ontology::canonical_for(&input.entity_type) == "Generic"
        && ontology::ALL.iter().any(|c| c.id == input.entity_type)
    {
        input.entity_type.clone()
    } else {
        ontology::canonical_for(&input.entity_type).to_string()
    };
    let source = if canonical != input.entity_type {
        Some(input.entity_type.as_str())
    } else {
        None
    };
    let case_id = input.case_id;
    let db = state.db.lock();
    db.ensure_case_writable(case_id).map_err(stringify)?;
    let result =
        db.upsert_entity(case_id, &canonical, source, &input.label, None, pos, None, actor)
            .map_err(stringify);
    drop(db);
    if result.is_ok() {
        crate::event_bus::publish_graph_changed(&state.event_bus, case_id, "entity_added");
    }
    result
}

pub fn update_entity_position(
    state: &AppState,
    id: &str,
    x: f64,
    y: f64,
) -> Result<(), String> {
    let db = state.db.lock();
    db.ensure_entity_writable(id).map_err(stringify)?;
    db.update_entity_position(id, x, y).map_err(stringify)
}

pub fn update_entity_notes(
    state: &AppState,
    id: &str,
    notes: Option<&str>,
) -> Result<(), String> {
    // Treat empty/whitespace as "clear notes" — saves the inspector from
    // having to special-case the textarea-cleared state.
    let trimmed = notes.map(str::trim).filter(|s| !s.is_empty());
    let db = state.db.lock();
    db.ensure_entity_writable(id).map_err(stringify)?;
    db.update_entity_notes(id, trimmed).map_err(stringify)
}

pub fn update_entity_bookmark(
    state: &AppState,
    id: &str,
    bookmarked: bool,
) -> Result<(), String> {
    let db = state.db.lock();
    db.ensure_entity_writable(id).map_err(stringify)?;
    db.update_entity_bookmark(id, bookmarked).map_err(stringify)
}

pub fn update_entity_label(
    state: &AppState,
    id: &str,
    new_label: &str,
    actor: Option<&str>,
) -> Result<(), String> {
    let db = state.db.lock();
    db.ensure_entity_writable(id).map_err(stringify)?;
    // Capture the old label for the audit payload before clobbering.
    let prev = db.get_entity(id).ok().flatten();
    db.update_entity_label(id, new_label).map_err(stringify)?;
    let case_id = prev.as_ref().map(|e| e.case_id);
    if let Some(e) = prev {
        let payload = serde_json::json!({
            "from": e.label,
            "to": new_label,
        });
        db.record_audit_event(
            e.case_id,
            "entity_renamed",
            Some(id),
            Some(&payload.to_string()),
            actor,
        );
    }
    drop(db);
    if let Some(cid) = case_id {
        crate::event_bus::publish_graph_changed(&state.event_bus, cid, "entity_renamed");
    }
    Ok(())
}

pub fn update_entity_locked(
    state: &AppState,
    id: &str,
    locked: bool,
) -> Result<(), String> {
    let db = state.db.lock();
    db.ensure_entity_writable(id).map_err(stringify)?;
    db.update_entity_locked(id, locked).map_err(stringify)
}

// ─── Case lock / evidence snapshot ──────────────────────────────
/// Toggle an investigation's evidence-lock. When locked, every mutator
/// refuses with a friendly error and the UI degrades into a read-only
/// view. Reads stay open so investigators can still export, browse,
/// chat with the AI panel, etc.
pub fn set_case_locked(
    state: &AppState,
    id: i64,
    locked: bool,
    actor: Option<&str>,
) -> Result<Case, String> {
    let db = state.db.lock();
    db.set_case_locked(id, locked).map_err(stringify)?;
    db.record_audit_event(
        id,
        if locked { "case_locked" } else { "case_unlocked" },
        None,
        None,
        actor,
    );
    let case = db
        .get_case(id)
        .map_err(stringify)?
        .ok_or_else(|| format!("case {id} not found after lock toggle"))?;
    drop(db);
    crate::event_bus::publish_graph_changed(
        &state.event_bus,
        id,
        if locked { "case_locked" } else { "case_unlocked" },
    );
    Ok(case)
}

// ─── Entity provenance ─────────────────────────────────────────
#[derive(Debug, Serialize)]
pub struct EdgeProvenance {
    /// Edge id (handy for cross-referencing the canvas).
    pub edge_id: String,
    /// Other end of the edge — the entity the inspected one was
    /// pivoted from (for incoming) or to (for outgoing).
    pub other_entity_id: String,
    pub other_entity_label: String,
    pub other_entity_type: String,
    /// Direction relative to the inspected entity.
    pub direction: &'static str, // "incoming" | "outgoing"
    pub transform_id: Option<String>,
    pub run_id: Option<i64>,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct EntityProvenance {
    /// The originating transform run, when the entity came from a
    /// sidecar invocation. `None` for manually-added / imported /
    /// seed entities.
    pub run: Option<TransformRun>,
    pub transform_id: Option<String>,
    pub source_url: Option<String>,
    pub created_at: String,
    /// Every edge touching the inspected entity, with the produce-run
    /// id and the other endpoint's metadata. Surfaced in the inspector
    /// so the investigator can answer "where did this node come from"
    /// and "what did it lead to" in a single glance.
    pub incoming: Vec<EdgeProvenance>,
    pub outgoing: Vec<EdgeProvenance>,
}

pub fn entity_provenance(state: &AppState, entity_id: &str) -> Result<EntityProvenance, String> {
    let db = state.db.lock();
    let entity = db
        .get_entity(entity_id)
        .map_err(stringify)?
        .ok_or_else(|| format!("entity {entity_id} not found"))?;

    // Resolve the originating run when the entity has one.
    let run = match entity.created_by_run_id {
        Some(rid) => db.get_transform_run(rid).map_err(stringify)?,
        None => None,
    };

    // Walk every edge in the case once; building two small lists is
    // cheaper than two separate filtered queries given the case-edges
    // table is already in cache from the graph render.
    let edges = db.list_edges(entity.case_id).map_err(stringify)?;
    let mut incoming = Vec::new();
    let mut outgoing = Vec::new();
    let mut other_ids = std::collections::HashSet::new();
    for e in &edges {
        if e.target_id == entity_id {
            other_ids.insert(e.source_id.clone());
        } else if e.source_id == entity_id {
            other_ids.insert(e.target_id.clone());
        }
    }
    // Cache other-endpoint lookups so the loop doesn't fan out to N
    // single-row queries.
    let mut others: HashMap<String, (String, String)> = HashMap::new();
    for id in &other_ids {
        if let Ok(Some(e)) = db.get_entity(id) {
            others.insert(id.clone(), (e.label, e.entity_type));
        }
    }
    for e in edges {
        let is_in = e.target_id == entity_id;
        let is_out = e.source_id == entity_id;
        if !is_in && !is_out {
            continue;
        }
        let other_id = if is_in { &e.source_id } else { &e.target_id };
        let (label, ty) = others
            .get(other_id)
            .cloned()
            .unwrap_or_else(|| ("(missing)".to_string(), "Generic".to_string()));
        let dto = EdgeProvenance {
            edge_id: e.id,
            other_entity_id: other_id.clone(),
            other_entity_label: label,
            other_entity_type: ty,
            direction: if is_in { "incoming" } else { "outgoing" },
            transform_id: e.label,
            run_id: e.run_id,
            created_at: e.created_at.to_rfc3339(),
        };
        if is_in {
            incoming.push(dto);
        } else {
            outgoing.push(dto);
        }
    }

    Ok(EntityProvenance {
        run,
        transform_id: entity.created_by_transform_id,
        source_url: entity.source_url,
        created_at: entity.created_at.to_rfc3339(),
        incoming,
        outgoing,
    })
}

// ─── Edge provenance ───────────────────────────────────────────
#[derive(Debug, Serialize)]
pub struct EdgeDetail {
    pub edge: Edge,
    pub source_label: Option<String>,
    pub source_type: Option<String>,
    pub target_label: Option<String>,
    pub target_type: Option<String>,
    /// The full run record this edge was produced by, when present.
    pub run: Option<TransformRun>,
}

pub fn edge_detail(state: &AppState, edge_id: &str) -> Result<EdgeDetail, String> {
    let db = state.db.lock();
    let edge = db
        .get_edge(edge_id)
        .map_err(stringify)?
        .ok_or_else(|| format!("edge {edge_id} not found"))?;
    let source = db.get_entity(&edge.source_id).map_err(stringify)?;
    let target = db.get_entity(&edge.target_id).map_err(stringify)?;
    let run = match edge.run_id {
        Some(rid) => db.get_transform_run(rid).map_err(stringify)?,
        None => None,
    };
    Ok(EdgeDetail {
        source_label: source.as_ref().map(|e| e.label.clone()),
        source_type: source.as_ref().map(|e| e.entity_type.clone()),
        target_label: target.as_ref().map(|e| e.label.clone()),
        target_type: target.as_ref().map(|e| e.entity_type.clone()),
        run,
        edge,
    })
}

pub fn list_bookmarked(state: &AppState, case_id: i64) -> Result<Vec<Entity>, String> {
    state.db.lock().list_bookmarked(case_id).map_err(stringify)
}

pub fn list_transform_runs_for_entity(
    state: &AppState,
    entity_id: &str,
    limit: i64,
) -> Result<Vec<TransformRun>, String> {
    state
        .db
        .lock()
        .list_transform_runs_for_entity(entity_id, limit.clamp(1, 200))
        .map_err(stringify)
}

// ─── Annotations ────────────────────────────────────────────────
pub fn list_annotations(state: &AppState, case_id: i64) -> Result<Vec<Annotation>, String> {
    state.db.lock().list_annotations(case_id).map_err(stringify)
}

pub fn create_annotation(
    state: &AppState,
    case_id: i64,
    text: &str,
    pos_x: f64,
    pos_y: f64,
    color: Option<&str>,
) -> Result<Annotation, String> {
    let color = color.filter(|c| !c.is_empty()).unwrap_or("#fbbf24");
    let db = state.db.lock();
    db.ensure_case_writable(case_id).map_err(stringify)?;
    db.create_annotation(case_id, text, pos_x, pos_y, color)
        .map_err(stringify)
}

pub fn update_annotation(
    state: &AppState,
    id: &str,
    text: Option<&str>,
    pos_x: Option<f64>,
    pos_y: Option<f64>,
    color: Option<&str>,
) -> Result<(), String> {
    // Annotation updates: positions move freely even when locked (so
    // users can rearrange a snapshot's commentary), but text/color
    // edits are blocked. The simplest correct cut: block text+color,
    // allow position-only.
    if text.is_some() || color.is_some() {
        let db = state.db.lock();
        // Pull the annotation's case id via a sub-query to avoid
        // exposing a new helper for one call site.
        let case_id: Option<i64> = db
            .list_annotations_case_id(id)
            .map_err(stringify)?;
        if let Some(cid) = case_id {
            db.ensure_case_writable(cid).map_err(stringify)?;
        }
    }
    state
        .db
        .lock()
        .update_annotation(id, text, pos_x, pos_y, color)
        .map_err(stringify)
}

pub fn delete_annotation(state: &AppState, id: &str) -> Result<(), String> {
    {
        let db = state.db.lock();
        if let Some(cid) = db.list_annotations_case_id(id).map_err(stringify)? {
            db.ensure_case_writable(cid).map_err(stringify)?;
        }
    }
    state.db.lock().delete_annotation(id).map_err(stringify)
}

// ─── Entity merge ──────────────────────────────────────────────
pub fn search_entities_across_cases(
    state: &AppState,
    query: &str,
    limit: i64,
) -> Result<Vec<CrossCaseMatch>, String> {
    state
        .db
        .lock()
        .search_entities_across_cases(query, limit)
        .map_err(stringify)
}

pub fn list_audit_events(
    state: &AppState,
    case_id: i64,
    limit: i64,
) -> Result<Vec<AuditEvent>, String> {
    state
        .db
        .lock()
        .list_audit_events(case_id, limit)
        .map_err(stringify)
}

pub fn merge_entities(
    state: &AppState,
    keep_id: &str,
    drop_id: &str,
    actor: Option<&str>,
) -> Result<(), String> {
    let db = state.db.lock();
    db.ensure_entity_writable(keep_id).map_err(stringify)?;
    // Snapshot labels for the audit payload before the merge erases them.
    let drop_label = db
        .get_entity(drop_id)
        .ok()
        .flatten()
        .map(|e| e.label)
        .unwrap_or_default();
    let keep = db
        .get_entity(keep_id)
        .ok()
        .flatten()
        .ok_or_else(|| format!("keep entity {keep_id} not found"))?;
    db.merge_entities(keep_id, drop_id).map_err(stringify)?;
    let payload = serde_json::json!({
        "keep_id": keep_id,
        "keep_label": keep.label,
        "drop_id": drop_id,
        "drop_label": drop_label,
    });
    db.record_audit_event(
        keep.case_id,
        "entity_merged",
        Some(keep_id),
        Some(&payload.to_string()),
        actor,
    );
    let case_id = keep.case_id;
    drop(db);
    crate::event_bus::publish_graph_changed(&state.event_bus, case_id, "entity_merged");
    Ok(())
}

pub fn delete_entity(state: &AppState, id: &str, actor: Option<&str>) -> Result<(), String> {
    {
        let db = state.db.lock();
        db.ensure_entity_writable(id).map_err(stringify)?;
    }
    // Snapshot identity for the audit row before the entity vanishes.
    let snap = {
        let db = state.db.lock();
        db.get_entity(id).ok().flatten()
    };
    // If this entity owns a file on disk, scrub it too so we don't leak
    // bytes after the row goes away. Best-effort: stale references are
    // tolerated.
    let payload = snap.as_ref().and_then(|e| parse_file_payload(e));
    state.db.lock().delete_entity(id).map_err(stringify)?;
    if let Some(p) = payload {
        files::try_delete(std::path::Path::new(&p.file_path));
    }
    if let Some(e) = snap {
        let payload = serde_json::json!({
            "label": e.label,
            "entity_type": e.entity_type,
        });
        state.db.lock().record_audit_event(
            e.case_id,
            "entity_deleted",
            Some(id),
            Some(&payload.to_string()),
            actor,
        );
        crate::event_bus::publish_graph_changed(
            &state.event_bus,
            e.case_id,
            "entity_deleted",
        );
    }
    Ok(())
}

pub async fn list_transforms(state: &AppState) -> Result<Vec<TransformMeta>, String> {
    state.sidecars.list_all_transforms().await.map_err(stringify)
}

pub fn list_plugins(state: &AppState) -> Vec<crate::manifest::PluginSummary> {
    state.sidecars.list_plugins()
}

// ─── File upload / preview ──────────────────────────────────────

/// Upload a file's bytes into the case file store and create a matching
/// Image (for image MIMEs) or File entity. `bytes_base64` is what the
/// browser/Tauri side sends after `FileReader.readAsDataURL`; the
/// `data:` prefix is allowed and stripped server-side.
pub fn upload_file_to_case(
    state: &AppState,
    case_id: i64,
    file_name: &str,
    bytes_base64: &str,
    actor: Option<&str>,
) -> Result<Entity, String> {
    let trimmed_name = file_name.trim();
    if trimmed_name.is_empty() {
        return Err("file name cannot be empty".into());
    }
    let bytes = files::decode_base64(bytes_base64).map_err(stringify)?;
    let stored = files::write_file(&state.data_dir, case_id, trimmed_name, &bytes)
        .map_err(stringify)?;
    let payload = FilePayload {
        file_path: stored.path.to_string_lossy().into_owned(),
        original_name: trimmed_name.to_string(),
        mime_type: stored.mime_type.clone(),
        size_bytes: stored.size_bytes,
    };
    let (canonical, source_event) = if files::is_image(&stored.mime_type) {
        ("Image", Some("IMAGE"))
    } else {
        ("File", Some("RAW_FILE_META_DATA"))
    };
    let data_json = serde_json::to_string(&payload).map_err(stringify)?;
    let db = state.db.lock();
    db.ensure_case_writable(case_id).map_err(stringify)?;
    db.upsert_entity(
        case_id,
        canonical,
        source_event,
        trimmed_name,
        Some(&data_json),
        None,
        None,
        actor,
    )
    .map_err(stringify)
}

/// Read an Image/File entity's bytes from disk and return a data URL the
/// frontend can drop into `<img src>`. Reads `data.file_path` if set,
/// falls back to the entity's `label` (so a manually-typed image path
/// still previews). Capped at MAX_PREVIEW_BYTES upstream.
pub fn read_entity_file_as_data_url(state: &AppState, entity_id: &str) -> Result<String, String> {
    let entity = state
        .db
        .lock()
        .get_entity(entity_id)
        .map_err(stringify)?
        .ok_or_else(|| format!("entity {entity_id} not found"))?;
    let payload = parse_file_payload(&entity);
    let (path, mime) = match payload {
        Some(p) => (PathBuf::from(p.file_path), p.mime_type),
        None => {
            // Manual entity: label IS the path. Sniff MIME by extension.
            let path = PathBuf::from(&entity.label);
            let ext = path
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            let mime = match ext.as_str() {
                "jpg" | "jpeg" => "image/jpeg",
                "png" => "image/png",
                "gif" => "image/gif",
                "webp" => "image/webp",
                "bmp" => "image/bmp",
                _ => "application/octet-stream",
            }
            .to_string();
            (path, mime)
        }
    };
    files::read_as_data_url(&path, &mime).map_err(stringify)
}

fn parse_file_payload(entity: &Entity) -> Option<FilePayload> {
    let raw = entity.data.as_deref()?;
    serde_json::from_str::<FilePayload>(raw).ok()
}

/// Anthropic's vision API accepts only these media types. Map our stored
/// mime onto a supported one, or `None` if the image can't be sent as-is.
fn vision_media_type(mime: &str) -> Option<&'static str> {
    match mime {
        "image/jpeg" | "image/jpg" => Some("image/jpeg"),
        "image/png" => Some("image/png"),
        "image/gif" => Some("image/gif"),
        "image/webp" => Some("image/webp"),
        _ => None,
    }
}

/// Send an Image entity to Claude's vision model for OSINT analysis and
/// return the analysis text. `instruction` overrides the default OSINT
/// prompt when non-empty. When `save_to_notes` is true the result is also
/// appended to the entity's notes so it persists with the case.
pub async fn analyze_image_entity(
    state: &AppState,
    entity_id: &str,
    instruction: &str,
    save_to_notes: bool,
    actor: Option<&str>,
) -> Result<String, String> {
    let entity = state
        .db
        .lock()
        .get_entity(entity_id)
        .map_err(stringify)?
        .ok_or_else(|| format!("entity {entity_id} not found"))?;

    // Resolve the on-disk path + mime (uploaded entity → payload; manual
    // entity → label is the path, sniff by extension).
    let (path, mime) = match parse_file_payload(&entity) {
        Some(p) => (std::path::PathBuf::from(p.file_path), p.mime_type),
        None => {
            let path = std::path::PathBuf::from(&entity.label);
            let mime = files::mime_for_extension(&path).to_string();
            (path, mime)
        }
    };

    let media_type = vision_media_type(&mime).ok_or_else(|| {
        format!("'{mime}' isn't a vision-supported image (use JPEG, PNG, GIF, or WebP)")
    })?;

    let image_b64 = files::read_as_base64(&path).map_err(stringify)?;

    let instruction = if instruction.trim().is_empty() {
        ai::DEFAULT_IMAGE_PROMPT
    } else {
        instruction.trim()
    };

    // OPSEC audit: record that this image was sent off-device to a third
    // party (Anthropic) BEFORE the call, so the trail exists even if the
    // request later errors mid-flight. This is evidence leaving the machine —
    // the investigator must be able to see it happened, when, and to where.
    {
        let payload = serde_json::json!({
            "destination": "anthropic_vision_api",
            "model": state.ai.model,
            "image_label": entity.label,
            "bytes": image_b64.len(),
        })
        .to_string();
        state.db.lock().record_audit_event(
            entity.case_id,
            "image_sent_to_ai",
            Some(entity_id),
            Some(&payload),
            actor,
        );
    }

    let ctx = format!("image entity \"{}\" in this case", entity.label);
    let reply = ai::analyze_image(&state.ai, &image_b64, media_type, instruction, Some(&ctx))
        .await
        .map_err(stringify)?;

    if save_to_notes {
        let existing = entity.notes.clone().unwrap_or_default();
        let combined = if existing.trim().is_empty() {
            format!("AI image analysis:\n{}", reply.message)
        } else {
            format!("{existing}\n\n--- AI image analysis ---\n{}", reply.message)
        };
        let _ = update_entity_notes(state, entity_id, Some(&combined));
    }

    Ok(reply.message)
}

pub async fn run_transform(
    state: &AppState,
    entity_id: &str,
    transform_id: &str,
) -> Result<TransformRunResult, String> {
    run_transform_with_params(state, entity_id, transform_id, None, None).await
}

/// `run_transform` variant that accepts a JSON-encoded parameters map
/// captured from the parameter-form dialog. Threads `MALTRACE_PARAMS`
/// through to the sidecar subprocess. `actor` is stamped on the
/// transform_runs row for the audit trail; `None` lets the row carry
/// a NULL (pre-v10 / scheduler-initiated runs).
pub async fn run_transform_with_params(
    state: &AppState,
    entity_id: &str,
    transform_id: &str,
    params_json: Option<String>,
    actor: Option<&str>,
) -> Result<TransformRunResult, String> {
    let source = {
        let db = state.db.lock();
        // Locked case: refuse before doing any work. Reads of the entity
        // are fine, but a transform run mutates the graph.
        db.ensure_entity_writable(entity_id).map_err(stringify)?;
        db.get_entity(entity_id)
            .map_err(stringify)?
            .ok_or_else(|| format!("entity {entity_id} not found"))?
    };

    // Pick the input event type the *target transform* accepts. A canonical
    // type (e.g. Domain) maps to several SF events, and modules are picky:
    // sfp_dnsresolve wants INTERNET_NAME, sfp_whois wants DOMAIN_NAME. Sending
    // the wrong one yields zero events. `consumes_for` reads the warmed meta
    // cache; warm it on a miss so a cold start still resolves correctly.
    let consumes = match state.sidecars.consumes_for(transform_id) {
        Some(c) => c,
        None => {
            let _ = state.sidecars.list_all_transforms().await;
            state.sidecars.consumes_for(transform_id).unwrap_or_default()
        }
    };
    let sf_input_type = ontology::input_event_for_transform(
        &source.entity_type,
        source.source_event_type.as_deref(),
        &consumes,
    );

    // For Image/File entities the `label` is the user-facing original
    // filename, not a path the sidecar can open. Substitute the real
    // on-disk path from `entity.data.file_path` when present so
    // face_detect & friends receive an openable path.
    let payload_for_sidecar = parse_file_payload(&source)
        .map(|p| p.file_path)
        .unwrap_or_else(|| source.label.clone());

    // Pull every stored secret and pass them as --opts JSON. Each sidecar
    // takes what it needs; unknown keys are harmless. We send `{}` instead
    // of nothing when the store is empty so the call shape stays uniform.
    let secrets_snapshot = state.secrets.snapshot_for_sidecar();
    let opts_json = serde_json::to_string(&secrets_snapshot).unwrap_or_else(|_| "{}".into());

    let started_at = Utc::now();
    let started_instant = std::time::Instant::now();

    // Create the run row up-front so we have an id to stamp onto every
    // produced entity and edge. Without this, provenance is the empty
    // string and the inspector can't trace anything back.
    let run_id = {
        let db = state.db.lock();
        db.start_transform_run(
            source.case_id,
            transform_id,
            Some(&source.id),
            Some(&source.label),
            started_at,
            actor,
        )
        .map_err(stringify)?
    };

    // Resolve the proxy URL for the sidecar that owns this transform.
    // Warm the transform index if cold so we don't end up applying the
    // global default to a sidecar that has a per-sidecar override.
    let mut sidecar_id = state.sidecars.sidecar_for_transform(transform_id);
    if sidecar_id.is_none() {
        let _ = state.sidecars.list_all_transforms().await;
        sidecar_id = state.sidecars.sidecar_for_transform(transform_id);
    }
    let proxy = match sidecar_id.as_deref() {
        Some(id) => state.proxies.resolve(id),
        None => state.proxies.resolve(""),
    };

    let sidecar_result = state
        .sidecars
        .run_transform(
            transform_id,
            &sf_input_type,
            &payload_for_sidecar,
            &opts_json,
            params_json.as_deref(),
            proxy.as_deref(),
            &state.rate_limiter,
        )
        .await;

    let duration_ms = started_instant.elapsed().as_millis() as i64;

    match sidecar_result {
        Ok(output) => {
            let db = state.db.lock();
            let result = persist_transform_output(
                &db,
                &source,
                transform_id,
                run_id,
                actor,
                &output.events,
            )
            .map_err(stringify)?;
            let new_count = result.new_entities.len() as i64;
            let stderr_excerpt = truncate(&output.stderr, 4000);
            let _ = db.complete_transform_run(
                run_id,
                duration_ms,
                "ok",
                new_count,
                None,
                if stderr_excerpt.is_empty() {
                    None
                } else {
                    Some(&stderr_excerpt)
                },
            );
            drop(db);
            if new_count > 0 || !result.new_edges.is_empty() {
                crate::event_bus::publish_graph_changed(
                    &state.event_bus,
                    source.case_id,
                    "transform_run",
                );
            }
            Ok(result)
        }
        Err(e) => {
            let err_msg = e.to_string();
            let (msg, stderr) = split_err_and_stderr(&err_msg);
            let db = state.db.lock();
            let _ = db.complete_transform_run(
                run_id,
                duration_ms,
                "error",
                0,
                Some(&msg),
                if stderr.is_empty() { None } else { Some(&stderr) },
            );
            Err(err_msg)
        }
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push_str("…[truncated]");
    out
}

/// Split a sidecar error message into (short_message, stderr_excerpt).
/// `sidecar.rs::run_transform` formats its Err as
/// `"...message...\n--stderr--\n...stderr..."`; we tease them apart so the
/// log table stores them in their own columns.
fn split_err_and_stderr(err: &str) -> (String, String) {
    if let Some(idx) = err.find("\n--stderr--\n") {
        let msg = err[..idx].to_string();
        let stderr = err[idx + "\n--stderr--\n".len()..].to_string();
        (msg, stderr)
    } else {
        (err.to_string(), String::new())
    }
}

// ─── Secrets ────────────────────────────────────────────────────
pub fn list_secret_names(state: &AppState) -> Vec<SecretInfo> {
    state.secrets.list_names()
}

pub fn set_secret(state: &AppState, name: &str, value: &str) -> Result<(), String> {
    state.secrets.set(name, value).map_err(stringify)
}

pub fn delete_secret(state: &AppState, name: &str) -> Result<(), String> {
    state.secrets.delete(name).map_err(stringify)
}

// ─── Transform-run log ──────────────────────────────────────────
pub fn list_transform_runs(
    state: &AppState,
    case_id: i64,
    limit: i64,
) -> Result<Vec<TransformRun>, String> {
    state
        .db
        .lock()
        .list_transform_runs(case_id, limit.clamp(1, 500))
        .map_err(stringify)
}

// ─── STIX 2.1 export ───────────────────────────────────────────
pub fn export_case_stix(state: &AppState, case_id: i64) -> Result<String, String> {
    let db = state.db.lock();
    let case = db
        .get_case(case_id)
        .map_err(stringify)?
        .ok_or_else(|| format!("case {case_id} not found"))?;
    let entities = db.list_entities(case_id).map_err(stringify)?;
    let edges = db.list_edges(case_id).map_err(stringify)?;
    drop(db);
    let bundle = crate::stix::build_bundle(&case.name, &entities, &edges);
    serde_json::to_string_pretty(&bundle).map_err(stringify)
}

// ─── Templated HTML report ─────────────────────────────────────
pub fn export_case_report_html(state: &AppState, case_id: i64) -> Result<String, String> {
    let db = state.db.lock();
    let case = db
        .get_case(case_id)
        .map_err(stringify)?
        .ok_or_else(|| format!("case {case_id} not found"))?;
    let entities = db.list_entities(case_id).map_err(stringify)?;
    let runs = db.list_transform_runs(case_id, 500).map_err(stringify)?;
    drop(db);
    Ok(crate::report::build_html(&case, &entities, &runs))
}

// ─── GraphML import ────────────────────────────────────────────
/// Result of an import run, surfaced for the toast/log on completion.
#[derive(Debug, Serialize)]
pub struct ImportResult {
    pub case_id: i64,
    pub new_entity_count: usize,
    pub new_edge_count: usize,
    pub skipped: Vec<String>,
}

/// Parse a GraphML document and ingest every `<node>` + `<edge>` into
/// the target case. Idempotent against the existing
/// `(case_id, entity_type, label)` upsert — re-importing the same file
/// produces no duplicates.
pub fn import_case_graphml(
    state: &AppState,
    case_id: i64,
    xml: &str,
    actor: Option<&str>,
) -> Result<ImportResult, String> {
    let parsed = parse_graphml(xml).map_err(stringify)?;
    let db = state.db.lock();
    db.ensure_case_writable(case_id).map_err(stringify)?;

    let mut id_map: HashMap<String, String> = HashMap::with_capacity(parsed.nodes.len());
    let mut new_entities = 0usize;
    let mut skipped = Vec::new();

    for n in &parsed.nodes {
        let entity_type = n
            .type_str
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or("Generic")
            .to_string();
        let canonical = if ontology::ALL.iter().any(|c| c.id == entity_type) {
            entity_type.clone()
        } else {
            ontology::canonical_for(&entity_type).to_string()
        };
        let label = n.label.clone().filter(|s| !s.is_empty()).unwrap_or_else(|| n.id.clone());
        let pos = match (n.x, n.y) {
            (Some(x), Some(y)) => Some((x, y)),
            _ => None,
        };
        match db.upsert_entity(
            case_id,
            &canonical,
            n.source_event_type.as_deref(),
            &label,
            None,
            pos,
            None,
            actor,
        ) {
            Ok(e) => {
                id_map.insert(n.id.clone(), e.id);
                new_entities += 1;
            }
            Err(e) => skipped.push(format!("node {}: {}", n.id, e)),
        }
    }

    let mut new_edges = 0usize;
    for e in &parsed.edges {
        let (Some(src), Some(tgt)) = (id_map.get(&e.source), id_map.get(&e.target)) else {
            skipped.push(format!("edge {}→{}: endpoint missing", e.source, e.target));
            continue;
        };
        if let Err(err) = db.upsert_edge(case_id, src, tgt, e.label.as_deref(), None) {
            skipped.push(format!("edge {}→{}: {}", e.source, e.target, err));
            continue;
        }
        new_edges += 1;
    }

    Ok(ImportResult {
        case_id,
        new_entity_count: new_entities,
        new_edge_count: new_edges,
        skipped,
    })
}

#[derive(Default)]
struct ParsedGraphML {
    nodes: Vec<ParsedNode>,
    edges: Vec<ParsedEdge>,
}

#[derive(Default)]
struct ParsedNode {
    id: String,
    label: Option<String>,
    type_str: Option<String>,
    source_event_type: Option<String>,
    x: Option<f64>,
    y: Option<f64>,
}

struct ParsedEdge {
    source: String,
    target: String,
    label: Option<String>,
}

/// Minimal GraphML parser. We only consume what we emit (label, type,
/// source_event_type, x, y) — anything else is ignored. Hand-rolled to
/// avoid pulling in a full XML crate; GraphML is well-structured enough
/// that a tiny tag scanner suffices.
fn parse_graphml(xml: &str) -> anyhow::Result<ParsedGraphML> {
    use std::collections::HashMap as Map;
    // Map of key-id → attr.name so we can resolve `<data key="d2">` to
    // the GraphML <key id="d2" attr.name="label" ...> declaration.
    let mut key_names: Map<String, String> = Map::new();
    let mut nodes: Vec<ParsedNode> = Vec::new();
    let mut edges: Vec<ParsedEdge> = Vec::new();

    // A 2-state cursor: when inside <node>/<edge> we collect <data>
    // children until the closing tag. We hop tag boundaries via
    // simple substring search since GraphML is well-formed.
    let mut idx = 0usize;
    let bytes = xml.as_bytes();
    while idx < bytes.len() {
        // Find next tag.
        let Some(lt) = find_from(bytes, idx, b'<') else { break };
        let Some(gt) = find_from(bytes, lt, b'>') else { break };
        let tag = &xml[lt + 1..gt];
        idx = gt + 1;

        if tag.starts_with("key ") || tag.starts_with("key\t") {
            // <key id="d1" attr.name="label" .../>
            let id = attr(tag, "id").unwrap_or_default();
            let name = attr(tag, "attr.name").unwrap_or_default();
            if !id.is_empty() && !name.is_empty() {
                key_names.insert(id, name);
            }
            continue;
        }

        if tag.starts_with("node ") || tag == "node" {
            let id = attr(tag, "id").unwrap_or_default();
            // Scan forward to </node>, collecting `<data key="...">`.
            let mut node = ParsedNode {
                id: id.clone(),
                ..Default::default()
            };
            idx = collect_data_block(xml, idx, &key_names, "node", &mut |k, v| {
                match k {
                    "label" => node.label = Some(v.into()),
                    "type" => node.type_str = Some(v.into()),
                    "source_event_type" => node.source_event_type = Some(v.into()),
                    "x" => node.x = v.parse().ok(),
                    "y" => node.y = v.parse().ok(),
                    _ => {}
                }
            });
            nodes.push(node);
            continue;
        }

        if tag.starts_with("edge ") || tag == "edge" {
            let source = attr(tag, "source").unwrap_or_default();
            let target = attr(tag, "target").unwrap_or_default();
            if source.is_empty() || target.is_empty() {
                continue;
            }
            let mut label: Option<String> = None;
            idx = collect_data_block(xml, idx, &key_names, "edge", &mut |k, v| {
                if k == "label" || k == "edge_label" {
                    label = Some(v.into());
                }
            });
            edges.push(ParsedEdge { source, target, label });
            continue;
        }
    }

    Ok(ParsedGraphML { nodes, edges })
}

/// Walk `xml[from..]` collecting `<data key="K">V</data>` until the
/// matching `</element>` close tag. Returns the index just past
/// `</element>`.
fn collect_data_block(
    xml: &str,
    from: usize,
    key_names: &HashMap<String, String>,
    element: &str,
    sink: &mut dyn FnMut(&str, &str),
) -> usize {
    let bytes = xml.as_bytes();
    let close = format!("</{element}>");
    let end = xml[from..].find(&close).map(|i| from + i).unwrap_or(bytes.len());
    let mut i = from;
    while i < end {
        let Some(lt) = find_from(bytes, i, b'<') else { break };
        if lt >= end {
            break;
        }
        let Some(gt) = find_from(bytes, lt, b'>') else { break };
        let tag = &xml[lt + 1..gt];
        if tag.starts_with("data ") || tag == "data" {
            let key_id = attr(tag, "key").unwrap_or_default();
            // Find </data> for the value text.
            let after = gt + 1;
            let close_data = xml[after..].find("</data>").map(|i| after + i).unwrap_or(after);
            let value = xml[after..close_data].trim();
            if let Some(name) = key_names.get(&key_id) {
                sink(name.as_str(), value);
            }
            i = close_data + "</data>".len();
        } else {
            i = gt + 1;
        }
    }
    end + close.len()
}

fn find_from(bytes: &[u8], from: usize, byte: u8) -> Option<usize> {
    bytes[from..].iter().position(|&b| b == byte).map(|i| from + i)
}

/// Extract `attr="value"` (or `attr='value'`) from a tag body. Returns
/// `None` when the attr is absent or malformed.
fn attr(tag: &str, name: &str) -> Option<String> {
    let needle = format!(" {name}=\"");
    if let Some(start) = tag.find(&needle) {
        let from = start + needle.len();
        let end = from + tag[from..].find('"')?;
        return Some(decode_xml(&tag[from..end]));
    }
    let needle = format!(" {name}='");
    if let Some(start) = tag.find(&needle) {
        let from = start + needle.len();
        let end = from + tag[from..].find('\'')?;
        return Some(decode_xml(&tag[from..end]));
    }
    None
}

fn decode_xml(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

// ─── Proxy + rate-limit settings ────────────────────────────────
pub fn get_proxy_config(state: &AppState) -> ProxyConfig {
    state.proxies.snapshot()
}

pub fn set_proxy_config(state: &AppState, cfg: ProxyConfig) -> Result<(), String> {
    state.proxies.set(cfg).map_err(stringify)
}

pub fn get_rate_limit_config(state: &AppState) -> RateLimitConfig {
    state.rate_limiter.snapshot()
}

pub fn set_rate_limit_config(
    state: &AppState,
    cfg: RateLimitConfig,
) -> Result<(), String> {
    state.rate_limiter.set(cfg).map_err(stringify)
}

// ─── Notifications ─────────────────────────────────────────────
pub fn list_notifications(
    state: &AppState,
    case_id: i64,
    limit: i64,
) -> Result<Vec<crate::db::Notification>, String> {
    state
        .db
        .lock()
        .list_notifications(case_id, limit)
        .map_err(stringify)
}

pub fn unread_notification_count(state: &AppState, case_id: i64) -> Result<i64, String> {
    state
        .db
        .lock()
        .unread_notification_count(case_id)
        .map_err(stringify)
}

pub fn mark_notifications_read(state: &AppState, case_id: i64) -> Result<(), String> {
    state
        .db
        .lock()
        .mark_notifications_read(case_id)
        .map_err(stringify)
}

// ─── Webhooks ──────────────────────────────────────────────────
pub fn get_webhook_config(state: &AppState) -> crate::webhooks::WebhookConfig {
    state.webhooks.snapshot()
}

pub fn set_webhook_config(
    state: &AppState,
    cfg: crate::webhooks::WebhookConfig,
) -> Result<(), String> {
    state.webhooks.set(cfg).map_err(stringify)
}

/// Send a one-off test message to every configured webhook so the user
/// can verify the wire-up before the next scheduled run fires.
pub async fn test_webhooks(state: &AppState) -> Result<usize, String> {
    let urls = state.webhooks.urls();
    let count = urls.len();
    let text = format!(
        "🧪 maltrace webhook test — fired at {}",
        chrono::Utc::now().to_rfc3339()
    );
    crate::webhooks::dispatch_all(&urls, text).await;
    Ok(count)
}

// ─── Typed edges ───────────────────────────────────────────────
pub fn update_edge_relationship(
    state: &AppState,
    edge_id: &str,
    relationship_type: Option<&str>,
) -> Result<(), String> {
    let db = state.db.lock();
    // Pull the edge's case to enforce the case-lock.
    let edge = db
        .get_edge(edge_id)
        .map_err(stringify)?
        .ok_or_else(|| format!("edge {edge_id} not found"))?;
    db.ensure_case_writable(edge.case_id).map_err(stringify)?;
    db.update_edge_relationship(edge_id, relationship_type)
        .map_err(stringify)
}

// ─── Case backup / restore ─────────────────────────────────────
pub fn export_case_backup(state: &AppState, case_id: i64) -> Result<crate::backup::ExportResult, String> {
    crate::backup::export_case(state, case_id).map_err(stringify)
}

pub fn restore_case_backup(
    state: &AppState,
    bytes_base64: &str,
) -> Result<crate::backup::RestoreResult, String> {
    crate::backup::restore_case(state, bytes_base64).map_err(stringify)
}

// ─── Geo exports (GeoJSON + KML) ───────────────────────────────
pub fn export_case_geojson(state: &AppState, case_id: i64) -> Result<String, String> {
    let entities = state.db.lock().list_entities(case_id).map_err(stringify)?;
    Ok(crate::geo_export::export_geojson(&entities))
}

pub fn export_case_kml(state: &AppState, case_id: i64) -> Result<String, String> {
    let (case, entities) = {
        let db = state.db.lock();
        let case = db
            .get_case(case_id)
            .map_err(stringify)?
            .ok_or_else(|| format!("case {case_id} not found"))?;
        let entities = db.list_entities(case_id).map_err(stringify)?;
        (case, entities)
    };
    Ok(crate::geo_export::export_kml(&case.name, &entities))
}

// ─── Saved searches ────────────────────────────────────────────
pub fn list_saved_searches(
    state: &AppState,
    case_id: i64,
) -> Result<Vec<crate::db::SavedSearch>, String> {
    state.db.lock().list_saved_searches(case_id).map_err(stringify)
}

pub fn create_saved_search(
    state: &AppState,
    case_id: i64,
    name: &str,
    predicate: &str,
) -> Result<crate::db::SavedSearch, String> {
    let db = state.db.lock();
    db.ensure_case_writable(case_id).map_err(stringify)?;
    db.create_saved_search(case_id, name, predicate).map_err(stringify)
}

pub fn update_saved_search(
    state: &AppState,
    id: i64,
    name: &str,
    predicate: &str,
) -> Result<(), String> {
    state
        .db
        .lock()
        .update_saved_search(id, name, predicate)
        .map_err(stringify)
}

pub fn delete_saved_search(state: &AppState, id: i64) -> Result<(), String> {
    state.db.lock().delete_saved_search(id).map_err(stringify)
}

// ─── Tags ──────────────────────────────────────────────────────
pub fn list_tags_for_case(
    state: &AppState,
    case_id: i64,
) -> Result<HashMap<String, Vec<String>>, String> {
    state.db.lock().list_tags_for_case(case_id).map_err(stringify)
}

pub fn list_tags_for_entity(
    state: &AppState,
    entity_id: &str,
) -> Result<Vec<String>, String> {
    state.db.lock().list_tags_for_entity(entity_id).map_err(stringify)
}

pub fn add_tag(state: &AppState, entity_id: &str, tag: &str) -> Result<(), String> {
    let db = state.db.lock();
    db.ensure_entity_writable(entity_id).map_err(stringify)?;
    db.add_tag(entity_id, tag).map_err(stringify)
}

pub fn remove_tag(state: &AppState, entity_id: &str, tag: &str) -> Result<(), String> {
    let db = state.db.lock();
    db.ensure_entity_writable(entity_id).map_err(stringify)?;
    db.remove_tag(entity_id, tag).map_err(stringify)
}

// ─── Restore (undo for destructive actions) ────────────────────
/// Snapshot of one entity and the edges incident on it, captured at
/// delete time so undo can rebuild the world byte-for-byte.
#[derive(Debug, Deserialize, Serialize)]
pub struct EntitySnapshot {
    pub entity: Entity,
    pub incident_edges: Vec<Edge>,
    pub tags: Vec<String>,
}

/// Build a snapshot of `entity_id` (the entity, every edge touching
/// it, every tag attached) so the frontend can stash it in the undo
/// stack and replay via `restore_entities` later. Called from
/// `delete_entity` / `expand_collection` before the rows go away.
pub fn snapshot_entity(state: &AppState, entity_id: &str) -> Result<EntitySnapshot, String> {
    let db = state.db.lock();
    let entity = db
        .get_entity(entity_id)
        .map_err(stringify)?
        .ok_or_else(|| format!("entity {entity_id} not found"))?;
    let edges = db.list_edges(entity.case_id).map_err(stringify)?;
    let incident_edges = edges
        .into_iter()
        .filter(|e| e.source_id == entity_id || e.target_id == entity_id)
        .collect();
    let tags = db.list_tags_for_entity(entity_id).map_err(stringify)?;
    Ok(EntitySnapshot {
        entity,
        incident_edges,
        tags,
    })
}

/// Reinsert one or more entity snapshots — undo for `delete_entity` and
/// `expand_collection`. Each entity is restored with its original id +
/// timestamp; edges that reference entities not in the snapshot (and
/// not currently present in the DB) are silently dropped so a stale
/// snapshot can't resurrect garbage.
pub fn restore_entities(
    state: &AppState,
    snapshots: &[EntitySnapshot],
) -> Result<Vec<Entity>, String> {
    let db = state.db.lock();
    // Refuse if the case is locked. Use the first snapshot's case_id
    // since all must share one anyway.
    if let Some(s) = snapshots.first() {
        db.ensure_case_writable(s.entity.case_id).map_err(stringify)?;
    }

    let mut restored: Vec<Entity> = Vec::with_capacity(snapshots.len());
    let snapshot_ids: std::collections::HashSet<&str> =
        snapshots.iter().map(|s| s.entity.id.as_str()).collect();

    for s in snapshots {
        db.restore_entity(
            &s.entity.id,
            s.entity.case_id,
            &s.entity.entity_type,
            s.entity.source_event_type.as_deref(),
            &s.entity.label,
            s.entity.data.as_deref(),
            s.entity.pos_x,
            s.entity.pos_y,
            s.entity.created_at,
            s.entity.notes.as_deref(),
            s.entity.bookmarked,
            s.entity.locked,
            s.entity.created_by_run_id,
            s.entity.created_by_transform_id.as_deref(),
            s.entity.source_url.as_deref(),
            s.entity.created_by_user.as_deref(),
        )
        .map_err(stringify)?;
        for t in &s.tags {
            db.add_tag(&s.entity.id, t).map_err(stringify)?;
        }
        restored.push(s.entity.clone());
    }

    // Restore edges. An edge is restorable iff both endpoints exist
    // now — either they were in the snapshot set we just inserted or
    // they survived independently in the DB.
    for s in snapshots {
        for e in &s.incident_edges {
            let src_present = snapshot_ids.contains(e.source_id.as_str())
                || db
                    .get_entity(&e.source_id)
                    .map_err(stringify)?
                    .is_some();
            let tgt_present = snapshot_ids.contains(e.target_id.as_str())
                || db
                    .get_entity(&e.target_id)
                    .map_err(stringify)?
                    .is_some();
            if !src_present || !tgt_present {
                continue;
            }
            db.restore_edge(
                &e.id,
                e.case_id,
                &e.source_id,
                &e.target_id,
                e.label.as_deref(),
                e.created_at,
                e.run_id,
            )
            .map_err(stringify)?;
        }
    }
    Ok(restored)
}

// ─── Plugin install / uninstall / venv bootstrap ───────────────
pub fn install_plugin_from_directory(
    state: &AppState,
    source_dir: &str,
) -> Result<crate::plugins::InstallResult, String> {
    let res = crate::plugins::install_from_directory(std::path::Path::new(source_dir))
        .map_err(stringify)?;
    // Re-scan so the new plugin shows up next call without restart.
    state.sidecars.reload();
    Ok(res)
}

pub fn install_plugin_from_tarball(
    state: &AppState,
    bytes_base64: &str,
) -> Result<crate::plugins::InstallResult, String> {
    let res = crate::plugins::install_from_tarball(bytes_base64).map_err(stringify)?;
    state.sidecars.reload();
    Ok(res)
}

pub fn uninstall_plugin(state: &AppState, plugin_id: &str) -> Result<(), String> {
    crate::plugins::uninstall(plugin_id).map_err(stringify)?;
    state.sidecars.reload();
    Ok(())
}

pub fn bootstrap_plugin_venv(
    state: &AppState,
    plugin_id: &str,
    python_bin: Option<&str>,
) -> Result<crate::plugins::BootstrapResult, String> {
    let res = crate::plugins::bootstrap_venv(plugin_id, python_bin).map_err(stringify)?;
    // After a successful bootstrap the runner command (.venv/bin/python)
    // exists for the first time — refresh the registry so the plugin
    // becomes runnable without a manual restart.
    if res.status == "ok" {
        state.sidecars.reload();
    }
    Ok(res)
}

// ─── Machine schedules ─────────────────────────────────────────
pub fn list_machine_schedules(
    state: &AppState,
    case_id: i64,
) -> Result<Vec<crate::db::MachineSchedule>, String> {
    state.db.lock().list_machine_schedules(case_id).map_err(stringify)
}

pub fn create_machine_schedule(
    state: &AppState,
    case_id: i64,
    machine_id: &str,
    entity_id: &str,
    interval_seconds: i64,
) -> Result<crate::db::MachineSchedule, String> {
    let db = state.db.lock();
    db.ensure_case_writable(case_id).map_err(stringify)?;
    let entity = db
        .get_entity(entity_id)
        .map_err(stringify)?
        .ok_or_else(|| format!("entity {entity_id} not found"))?;
    if entity.case_id != case_id {
        return Err("entity belongs to a different case".to_string());
    }
    // Best-effort: refuse a schedule that points at an unknown machine.
    if state.machines.get(machine_id).is_none() {
        return Err(format!("unknown machine: {machine_id}"));
    }
    db.create_machine_schedule(
        case_id,
        machine_id,
        entity_id,
        &entity.label,
        interval_seconds,
    )
    .map_err(stringify)
}

pub fn set_machine_schedule_enabled(
    state: &AppState,
    id: i64,
    enabled: bool,
) -> Result<(), String> {
    state
        .db
        .lock()
        .set_machine_schedule_enabled(id, enabled)
        .map_err(stringify)
}

pub fn delete_machine_schedule(state: &AppState, id: i64) -> Result<(), String> {
    state.db.lock().delete_machine_schedule(id).map_err(stringify)
}

pub fn list_schedule_runs(
    state: &AppState,
    schedule_id: i64,
    limit: i64,
) -> Result<Vec<crate::db::MachineScheduleRun>, String> {
    state
        .db
        .lock()
        .list_schedule_runs(schedule_id, limit)
        .map_err(stringify)
}

// ─── Case snapshots ────────────────────────────────────────────
pub fn list_case_snapshots(
    state: &AppState,
    case_id: i64,
) -> Result<Vec<crate::db::CaseSnapshotMeta>, String> {
    state.db.lock().list_case_snapshots(case_id).map_err(stringify)
}

pub fn create_case_snapshot(
    state: &AppState,
    case_id: i64,
    name: &str,
) -> Result<crate::db::CaseSnapshotMeta, String> {
    let db = state.db.lock();
    db.create_case_snapshot(case_id, name).map_err(stringify)
}

pub fn delete_case_snapshot(state: &AppState, id: i64) -> Result<(), String> {
    state.db.lock().delete_case_snapshot(id).map_err(stringify)
}

pub fn diff_case_snapshots(
    state: &AppState,
    snapshot_id: i64,
    other_id: Option<i64>,
) -> Result<crate::db::CaseDiff, String> {
    state
        .db
        .lock()
        .diff_case_snapshots(snapshot_id, other_id)
        .map_err(stringify)
}

// ─── CSV export ─────────────────────────────────────────────────
/// Render the case's entities as CSV. One row per entity. Columns chosen
/// to round-trip through Excel/Sheets without escaping issues — full
/// payload (`data`) is shipped as a single quoted JSON column rather than
/// being exploded, so downstream tools can choose to parse it.
pub fn export_case_entities_csv(state: &AppState, case_id: i64) -> Result<String, String> {
    let db = state.db.lock();
    let entities = db.list_entities(case_id).map_err(stringify)?;
    drop(db);
    let mut out = String::new();
    out.push_str("id,entity_type,label,source_event_type,pos_x,pos_y,created_at,data\n");
    for e in &entities {
        out.push_str(&csv_field(&e.id));
        out.push(',');
        out.push_str(&csv_field(&e.entity_type));
        out.push(',');
        out.push_str(&csv_field(&e.label));
        out.push(',');
        out.push_str(&csv_field(e.source_event_type.as_deref().unwrap_or("")));
        out.push(',');
        out.push_str(
            &e.pos_x
                .map(|v| v.to_string())
                .unwrap_or_default(),
        );
        out.push(',');
        out.push_str(
            &e.pos_y
                .map(|v| v.to_string())
                .unwrap_or_default(),
        );
        out.push(',');
        out.push_str(&csv_field(&e.created_at.to_rfc3339()));
        out.push(',');
        out.push_str(&csv_field(e.data.as_deref().unwrap_or("")));
        out.push('\n');
    }
    Ok(out)
}

fn csv_field(s: &str) -> String {
    let needs_quoting = s.contains(',') || s.contains('"') || s.contains('\n') || s.contains('\r');
    if !needs_quoting {
        return s.to_string();
    }
    let escaped = s.replace('"', "\"\"");
    format!("\"{escaped}\"")
}

/// Run every compatible transform from `transform_ids` against a single
/// entity, sequentially. Sequential keeps blast-radius small (rate limits
/// + shared HTTP connections in sidecars). Per-transform failures are
/// captured in the run log via `run_transform`, and we keep going so a
/// single rate-limited service doesn't stop the others.
pub async fn run_all_transforms(
    state: &AppState,
    entity_id: &str,
    transform_ids: &[String],
    actor: Option<&str>,
) -> Result<Vec<TransformRunResult>, String> {
    let mut results = Vec::with_capacity(transform_ids.len());
    for tid in transform_ids {
        match run_transform_with_params(state, entity_id, tid, None, actor).await {
            Ok(r) => results.push(r),
            // Swallow per-transform failures: the user wants progress on
            // the other transforms, and the run log already captured why.
            Err(_) => continue,
        }
    }
    Ok(results)
}

pub async fn run_transform_batch(
    state: &AppState,
    entity_ids: &[String],
    transform_id: &str,
    actor: Option<&str>,
) -> Result<Vec<TransformRunResult>, String> {
    let mut results = Vec::with_capacity(entity_ids.len());
    // Sequential: keep blast-radius small and avoid hammering APIs in
    // parallel from a single user click. The UI shows a spinner across
    // the whole batch.
    for id in entity_ids {
        // Surface per-entity failures as Err entries in the batch so a
        // single bad target doesn't abort the rest. The frontend will
        // show one toast per failure.
        match run_transform_with_params(state, id, transform_id, None, actor).await {
            Ok(r) => results.push(r),
            Err(e) => return Err(format!("{id}: {e}")),
        }
    }
    Ok(results)
}

/// Materialise the children of a Collection entity: read its child payload,
/// re-create real entities + edges from its parent to each child, then
/// delete the Collection.
///
/// The Collection's incoming edge tells us which source it pivoted from —
/// we preserve that as the source of the materialised children's edges.
pub fn expand_collection(
    state: &AppState,
    collection_id: &str,
    actor: Option<&str>,
) -> Result<TransformRunResult, String> {
    let db = state.db.lock();
    db.ensure_entity_writable(collection_id).map_err(stringify)?;
    let collection = db
        .get_entity(collection_id)
        .map_err(stringify)?
        .ok_or_else(|| format!("collection {collection_id} not found"))?;
    if collection.entity_type != "Collection" {
        return Err(format!(
            "entity {collection_id} is not a Collection (got {})",
            collection.entity_type
        ));
    }
    let payload: CollectionPayload = match collection.data.as_deref() {
        Some(s) => serde_json::from_str(s)
            .map_err(|e| format!("collection {collection_id} has malformed data: {e}"))?,
        None => return Err(format!("collection {collection_id} has no children payload")),
    };

    // Find the (single) incoming edge from the parent that produced this
    // Collection so we can attribute the materialised children's edges
    // back to the same parent + transform.
    let incoming_edges: Vec<Edge> = db
        .list_edges(collection.case_id)
        .map_err(stringify)?
        .into_iter()
        .filter(|e| e.target_id == collection.id)
        .collect();
    let parent_edge = incoming_edges.first().ok_or_else(|| {
        format!("collection {collection_id} has no parent edge — cannot expand")
    })?;
    let parent = db
        .get_entity(&parent_edge.source_id)
        .map_err(stringify)?
        .ok_or_else(|| format!("parent {} not found", parent_edge.source_id))?;
    let transform_id = parent_edge.label.clone().unwrap_or_else(|| payload.transform_id.clone());

    // Re-emit the events around the collection's existing position so the
    // children spread out where the user was already looking.
    let pseudo_source = Entity {
        pos_x: collection.pos_x,
        pos_y: collection.pos_y,
        ..parent.clone()
    };
    // Inherit the original collection's run_id so expanded children
    // keep their provenance trail. The parent_edge.run_id is the
    // attribution we want; passing it (rather than starting a new run)
    // means the run log doesn't gain a phantom entry for "expand".
    let inherited_run_id = parent_edge.run_id;
    let result = persist_transform_output_inner(
        &db,
        &pseudo_source,
        &parent.id,
        &transform_id,
        inherited_run_id,
        actor,
        &payload.items,
        /* allow_collection = */ false,
    )
    .map_err(stringify)?;

    // Tear down the collection itself; cascade removes its single edge.
    db.delete_entity(&collection.id).map_err(stringify)?;

    Ok(TransformRunResult {
        source_entity: parent,
        new_entities: result.new_entities,
        new_edges: result.new_edges,
        transform_id,
        skipped: result.skipped,
        removed_entity_ids: vec![collection.id],
    })
}

pub async fn list_machines(state: &AppState) -> Vec<MachineMeta> {
    state.machines.list_meta()
}

/// Re-scan the `machines/` dir and return the new count. Used by the
/// MachinesPanel's reload button so investigators can edit a JSON file
/// without restarting the app.
pub fn reload_machines(state: &AppState) -> usize {
    state.machines.reload()
}

#[derive(Debug, Deserialize)]
pub struct AiChatInput {
    pub case_id: i64,
    pub history: Vec<ai::ChatTurn>,
    pub message: String,
    /// When true, the chat function builds a compact graph summary
    /// (entity counts, selected entity, recent additions) from the current
    /// case and prepends it to the user message as a `[Graph context: …]`
    /// block. Default false — the panel passes true unless the user opted
    /// out for a generic question.
    #[serde(default)]
    pub include_graph: bool,
    /// Optional id of the entity currently selected in the UI; included in
    /// the graph context summary when `include_graph` is true.
    #[serde(default)]
    pub selected_entity_id: Option<String>,
    /// When true, expose investigation tools (search_graph,
    /// run_transform, …) so Claude can pivot autonomously. Default
    /// false — the panel passes true on a per-message toggle.
    #[serde(default)]
    pub allow_tools: bool,
}

/// Maximum tool-use round-trips per chat turn. Each round costs one
/// Anthropic API call so we cap aggressively — most pivots are 2-4
/// hops anyway, and a runaway loop would be expensive.
const MAX_TOOL_ROUNDS: usize = 10;

pub async fn ai_chat(state: &AppState, input: AiChatInput) -> Result<ChatReply, String> {
    let graph_context = if input.include_graph {
        Some(build_graph_context(state, input.case_id, input.selected_entity_id.as_deref())?)
    } else {
        None
    };

    if !input.allow_tools {
        // Fast-path: classic single-shot chat, no tool plumbing.
        return ai::chat(
            &state.ai,
            ChatInput {
                history: input.history,
                message: input.message,
                graph_context,
            },
        )
        .await
        .map_err(stringify);
    }

    // ─── Tool-use loop ──────────────────────────────────────
    let case_id = input.case_id;
    let mut user_text = String::new();
    if let Some(ctx) = graph_context.as_deref().filter(|s| !s.is_empty()) {
        user_text.push_str("[Graph context: ");
        user_text.push_str(ctx);
        user_text.push_str("]\n\n");
    }
    user_text.push_str(&input.message);

    // Seed the conversation with prior turns + the new user message.
    let mut messages: Vec<serde_json::Value> = input
        .history
        .iter()
        .map(|t| {
            serde_json::json!({
                "role": t.role,
                "content": t.content,
            })
        })
        .collect();
    messages.push(serde_json::json!({
        "role": "user",
        "content": [{"type": "text", "text": user_text}],
    }));

    let tools = build_tool_specs();
    let mut tool_calls: Vec<ai::ToolCallTurn> = Vec::new();
    let mut last_usage = ai::Usage::default();
    let mut last_model = String::new();

    for round in 0..MAX_TOOL_ROUNDS {
        let resp = ai::chat_once(&state.ai, &messages, Some(&tools))
            .await
            .map_err(stringify)?;
        last_usage = resp.usage.clone();
        last_model = resp.model.clone();

        if resp.tool_uses.is_empty() {
            // Done — return the text.
            return Ok(ChatReply {
                message: if resp.text.is_empty() {
                    "(empty response from model)".to_string()
                } else {
                    resp.text
                },
                model: last_model,
                usage: last_usage,
                skills_loaded: state.ai.skills_loaded,
                tool_calls,
            });
        }

        // Append the assistant turn verbatim (preserving each block
        // exactly as Claude emitted it — `tool_use` ids must round-trip).
        messages.push(serde_json::json!({
            "role": "assistant",
            "content": resp.raw_content,
        }));

        // Execute every tool_use the assistant requested. Build one
        // user message containing all the tool_results.
        let mut tool_results: Vec<serde_json::Value> = Vec::with_capacity(resp.tool_uses.len());
        for tu in &resp.tool_uses {
            let result_value = dispatch_ai_tool(state, case_id, &tu.name, &tu.input).await;
            let (is_error, result_json) = match result_value {
                Ok(v) => (false, v),
                Err(e) => (
                    true,
                    serde_json::json!({"error": e}),
                ),
            };
            let result_string = result_json.to_string();
            // Truncate long tool outputs so a noisy result can't blow
            // up the context window.
            let trimmed = if result_string.len() > 8000 {
                format!("{}…[truncated]", &result_string[..8000])
            } else {
                result_string.clone()
            };
            tool_calls.push(ai::ToolCallTurn {
                tool_name: tu.name.clone(),
                args: tu.input.to_string(),
                result: trimmed.clone(),
            });
            tool_results.push(serde_json::json!({
                "type": "tool_result",
                "tool_use_id": tu.id,
                "content": trimmed,
                "is_error": is_error,
            }));
        }
        messages.push(serde_json::json!({
            "role": "user",
            "content": tool_results,
        }));

        // If Claude said "end_turn" but also emitted tool_uses (rare),
        // we still feed back; the next round will produce a final
        // text reply.
        let _ = round;
    }

    // Loop budget exhausted. Surface what we have so the UI doesn't
    // blow up.
    Ok(ChatReply {
        message: format!(
            "(tool-use loop hit the {}-round cap — partial reply; consider asking Claude to summarise progress)",
            MAX_TOOL_ROUNDS
        ),
        model: last_model,
        usage: last_usage,
        skills_loaded: state.ai.skills_loaded,
        tool_calls,
    })
}

/// Schema for every investigation tool Claude can call. The shape
/// follows the Anthropic tool-use contract: each tool is an object with
/// `name` + `description` + `input_schema` (JSON Schema for the args).
fn build_tool_specs() -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({
            "name": "search_graph",
            "description": "Fuzzy search on entity labels in the current case. Returns matching entities with their type + id so other tools can be called against them.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "Substring to match (case-insensitive)."},
                    "limit": {"type": "integer", "default": 20}
                },
                "required": ["query"]
            }
        }),
        serde_json::json!({
            "name": "get_entity_neighbors",
            "description": "Return the entities one hop away from an entity id, with the producing transform on each edge. Use this to follow a pivot trail.",
            "input_schema": {
                "type": "object",
                "properties": {"entity_id": {"type": "string"}},
                "required": ["entity_id"]
            }
        }),
        serde_json::json!({
            "name": "list_compatible_transforms",
            "description": "List the transforms whose input type matches the given entity. Use this before calling run_transform so you pick a valid pivot.",
            "input_schema": {
                "type": "object",
                "properties": {"entity_id": {"type": "string"}},
                "required": ["entity_id"]
            }
        }),
        serde_json::json!({
            "name": "run_transform",
            "description": "Actually run a transform on an entity. The result is added to the graph and returned to you. Confirm with the user before calling active transforms (sfp_portscan, sfp_dnsbrute) — passive ones (sfp_whois, sfp_dns_resolve) are safer to chain.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "entity_id": {"type": "string"},
                    "transform_id": {"type": "string"}
                },
                "required": ["entity_id", "transform_id"]
            }
        }),
        serde_json::json!({
            "name": "add_entity",
            "description": "Drop a manual entity onto the graph (e.g. a hypothesis-driven new domain to investigate). Use sparingly — most entities should come from transforms, not from you.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "entity_type": {"type": "string", "description": "Canonical type id (Domain, EmailAddress, IpAddress, …)."},
                    "label": {"type": "string"}
                },
                "required": ["entity_type", "label"]
            }
        }),
    ]
}

/// Dispatch one tool call. Returns the JSON value Claude will see as
/// the `tool_result` content. Errors surface as `is_error: true` to
/// the loop, which surfaces them to Claude as `{"error": "..."}` so
/// the model can recover (try a different entity / transform / etc.).
async fn dispatch_ai_tool(
    state: &AppState,
    case_id: i64,
    name: &str,
    input: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    match name {
        "search_graph" => {
            let q = input.get("query").and_then(|v| v.as_str()).unwrap_or("");
            let limit = input
                .get("limit")
                .and_then(|v| v.as_i64())
                .unwrap_or(20)
                .clamp(1, 100);
            if q.is_empty() {
                return Err("query is empty".into());
            }
            let entities = state.db.lock().list_entities(case_id).map_err(stringify)?;
            let needle = q.to_lowercase();
            let matches: Vec<serde_json::Value> = entities
                .into_iter()
                .filter(|e| {
                    e.label.to_lowercase().contains(&needle)
                        || e.entity_type.to_lowercase().contains(&needle)
                })
                .take(limit as usize)
                .map(|e| {
                    serde_json::json!({
                        "id": e.id,
                        "type": e.entity_type,
                        "label": e.label,
                        "bookmarked": e.bookmarked,
                    })
                })
                .collect();
            Ok(serde_json::json!({"matches": matches}))
        }
        "get_entity_neighbors" => {
            let id = input.get("entity_id").and_then(|v| v.as_str()).unwrap_or("");
            if id.is_empty() {
                return Err("entity_id is empty".into());
            }
            let prov = entity_provenance(state, id)?;
            let incoming: Vec<_> = prov
                .incoming
                .iter()
                .map(|e| {
                    serde_json::json!({
                        "entity_id": e.other_entity_id,
                        "type": e.other_entity_type,
                        "label": e.other_entity_label,
                        "via_transform": e.transform_id,
                    })
                })
                .collect();
            let outgoing: Vec<_> = prov
                .outgoing
                .iter()
                .map(|e| {
                    serde_json::json!({
                        "entity_id": e.other_entity_id,
                        "type": e.other_entity_type,
                        "label": e.other_entity_label,
                        "via_transform": e.transform_id,
                    })
                })
                .collect();
            Ok(serde_json::json!({
                "incoming": incoming,
                "outgoing": outgoing,
            }))
        }
        "list_compatible_transforms" => {
            let id = input.get("entity_id").and_then(|v| v.as_str()).unwrap_or("");
            if id.is_empty() {
                return Err("entity_id is empty".into());
            }
            let entity = state
                .db
                .lock()
                .get_entity(id)
                .map_err(stringify)?
                .ok_or_else(|| format!("entity {id} not found"))?;
            let transforms = list_transforms(state).await?;
            let sf_inputs = ontology::sf_inputs_for(&entity.entity_type);
            let candidates = if let Some(ref s) = entity.source_event_type {
                let mut v = sf_inputs.clone();
                v.push(s);
                v
            } else {
                sf_inputs
            };
            let compatible: Vec<_> = transforms
                .into_iter()
                .filter(|t| {
                    t.consumes.iter().any(|c| c == "*")
                        || candidates.iter().any(|s| t.consumes.iter().any(|c| c == s))
                })
                .map(|t| {
                    serde_json::json!({
                        "id": t.id,
                        "name": t.name,
                        "description": t.description,
                        "passivity": t.passivity,
                    })
                })
                .collect();
            Ok(serde_json::json!({"transforms": compatible}))
        }
        "run_transform" => {
            let entity_id = input
                .get("entity_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let transform_id = input
                .get("transform_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if entity_id.is_empty() || transform_id.is_empty() {
                return Err("entity_id and transform_id are required".into());
            }
            let res = run_transform_with_params(
                state,
                entity_id,
                transform_id,
                None,
                Some("ai-analyst"),
            )
            .await?;
            // Return only the bits Claude needs to keep pivoting —
            // not the whole entity payload.
            let new_entities: Vec<_> = res
                .new_entities
                .iter()
                .map(|e| {
                    serde_json::json!({
                        "id": e.id,
                        "type": e.entity_type,
                        "label": e.label,
                    })
                })
                .collect();
            Ok(serde_json::json!({
                "transform_id": res.transform_id,
                "new_entity_count": res.new_entities.len(),
                "new_entities": new_entities,
                "skipped": res.skipped,
            }))
        }
        "add_entity" => {
            let entity_type = input
                .get("entity_type")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let label = input.get("label").and_then(|v| v.as_str()).unwrap_or("");
            if entity_type.is_empty() || label.is_empty() {
                return Err("entity_type and label are required".into());
            }
            let e = add_entity(
                state,
                AddEntityInput {
                    case_id,
                    entity_type: entity_type.to_string(),
                    label: label.to_string(),
                    pos_x: None,
                    pos_y: None,
                },
                Some("ai-analyst"),
            )?;
            Ok(serde_json::json!({
                "id": e.id,
                "type": e.entity_type,
                "label": e.label,
            }))
        }
        _ => Err(format!("unknown tool: {name}")),
    }
}

#[derive(Debug, Serialize)]
pub struct AiStatus {
    pub configured: bool,
    pub model: String,
    pub skills_loaded: bool,
}

pub fn ai_status(state: &AppState) -> AiStatus {
    AiStatus {
        configured: state.ai.api_key.is_some(),
        model: state.ai.model.clone(),
        skills_loaded: state.ai.skills_loaded,
    }
}

/// Build a compact string the LLM can use to ground its answer. Kept terse
/// on purpose — verbose dumps would push the user's actual question down,
/// and we don't want to fight prompt caching by jamming the whole graph in.
fn build_graph_context(
    state: &AppState,
    case_id: i64,
    selected_id: Option<&str>,
) -> Result<String, String> {
    let db = state.db.lock();
    let entities = db.list_entities(case_id).map_err(stringify)?;
    let edges = db.list_edges(case_id).map_err(stringify)?;

    if entities.is_empty() {
        return Ok("the case is empty — no entities yet".to_string());
    }

    // Tally counts per canonical type so the model gets a quick mix-summary
    // ("8 Domains, 3 Emails, 1 WhoisRecord").
    let mut by_type: HashMap<&str, usize> = HashMap::new();
    for e in &entities {
        *by_type.entry(e.entity_type.as_str()).or_insert(0) += 1;
    }
    let mut by_type_vec: Vec<_> = by_type.into_iter().collect();
    by_type_vec.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    let type_summary = by_type_vec
        .iter()
        .map(|(t, n)| format!("{n} {t}"))
        .collect::<Vec<_>>()
        .join(", ");

    let selected_block = selected_id
        .and_then(|id| entities.iter().find(|e| e.id == id))
        .map(|e| {
            let src = e
                .source_event_type
                .as_deref()
                .map(|s| format!(" (from {s})"))
                .unwrap_or_default();
            format!("selected = {} \"{}\"{}", e.entity_type, e.label, src)
        });

    // Last few entities added — newest first; cap text so we don't blow up
    // a small chat panel with a 100-entity dump.
    let recent: Vec<_> = entities
        .iter()
        .rev()
        .take(5)
        .map(|e| format!("{} \"{}\"", e.entity_type, e.label))
        .collect();

    let mut parts = vec![
        format!("{} entities, {} edges", entities.len(), edges.len()),
        format!("mix: {type_summary}"),
    ];
    if let Some(s) = selected_block {
        parts.push(s);
    }
    if !recent.is_empty() {
        parts.push(format!("most recent: {}", recent.join("; ")));
    }
    Ok(parts.join(" | "))
}

pub async fn run_machine_as(
    state: &AppState,
    machine_id: &str,
    entity_id: &str,
    actor: Option<&str>,
) -> Result<crate::machine::MachineRunResult, String> {
    crate::machine::run_as(state, machine_id, entity_id, actor).await
}

pub async fn run_machine(
    state: &AppState,
    machine_id: &str,
    entity_id: &str,
) -> Result<MachineRunResult, String> {
    crate::machine::run(state, machine_id, entity_id).await
}

/// Per-canonical-type bag used while deciding whether a transform output
/// should collapse into a Collection.
#[derive(Default)]
struct BatchByType<'a> {
    events: Vec<&'a ProducedEvent>,
}

#[derive(Debug, Serialize, Deserialize)]
struct CollectionPayload {
    /// Canonical type id of the contained children (every item has the
    /// same canonical type — collections are homogeneous).
    collection_of: String,
    /// The transform invocation that produced this batch.
    transform_id: String,
    /// Raw ProducedEvents, re-materialised verbatim on expand.
    items: Vec<ProducedEvent>,
}

fn persist_transform_output(
    db: &Db,
    source: &Entity,
    transform_id: &str,
    run_id: i64,
    actor: Option<&str>,
    events: &[ProducedEvent],
) -> anyhow::Result<TransformRunResult> {
    persist_transform_output_inner(
        db,
        source,
        &source.id,
        transform_id,
        Some(run_id),
        actor,
        events,
        true,
    )
}

/// `parent_id` is normally the source's own id but differs during
/// `expand_collection` (where the visual source is the placeholder
/// collection but the edge attribution stays on the real parent).
///
/// `run_id` is the `transform_runs.id` to stamp on every persisted
/// entity / edge. `None` only for the `expand_collection` re-emit, where
/// there's no fresh run — we want the materialised children to inherit
/// the original collection's provenance via the `run_id` we read off the
/// collection-parent edge instead. (The caller handles that detail.)
///
/// `allow_collection`: pass `false` when re-materialising a collection's
/// children so a giant set doesn't immediately re-collapse.
fn persist_transform_output_inner(
    db: &Db,
    source: &Entity,
    parent_id: &str,
    transform_id: &str,
    run_id: Option<i64>,
    actor: Option<&str>,
    events: &[ProducedEvent],
    allow_collection: bool,
) -> anyhow::Result<TransformRunResult> {
    let mut new_entities = Vec::new();
    let mut new_edges = Vec::new();
    let mut skipped = Vec::new();

    let center_x = source.pos_x.unwrap_or(0.0);
    let center_y = source.pos_y.unwrap_or(0.0);

    let real_events: Vec<&ProducedEvent> = events
        .iter()
        .filter(|e| !ontology::is_noise(&e.event_type))
        .collect();

    // Group by canonical type so we only collapse homogeneous bags. A
    // single sfp_dnsresolve run that produces 14 IPs + 1 WhoisRecord
    // becomes 1 IpAddress Collection + 1 inline WhoisRecord — not a
    // 15-element collection.
    let mut by_type: HashMap<&'static str, BatchByType<'_>> = HashMap::new();
    let mut type_order: Vec<&'static str> = Vec::new();
    for evt in &real_events {
        let canonical = ontology::canonical_for(&evt.event_type);
        let bag = by_type.entry(canonical).or_default();
        if bag.events.is_empty() {
            type_order.push(canonical);
        }
        bag.events.push(*evt);
    }

    // Lay every produced node out around the source on a circle. Whether
    // a per-type bag becomes a Collection or N inline nodes doesn't
    // change the *number of slots* we want around the source — we count
    // slots = (one per inline event) + (one per collection bag).
    let mut slots = 0usize;
    for canonical in &type_order {
        let bag = &by_type[canonical];
        if allow_collection && bag.events.len() > COLLECTION_THRESHOLD {
            slots += 1;
        } else {
            slots += bag.events.len();
        }
    }
    let slot_total = slots.max(1) as f64;
    let radius = 200.0;
    let mut slot_idx: usize = 0;

    for canonical in &type_order {
        let bag = by_type.remove(canonical).unwrap();

        if allow_collection && bag.events.len() > COLLECTION_THRESHOLD {
            // Collapse → one Collection entity.
            let canonical_label = ontology::ALL
                .iter()
                .find(|c| c.id == *canonical)
                .map(|c| c.label)
                .unwrap_or(canonical);
            // Short fingerprint keeps repeated runs of the same transform
            // from colliding on the (case_id, entity_type, label) unique
            // index — the DB has one row per call.
            let short = uuid::Uuid::new_v4()
                .as_simple()
                .to_string()
                .chars()
                .take(6)
                .collect::<String>();
            let label = format!(
                "{} {} ({})",
                bag.events.len(),
                canonical_label,
                short
            );
            let items: Vec<ProducedEvent> = bag.events.iter().map(|e| (*e).clone()).collect();
            let payload = serde_json::to_string(&CollectionPayload {
                collection_of: (*canonical).to_string(),
                transform_id: transform_id.to_string(),
                items,
            })?;
            let angle = (slot_idx as f64 / slot_total) * std::f64::consts::TAU;
            let pos = Some((
                center_x + radius * angle.cos(),
                center_y + radius * angle.sin(),
            ));
            slot_idx += 1;
            // Collections themselves don't have a `source_url` — they
            // are a *bag* of N events from many backends. Provenance is
            // still threaded so the inspector can name the run that
            // produced the bag.
            let collection_prov = run_id.map(|rid| crate::db::Provenance {
                run_id: rid,
                transform_id,
                source_url: None,
            });
            let collection_entity = db.insert_entity_force(
                source.case_id,
                "Collection",
                None,
                &label,
                Some(&payload),
                pos,
                collection_prov.as_ref(),
                actor,
            )?;
            let edge = db.upsert_edge(
                source.case_id,
                parent_id,
                &collection_entity.id,
                Some(transform_id),
                run_id,
            )?;
            new_entities.push(collection_entity);
            new_edges.push(edge);
        } else {
            for evt in &bag.events {
                let label = if evt.data.len() > 80 {
                    format!("{}…", evt.data.chars().take(80).collect::<String>())
                } else {
                    evt.data.clone()
                };
                let angle = (slot_idx as f64 / slot_total) * std::f64::consts::TAU;
                let pos = Some((
                    center_x + radius * angle.cos(),
                    center_y + radius * angle.sin(),
                ));
                slot_idx += 1;
                let payload = serde_json::json!({
                    "full_data": evt.data,
                    "source_event_type": evt.source_event_type,
                    "confidence": evt.confidence,
                    "extra": evt.extra,
                })
                .to_string();
                // Source citation: many sidecars emit a canonical URL
                // for each fact under `extra.url` (brazuca every
                // resource, sfp_* anything that comes from a single
                // web source). When present, surface it as the
                // entity's `source_url` for the provenance UI.
                let source_url = evt
                    .extra
                    .as_ref()
                    .and_then(|v| v.get("url"))
                    .and_then(|v| v.as_str());
                let prov = run_id.map(|rid| crate::db::Provenance {
                    run_id: rid,
                    transform_id,
                    source_url,
                });
                let new_entity = db.upsert_entity(
                    source.case_id,
                    canonical,
                    Some(&evt.event_type),
                    &label,
                    Some(&payload),
                    pos,
                    prov.as_ref(),
                    actor,
                )?;
                let new_edge = db.upsert_edge(
                    source.case_id,
                    parent_id,
                    &new_entity.id,
                    Some(transform_id),
                    run_id,
                )?;
                new_entities.push(new_entity);
                new_edges.push(new_edge);
            }
        }
    }

    for evt in events.iter().filter(|e| ontology::is_noise(&e.event_type)) {
        skipped.push(format!("{}: {}", evt.event_type, evt.data));
    }

    Ok(TransformRunResult {
        source_entity: source.clone(),
        new_entities,
        new_edges,
        transform_id: transform_id.to_string(),
        skipped,
        removed_entity_ids: Vec::new(),
    })
}

pub(crate) fn stringify<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}
