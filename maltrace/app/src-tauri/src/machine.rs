//! Machines: investigator-defined pipelines of transforms.
//!
//! A Machine is an ordered list of steps. Each step picks a *frontier* of
//! entities to act on (the input, the entities produced by the previous
//! step, or everything touched so far) and runs one transform against every
//! frontier entity whose source event type the transform consumes.
//!
//! Definitions live as JSON files under `<repo>/machines/`. The format is
//! intentionally narrow so anyone can hand-write one — see
//! `machines/footprint_l1.json` for a full example.
//!
//! Errors per-step are *non-fatal*: if a transform fails on one entity the
//! machine logs that failure into the result and keeps going. A machine that
//! produces nothing still returns successfully (the user sees an empty
//! result, not a crash).

use crate::core::{run_transform_with_params, AppState, TransformRunResult};
use crate::db::{Edge, Entity};
use crate::ontology;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;

/// What the runner runs a step against.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ApplyTo {
    /// The original input entity. Useful for chaining multiple transforms
    /// on the same starting node (e.g. WHOIS + DNS + subdomain enumeration
    /// all from the same domain).
    Input,
    /// Entities produced by the previous step. Default.
    New,
    /// Every entity touched by the machine so far (input + all produced).
    /// Use sparingly — fans out fast.
    All,
}

impl Default for ApplyTo {
    fn default() -> Self {
        Self::New
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MachineStep {
    /// SpiderFoot module id or other transform id (must exist in the
    /// SidecarRegistry at run time).
    pub transform: String,
    /// Which entities this step is applied to. Defaults to `new`.
    #[serde(default)]
    pub on: ApplyTo,
    /// Optional canonical-type filter; only entities whose `entity_type`
    /// matches will be considered targets in this step.
    #[serde(default)]
    pub filter_type: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Machine {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Canonical type the machine expects to start from. The UI gates
    /// runnability on this.
    pub input_type: String,
    pub steps: Vec<MachineStep>,
}

/// Metadata-only projection of a Machine for list/UI calls. (Currently
/// the same shape as Machine — kept separate so we can prune detail
/// later, e.g. if step definitions grow.)
#[derive(Debug, Clone, Serialize)]
pub struct MachineMeta {
    pub id: String,
    pub name: String,
    pub description: String,
    pub input_type: String,
    pub step_count: usize,
    pub steps: Vec<MachineStep>,
}

#[derive(Debug, Serialize)]
pub struct MachineStepResult {
    pub transform: String,
    pub on: ApplyTo,
    /// How many entities the step was applied to (after compatibility +
    /// filter pruning).
    pub targets_run: usize,
    /// How many entities the step's runs produced in total.
    pub new_entity_count: usize,
    /// Per-target run failures (transform-level errors). Empty on success.
    pub failures: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct MachineRunResult {
    pub machine_id: String,
    pub source_entity: Entity,
    pub new_entities: Vec<Entity>,
    pub new_edges: Vec<Edge>,
    pub removed_entity_ids: Vec<String>,
    pub steps: Vec<MachineStepResult>,
}

#[derive(Debug, Clone, Default)]
pub struct MachineRegistry {
    machines: std::sync::Arc<parking_lot::Mutex<Vec<Machine>>>,
}

impl MachineRegistry {
    /// Load every `*.json` from `<repo>/machines/`. Files that fail to
    /// parse are logged and skipped — one bad file doesn't break the rest.
    pub fn from_repo_layout() -> Self {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        // CARGO_MANIFEST_DIR == .../maltrace/app/src-tauri
        let machines_dir = manifest_dir
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("machines"))
            .unwrap_or_else(|| PathBuf::from("machines"));
        Self::from_dir(&machines_dir)
    }

    pub fn from_dir(dir: &std::path::Path) -> Self {
        let me = Self::default();
        me.reload_from(dir);
        me
    }

    /// Re-scan `<repo>/machines/` and replace the in-memory list. Lets
    /// the user hot-edit a JSON without restarting the app. Returns the
    /// new count so the UI can show "loaded N machines".
    pub fn reload(&self) -> usize {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let machines_dir = manifest_dir
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("machines"))
            .unwrap_or_else(|| PathBuf::from("machines"));
        self.reload_from(&machines_dir)
    }

    fn reload_from(&self, dir: &std::path::Path) -> usize {
        let mut machines = Vec::new();
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(e) => {
                eprintln!(
                    "maltrace: machines dir {} not readable — {}; running with no machines",
                    dir.display(),
                    e
                );
                *self.machines.lock() = machines;
                return 0;
            }
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            match std::fs::read_to_string(&p) {
                Ok(s) => match serde_json::from_str::<Machine>(&s) {
                    Ok(m) => machines.push(m),
                    Err(e) => eprintln!("maltrace: skipped machine {} — {}", p.display(), e),
                },
                Err(e) => eprintln!("maltrace: skipped machine {} — {}", p.display(), e),
            }
        }
        // Deterministic order so list_machines is stable across restarts.
        machines.sort_by(|a, b| a.id.cmp(&b.id));
        let n = machines.len();
        *self.machines.lock() = machines;
        n
    }

    pub fn list_meta(&self) -> Vec<MachineMeta> {
        self.machines
            .lock()
            .iter()
            .map(|m| MachineMeta {
                id: m.id.clone(),
                name: m.name.clone(),
                description: m.description.clone(),
                input_type: m.input_type.clone(),
                step_count: m.steps.len(),
                steps: m.steps.clone(),
            })
            .collect()
    }

    /// Returns a *clone* of the machine so the caller can hold it
    /// across an `.await` without keeping the Mutex.
    pub fn get(&self, id: &str) -> Option<Machine> {
        self.machines.lock().iter().find(|m| m.id == id).cloned()
    }
}

/// Frontier-based machine runner.
///
/// Invariant: `all_touched` always contains every entity id we've seen as
/// either input or output; `prev_new` is the bag of entity ids produced
/// by the most recent step (or `[input]` at start). Steps choose their
/// target set with `ApplyTo`.
/// Convenience wrapper — no actor.
pub async fn run(
    state: &AppState,
    machine_id: &str,
    entity_id: &str,
) -> Result<MachineRunResult, String> {
    run_as(state, machine_id, entity_id, None).await
}

/// Actor-attributed variant. Threads `actor` into every nested
/// `run_transform` call so the run-log rows carry "scheduler" /
/// authenticated-user attribution instead of NULL.
pub async fn run_as(
    state: &AppState,
    machine_id: &str,
    entity_id: &str,
    actor: Option<&str>,
) -> Result<MachineRunResult, String> {
    let machine = state
        .machines
        .get(machine_id)
        .ok_or_else(|| format!("unknown machine: {machine_id}"))?;

    let source = {
        let db = state.db.lock();
        db.get_entity(entity_id)
            .map_err(crate::core::stringify)?
            .ok_or_else(|| format!("entity {entity_id} not found"))?
    };

    if source.entity_type != machine.input_type {
        return Err(format!(
            "machine {} expects {} as input, got {}",
            machine_id, machine.input_type, source.entity_type
        ));
    }

    // Look up the transform catalogue once so we can decide compatibility
    // per step without a sidecar round-trip on every entity.
    let transforms = state
        .sidecars
        .list_all_transforms()
        .await
        .map_err(crate::core::stringify)?;

    let mut all_touched: HashSet<String> = HashSet::new();
    all_touched.insert(source.id.clone());
    let mut prev_new: Vec<String> = vec![source.id.clone()];

    let mut agg_new_entities: Vec<Entity> = Vec::new();
    let mut agg_new_edges: Vec<Edge> = Vec::new();
    let mut agg_removed: Vec<String> = Vec::new();
    let mut step_results: Vec<MachineStepResult> = Vec::new();

    for step in &machine.steps {
        let transform_meta = transforms.iter().find(|t| t.id == step.transform);
        let consumed: Vec<String> = transform_meta
            .map(|t| t.consumes.clone())
            .unwrap_or_default();

        let candidate_ids: Vec<String> = match step.on {
            ApplyTo::Input => vec![source.id.clone()],
            ApplyTo::New => prev_new.clone(),
            ApplyTo::All => all_touched.iter().cloned().collect(),
        };

        // Resolve entity records once per step and filter by compatibility.
        // We touch the DB lock briefly, then drop it before awaiting any
        // transform — the lock is parking_lot which is not async-safe to
        // hold across .await.
        let candidates: Vec<Entity> = {
            let db = state.db.lock();
            candidate_ids
                .iter()
                .filter_map(|id| db.get_entity(id).ok().flatten())
                .collect()
        };

        let targets: Vec<Entity> = candidates
            .into_iter()
            .filter(|e| {
                if let Some(ref ft) = step.filter_type {
                    if e.entity_type != *ft {
                        return false;
                    }
                }
                if consumed.iter().any(|c| c == "*") {
                    return true;
                }
                // Compatibility test: either the entity has a known
                // source_event_type the transform consumes, or its
                // canonical type's default SF event is consumed.
                let sf_inputs = ontology::sf_inputs_for(&e.entity_type);
                let candidates = if let Some(ref s) = e.source_event_type {
                    let mut v = sf_inputs.clone();
                    v.push(s);
                    v
                } else {
                    sf_inputs
                };
                candidates.iter().any(|s| consumed.iter().any(|c| c == s))
            })
            // Never run a step on a Collection entity — the right thing is
            // to expand it first. Machines should treat Collections as
            // opaque.
            .filter(|e| e.entity_type != "Collection")
            .collect();

        let mut step_new_ids: Vec<String> = Vec::new();
        let mut failures: Vec<String> = Vec::new();
        let mut new_entity_count = 0usize;

        for tgt in &targets {
            match run_transform_with_params(state, &tgt.id, &step.transform, None, actor).await {
                Ok(r) => persist_step_outcome(
                    r,
                    &mut all_touched,
                    &mut step_new_ids,
                    &mut new_entity_count,
                    &mut agg_new_entities,
                    &mut agg_new_edges,
                    &mut agg_removed,
                ),
                Err(e) => failures.push(format!("{}: {}", tgt.label, e)),
            }
        }

        step_results.push(MachineStepResult {
            transform: step.transform.clone(),
            on: step.on,
            targets_run: targets.len(),
            new_entity_count,
            failures,
        });

        prev_new = step_new_ids;
    }

    Ok(MachineRunResult {
        machine_id: machine.id,
        source_entity: source,
        new_entities: agg_new_entities,
        new_edges: agg_new_edges,
        removed_entity_ids: agg_removed,
        steps: step_results,
    })
}

#[allow(clippy::too_many_arguments)]
fn persist_step_outcome(
    r: TransformRunResult,
    all_touched: &mut HashSet<String>,
    step_new_ids: &mut Vec<String>,
    new_entity_count: &mut usize,
    agg_new_entities: &mut Vec<Entity>,
    agg_new_edges: &mut Vec<Edge>,
    agg_removed: &mut Vec<String>,
) {
    for e in r.new_entities {
        if all_touched.insert(e.id.clone()) {
            step_new_ids.push(e.id.clone());
            *new_entity_count += 1;
            agg_new_entities.push(e);
        }
    }
    for edge in r.new_edges {
        agg_new_edges.push(edge);
    }
    for id in r.removed_entity_ids {
        agg_removed.push(id);
    }
}
