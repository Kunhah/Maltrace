// Mirrors the Serialize structs in src-tauri/src/{db,sidecar,commands,ontology}.rs.

/// Result of `search_entities_across_cases`. Carries the originating case
/// name so the cross-case finder can render "found in case X" + a jump.
export interface CrossCaseMatch {
  entity_id: string;
  case_id: number;
  entity_type: string;
  label: string;
  bookmarked: boolean;
  case_name: string;
}

/// Audit-log row: one investigator-significant mutation. Together with
/// `transform_runs` (transform invocations) they form the evidence-
/// integrity trail.
export interface AuditEvent {
  id: number;
  case_id: number;
  /// Stable verb (`entity_renamed`, `entity_deleted`, `entity_merged`, …).
  kind: string;
  entity_id: string | null;
  /// JSON-encoded payload — schema varies by `kind`.
  payload: string | null;
  created_at: string;
  /// Authenticated user or `"desktop"` that took the action.
  /// `null` only on pre-v10 rows.
  actor?: string | null;
}

export interface Case {
  id: number;
  name: string;
  created_at: string;
  /// True when the case is "filed" — write paths refuse to mutate it.
  /// Mirrored from the backend `cases.locked` column.
  locked?: boolean;
  /// ISO timestamp of the most recent lock. Shown next to the lock badge.
  locked_at?: string | null;
}

export interface Entity {
  id: string;
  case_id: number;
  /// Canonical type id, e.g. "Domain", "EmailAddress".
  entity_type: string;
  /// Original SpiderFoot event type when the entity came from a transform.
  source_event_type: string | null;
  label: string;
  data: string | null;
  pos_x: number | null;
  pos_y: number | null;
  created_at: string;
  /// Investigator notes — free-form, persisted across sessions.
  notes?: string | null;
  /// "Pin / star" flag; surfaces a filter chip and a visual indicator on
  /// the node.
  bookmarked?: boolean;
  /// Pinned-position flag — locked nodes don't move during layouts and
  /// can't be dragged. Cytoscape's `node.lock()` semantics.
  locked?: boolean;
  /// Provenance: `transform_runs.id` of the run that created this
  /// entity. `null` for manual / imported / seed rows. Powers the
  /// inspector's "where did this come from" section.
  created_by_run_id?: number | null;
  created_by_transform_id?: string | null;
  /// Source citation extracted from the originating ProducedEvent's
  /// `extra.url` field.
  source_url?: string | null;
  /// Authenticated user (server build) or `"desktop"` (Tauri build)
  /// that created this row. `null` only on pre-v10 / pre-auth rows.
  created_by_user?: string | null;
}

/// Free-floating sticky-note on the canvas. Not an entity — doesn't
/// participate in transforms, ontology, or export. Maltego users use
/// these for "this cluster is the suspect's infra" labels.
export interface Annotation {
  id: string;
  case_id: number;
  text: string;
  pos_x: number;
  pos_y: number;
  /// Hex colour for the sticky-note background.
  color: string;
  created_at: string;
}

export interface Edge {
  id: string;
  case_id: number;
  source_id: string;
  target_id: string;
  label: string | null;
  created_at: string;
  /// `transform_runs.id` that produced this edge.
  run_id?: number | null;
  /// Optional typed relationship label (e.g. `owns`, `resolves_to`,
  /// `abuses_of`). Set by the user via the edge inspector — separate
  /// from `label` (which carries the producer transform id).
  relationship_type?: string | null;
}

/// Wire-format result of `GET /entities/:id/provenance`. One fetch, all
/// the facts the provenance section needs.
export interface EdgeProvenance {
  edge_id: string;
  other_entity_id: string;
  other_entity_label: string;
  other_entity_type: string;
  direction: "incoming" | "outgoing";
  transform_id: string | null;
  run_id: number | null;
  created_at: string;
}

export interface EntityProvenance {
  run: TransformRun | null;
  transform_id: string | null;
  source_url: string | null;
  created_at: string;
  incoming: EdgeProvenance[];
  outgoing: EdgeProvenance[];
}

/// `GET /edges/:id` payload — used by the edge inspector pane.
export interface EdgeDetail {
  edge: Edge;
  source_label: string | null;
  source_type: string | null;
  target_label: string | null;
  target_type: string | null;
  run: TransformRun | null;
}

export interface TransformMeta {
  id: string;
  name: string;
  description: string;
  categories: string[];
  group: string[];
  consumes: string[];
  produces: string[];
  /// Which sidecar contributed this transform (e.g. "spiderfoot",
  /// "face-search"). Empty string when the source isn't known.
  source: string;
  /// User-overridable knobs. Empty when the transform doesn't surface
  /// any; the UI skips the parameter dialog and runs directly.
  parameters?: ParameterSpec[];
  /// OPSEC classification — `"active"` (touches the target),
  /// `"passive"` (third-party index only), or `"unknown"` (unrated).
  /// Stealth-mode filter hides every non-`passive` transform.
  passivity?: "active" | "passive" | "unknown" | string;
}

export type ParameterType = "string" | "number" | "boolean" | "choice";

export interface ParameterSpec {
  name: string;
  type: ParameterType;
  description?: string;
  default?: unknown;
  choices?: string[];
  required?: boolean;
}

export interface TransformRunResult {
  source_entity: Entity;
  new_entities: Entity[];
  new_edges: Edge[];
  transform_id: string;
  skipped: string[];
  /// Entity ids the caller should drop from its local graph state
  /// (set when a Collection was expanded into its children).
  removed_entity_ids: string[];
}

export type MachineApplyTo = "input" | "new" | "all";

export interface MachineStep {
  transform: string;
  on: MachineApplyTo;
  filter_type?: string | null;
}

export interface MachineMeta {
  id: string;
  name: string;
  description: string;
  input_type: string;
  step_count: number;
  steps: MachineStep[];
}

export interface MachineStepResult {
  transform: string;
  on: MachineApplyTo;
  targets_run: number;
  new_entity_count: number;
  failures: string[];
}

export interface MachineRunResult {
  machine_id: string;
  source_entity: Entity;
  new_entities: Entity[];
  new_edges: Edge[];
  removed_entity_ids: string[];
  steps: MachineStepResult[];
}

/// Payload stored in `Entity.data` when entity_type === "Collection".
export interface CollectionPayload {
  collection_of: string;
  transform_id: string;
  items: Array<{
    event_type: string;
    data: string;
    module?: string | null;
    confidence?: number | null;
    source_event_type?: string | null;
    extra?: unknown;
  }>;
}

export type CanonicalKind =
  | "identity"
  | "asset"
  | "evidence"
  | "threat"
  | "generic";

export interface CanonicalType {
  id: string;
  label: string;
  kind: CanonicalKind;
  color: string;
  /// Every SpiderFoot event type that maps to this canonical id. Used to
  /// filter the transform palette for an entity whose `source_event_type`
  /// is null (e.g. manually added).
  sf_inputs: string[];
  /// The SF event used when invoking a transform on a manual entity of
  /// this type (always the first entry of `sf_inputs`, or the canonical id
  /// itself as a fallback for types with no SF coverage).
  default_sf_event: string;
  /// True for user-defined types created at runtime. Built-ins have
  /// `custom: false`.
  custom: boolean;
}

// ─── AI assistant panel ──────────────────────────────────────────
export interface AiStatus {
  /// True when `` is set on the backend process.
  configured: boolean;
  /// Model ID the backend will call (e.g. "claude-opus-4-7").
  model: string;
  /// True when the Claude-OSINT skill files were found and embedded into the
  /// system prompt; false means the panel runs with a generic role only.
  skills_loaded: boolean;
}

export interface ChatTurn {
  role: "user" | "assistant";
  content: string;
}

export interface AiUsage {
  input_tokens: number;
  output_tokens: number;
  cache_creation_input_tokens: number;
  cache_read_input_tokens: number;
}

export interface ChatReply {
  message: string;
  model: string;
  usage: AiUsage;
  skills_loaded: boolean;
  /// Tool calls Claude made during this turn (when `allow_tools` was
  /// set on the request). Empty when tools are off or Claude didn't
  /// pivot. Rendered as inline cards in the chat transcript.
  tool_calls?: ToolCallTurn[];
}

export interface ToolCallTurn {
  tool_name: string;
  /// JSON-stringified args.
  args: string;
  /// JSON-stringified result returned to Claude.
  result: string;
}

export interface AiChatInput {
  case_id: number;
  history: ChatTurn[];
  message: string;
  include_graph?: boolean;
  selected_entity_id?: string | null;
  /// When true, Claude can call investigation tools (search_graph,
  /// run_transform, …). Default false.
  allow_tools?: boolean;
}

// ─── Plugin SDK ──────────────────────────────────────────────────
/// Summary of one loaded plugin, returned by `list_plugins`. See
/// docs/PLUGIN_SDK.md for the public plugin contract.
export interface PluginSummary {
  id: string;
  name: string;
  version: string;
  license: string;
  description: string;
  homepage: string | null;
  plugin_dir: string;
  /// Transforms this plugin contributed last time the catalogue was fetched.
  /// `null` until `listTransforms()` has been called at least once.
  transform_count: number | null;
  /// False when the runner command doesn't exist on disk — typically means
  /// the plugin's venv hasn't been set up yet.
  runnable: boolean;
}

/// Per-sidecar load status, returned by `list_sidecar_status`. Captured the
/// last time the transform catalogue was refreshed. `ok: false` means the
/// sidecar failed to list its transforms (broken venv, adapter error) and
/// was dropped from the catalogue — the UI surfaces these as a warning.
export interface SidecarStatus {
  id: string;
  ok: boolean;
  transform_count: number;
  error: string | null;
}

// ─── Secrets (API keys store) ────────────────────────────────────
export interface SecretInfo {
  name: string;
  /// Always true for entries returned from the store; the UI synthesises
  /// `set: false` rows when merging in preset key names.
  set: boolean;
}

// ─── Transform-run log ──────────────────────────────────────────
export interface TransformRun {
  id: number;
  case_id: number;
  transform_id: string;
  source_entity_id: string | null;
  source_label: string | null;
  started_at: string;
  duration_ms: number;
  status: "ok" | "error" | string;
  new_entity_count: number;
  error_message: string | null;
  stderr_excerpt: string | null;
  /// Authenticated user or `"desktop"` or `"scheduler"` (for autonomous
  /// scheduled re-runs) that triggered this run.
  created_by_user?: string | null;
}

// ─── Proxy config (per-sidecar) ───────────────────────────────────
export interface ProxyConfig {
  /// Applied when a sidecar doesn't have a per-sidecar override.
  /// Empty string disables the default.
  default_proxy: string;
  /// sidecar_id → proxy URL. Empty string opts a sidecar out even if a
  /// default is set.
  per_sidecar: Record<string, string>;
}

// ─── Rate limits (per-sidecar) ────────────────────────────────────
export interface Limits {
  max_concurrent: number;
  min_interval_ms: number;
}

export interface RateLimitConfig {
  default?: Limits | null;
  per_sidecar: Record<string, Limits>;
}

// ─── Machine schedules ────────────────────────────────────────────
export interface MachineSchedule {
  id: number;
  case_id: number;
  machine_id: string;
  entity_id: string;
  entity_label: string;
  interval_seconds: number;
  enabled: boolean;
  created_at: string;
  next_run_at: string;
  last_run_at: string | null;
}

export interface MachineScheduleRun {
  id: number;
  schedule_id: number;
  started_at: string;
  status: "ok" | "error" | string;
  new_entity_count: number;
  error_message: string | null;
}

// ─── GraphML import ───────────────────────────────────────────────
export interface ImportResult {
  case_id: number;
  new_entity_count: number;
  new_edge_count: number;
  skipped: string[];
}

// ─── Snapshot + restore (undo for destructive ops) ────────────────
export interface EntitySnapshot {
  entity: Entity;
  incident_edges: Edge[];
  tags: string[];
}

// ─── Plugin install ───────────────────────────────────────────────
export interface PluginInstallResult {
  plugin_id: string;
  plugin_dir: string;
  needs_venv: boolean;
}

export interface PluginBootstrapResult {
  plugin_id: string;
  status: "ok" | "error" | string;
  stdout_excerpt: string;
  stderr_excerpt: string;
}

// ─── Saved searches ──────────────────────────────────────────────
/// JSON shape stored in `saved_searches.predicate`. The backend treats
/// it as opaque; the frontend interprets it client-side when applying
/// the filter. All fields optional and combined with AND semantics.
export interface SearchPredicate {
  /// Only entities whose `entity_type` is in this set survive.
  entity_types?: string[];
  /// Substring match (case-insensitive) on entity label.
  label_contains?: string;
  /// Only entities that carry all of these tags.
  all_tags?: string[];
  /// Only entities with at least one of these tags.
  any_tag?: string[];
  /// True = only bookmarked entities.
  bookmarked_only?: boolean;
  /// True = only entities with a non-null `source_url`.
  has_source_url?: boolean;
}

export interface SavedSearch {
  id: number;
  case_id: number;
  name: string;
  /// JSON-encoded SearchPredicate.
  predicate: string;
  created_at: string;
}

// ─── Case snapshots ─────────────────────────────────────────────
export interface CaseSnapshotMeta {
  id: number;
  case_id: number;
  name: string;
  created_at: string;
  entity_count: number;
  edge_count: number;
}

export interface SnapshotEntity {
  id: string;
  entity_type: string;
  label: string;
  source_event_type: string | null;
}

export interface SnapshotEdge {
  id: string;
  source_id: string;
  target_id: string;
  label: string | null;
}

export interface RenamedEntity {
  id: string;
  entity_type: string;
  before: string;
  after: string;
}

export interface CaseDiff {
  added_entities: SnapshotEntity[];
  removed_entities: SnapshotEntity[];
  renamed_entities: RenamedEntity[];
  added_edges: SnapshotEdge[];
  removed_edges: SnapshotEdge[];
}

// ─── Notifications + webhooks ────────────────────────────────────
export interface Notification {
  id: number;
  case_id: number;
  kind: string;
  title: string;
  detail: string | null;
  payload: string | null;
  created_at: string;
  read_at: string | null;
}

export interface WebhookConfig {
  urls: string[];
}

// ─── Backup ───────────────────────────────────────────────────────
export interface BackupExportResult {
  bytes_base64: string;
  manifest: {
    schema_version: number;
    case_name: string;
    exported_at: string;
    entity_count: number;
    edge_count: number;
    run_count: number;
    file_count: number;
  };
}

export interface BackupRestoreResult {
  new_case_id: number;
  new_case_name: string;
  entity_count: number;
  edge_count: number;
}

// ─── Auth ─────────────────────────────────────────────────────────
export interface AuthStatus {
  /// True when the server is running without auth (Tauri build, or
  /// `MALTRACE_NO_AUTH=1` / empty users.json). The UI hides the login
  /// modal when this is set.
  bypass: boolean;
  authenticated: boolean;
  name: string | null;
  role: "analyst" | "reader" | null;
}
