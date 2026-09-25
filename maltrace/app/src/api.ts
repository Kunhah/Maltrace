import type {
  AiChatInput,
  AiStatus,
  Annotation,
  AuditEvent,
  AuthStatus,
  CanonicalType,
  Case,
  ChatReply,
  CrossCaseMatch,
  Edge,
  EdgeDetail,
  Entity,
  EntityProvenance,
  EntitySnapshot,
  ImportResult,
  MachineMeta,
  MachineRunResult,
  MachineSchedule,
  MachineScheduleRun,
  PluginBootstrapResult,
  PluginInstallResult,
  PluginSummary,
  ProxyConfig,
  RateLimitConfig,
  SecretInfo,
  SidecarStatus,
  TransformMeta,
  TransformRun,
  TransformRunResult,
} from "./types";

/// Dual-transport API client.
///
/// Tauri builds talk to the Rust core via `invoke` (no HTTP server needed).
/// Browser builds talk to `maltrace-server` over `fetch` against same-origin
/// `/api/*` (Vite proxies these in `dev`).
const isTauri =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

const API_BASE = "/api";

/// Lazy-loaded `invoke` so the web bundle never imports `@tauri-apps/api`
/// (which probes `window.__TAURI_INTERNALS__` and warns when absent).
type InvokeFn = <T>(cmd: string, args?: Record<string, unknown>) => Promise<T>;
let invokeFn: InvokeFn | null = null;
async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (!invokeFn) {
    const mod = await import("@tauri-apps/api/core");
    invokeFn = mod.invoke as unknown as InvokeFn;
  }
  return invokeFn<T>(cmd, args);
}

/// Same lazy-load pattern for the opener plugin.
type OpenerFn = (url: string) => Promise<void>;
let openerFn: OpenerFn | null = null;

/// Bearer token threaded into the `Authorization` header on every
/// `http()` call. Set via `api.setAuthToken(...)` from the login flow;
/// stored in localStorage so reloads keep the session.
const TOKEN_KEY = "maltrace_auth_token";
let authToken: string | null = (() => {
  try {
    return typeof window !== "undefined" ? localStorage.getItem(TOKEN_KEY) : null;
  } catch {
    return null;
  }
})();

/// Special-case `401`: the auth middleware uses it to mean "log in".
/// Errors carry a `.status` field so the UI can switch to the login
/// modal without parsing the message.
export class HttpError extends Error {
  status: number;
  constructor(status: number, message: string) {
    super(message);
    this.status = status;
  }
}

async function http<T>(
  method: string,
  path: string,
  body?: unknown,
): Promise<T> {
  const init: RequestInit = { method, headers: {} };
  if (body !== undefined) {
    init.body = JSON.stringify(body);
    (init.headers as Record<string, string>)["content-type"] = "application/json";
  }
  if (authToken) {
    (init.headers as Record<string, string>)["authorization"] = `Bearer ${authToken}`;
  }
  const res = await fetch(`${API_BASE}${path}`, init);
  if (!res.ok) {
    let msg = `${method} ${path} → ${res.status}`;
    try {
      const j = await res.json();
      if (j && typeof j === "object" && "error" in j) msg = String((j as { error: unknown }).error);
    } catch {
      /* body was not JSON; keep status-line message */
    }
    throw new HttpError(res.status, msg);
  }
  if (res.status === 204) return undefined as T;
  // Some endpoints return raw text (CSV, GraphML, data URLs) rather than
  // JSON — dispatch on content-type so the call shape stays uniform.
  const ct = res.headers.get("content-type") ?? "";
  if (ct.startsWith("application/json")) return (await res.json()) as T;
  return (await res.text()) as T;
}

export const api = {
  currentCase: (): Promise<Case> =>
    isTauri ? invoke<Case>("current_case") : http<Case>("GET", "/case"),

  listCases: (): Promise<Case[]> =>
    isTauri ? invoke<Case[]>("list_cases") : http<Case[]>("GET", "/cases"),

  createCase: (name: string): Promise<Case> =>
    isTauri
      ? invoke<Case>("create_case", { name })
      : http<Case>("POST", "/cases", { name }),

  /// Drop a small "what a graph looks like" demo into a case. Idempotent
  /// against the existing unique index — running it twice doesn't dupe.
  seedSampleGraph: (caseId: number): Promise<{ entities: Entity[]; edges: Edge[] }> =>
    isTauri
      ? invoke<{ entities: Entity[]; edges: Edge[] }>("seed_sample_graph", {
          caseId,
        })
      : http<{ entities: Entity[]; edges: Edge[] }>(
          "POST",
          `/cases/${caseId}/seed-sample`,
        ),

  renameCase: (id: number, name: string): Promise<void> =>
    isTauri
      ? invoke<void>("rename_case", { id, name })
      : http<void>("PATCH", `/cases/${id}`, { name }),

  deleteCase: (id: number): Promise<void> =>
    isTauri
      ? invoke<void>("delete_case", { id })
      : http<void>("DELETE", `/cases/${id}`),

  /// Toggle a case's evidence-lock. When locked, every backend mutator
  /// refuses with a clear error. Returns the updated Case so the UI can
  /// read `locked_at`.
  setCaseLocked: (id: number, locked: boolean): Promise<Case> =>
    isTauri
      ? invoke<Case>("set_case_locked", { id, locked })
      : http<Case>("PATCH", `/cases/${id}/lock`, { locked }),

  /// Fetch the full provenance record for one entity: originating run,
  /// transform id, source URL, and incoming/outgoing edges with their
  /// own run attribution. Used by the inspector's Provenance section.
  getEntityProvenance: (entityId: string): Promise<EntityProvenance> =>
    isTauri
      ? invoke<EntityProvenance>("entity_provenance", { id: entityId })
      : http<EntityProvenance>(
          "GET",
          `/entities/${encodeURIComponent(entityId)}/provenance`,
        ),

  /// Resolve one edge id to its source/target metadata + the run that
  /// produced it. Used by the edge inspector pane.
  getEdgeDetail: (edgeId: string): Promise<EdgeDetail> =>
    isTauri
      ? invoke<EdgeDetail>("edge_detail", { id: edgeId })
      : http<EdgeDetail>("GET", `/edges/${encodeURIComponent(edgeId)}`),

  /// Returns the GraphML XML as a UTF-8 string. The caller decides how to
  /// surface it — for Tauri/web, the App wraps it in a Blob and triggers a
  /// download via an anchor element.
  exportCaseGraphml: (caseId: number): Promise<string> =>
    isTauri
      ? invoke<string>("export_case_graphml", { caseId })
      : http<string>("GET", `/cases/${caseId}/graphml`),

  exportCaseEntitiesCsv: (caseId: number): Promise<string> =>
    isTauri
      ? invoke<string>("export_case_entities_csv", { caseId })
      : http<string>("GET", `/cases/${caseId}/entities.csv`),

  listTransformRuns: (caseId: number, limit = 100): Promise<TransformRun[]> =>
    isTauri
      ? invoke<TransformRun[]>("list_transform_runs", { caseId, limit })
      : http<TransformRun[]>("GET", `/cases/${caseId}/runs?limit=${limit}`),

  listSecretNames: (): Promise<SecretInfo[]> =>
    isTauri
      ? invoke<SecretInfo[]>("list_secret_names")
      : http<SecretInfo[]>("GET", "/secrets"),

  setSecret: (name: string, value: string): Promise<void> =>
    isTauri
      ? invoke<void>("set_secret", { name, value })
      : http<void>("POST", "/secrets", { name, value }),

  deleteSecret: (name: string): Promise<void> =>
    isTauri
      ? invoke<void>("delete_secret", { name })
      : http<void>("DELETE", `/secrets/${encodeURIComponent(name)}`),

  listEntities: (caseId: number): Promise<Entity[]> =>
    isTauri
      ? invoke<Entity[]>("list_entities", { caseId })
      : http<Entity[]>("GET", `/entities?case_id=${caseId}`),

  listEdges: (caseId: number): Promise<Edge[]> =>
    isTauri
      ? invoke<Edge[]>("list_edges", { caseId })
      : http<Edge[]>("GET", `/edges?case_id=${caseId}`),

  listCanonicalTypes: (): Promise<CanonicalType[]> =>
    isTauri
      ? invoke<CanonicalType[]>("list_canonical_types")
      : http<CanonicalType[]>("GET", "/canonical-types"),

  createCustomEntityType: (
    label: string,
    kind: string,
    color: string,
  ): Promise<{
    id: string;
    label: string;
    kind: string;
    color: string;
    created_at: string;
  }> =>
    isTauri
      ? invoke("create_custom_entity_type", { label, kind, color })
      : http("POST", "/canonical-types", { label, kind, color }),

  deleteCustomEntityType: (id: string): Promise<void> =>
    isTauri
      ? invoke<void>("delete_custom_entity_type", { id })
      : http<void>("DELETE", `/canonical-types/${encodeURIComponent(id)}`),

  addEntity: (
    caseId: number,
    entityType: string,
    label: string,
    pos?: { x: number; y: number },
  ): Promise<Entity> =>
    isTauri
      ? invoke<Entity>("add_entity", {
          caseId,
          entityType,
          label,
          posX: pos?.x ?? null,
          posY: pos?.y ?? null,
        })
      : http<Entity>("POST", "/entities", {
          case_id: caseId,
          entity_type: entityType,
          label,
          pos_x: pos?.x ?? null,
          pos_y: pos?.y ?? null,
        }),

  updateEntityPosition: (id: string, x: number, y: number): Promise<void> =>
    isTauri
      ? invoke<void>("update_entity_position", { id, x, y })
      : http<void>("PATCH", `/entities/${encodeURIComponent(id)}/position`, {
          x,
          y,
        }),

  updateEntityNotes: (id: string, notes: string | null): Promise<void> =>
    isTauri
      ? invoke<void>("update_entity_notes", { id, notes })
      : http<void>("PATCH", `/entities/${encodeURIComponent(id)}/notes`, {
          notes,
        }),

  updateEntityBookmark: (id: string, bookmarked: boolean): Promise<void> =>
    isTauri
      ? invoke<void>("update_entity_bookmark", { id, bookmarked })
      : http<void>("PATCH", `/entities/${encodeURIComponent(id)}/bookmark`, {
          bookmarked,
        }),

  updateEntityLabel: (id: string, label: string): Promise<void> =>
    isTauri
      ? invoke<void>("update_entity_label", { id, label })
      : http<void>("PATCH", `/entities/${encodeURIComponent(id)}/label`, {
          label,
        }),

  updateEntityLocked: (id: string, locked: boolean): Promise<void> =>
    isTauri
      ? invoke<void>("update_entity_locked", { id, locked })
      : http<void>("PATCH", `/entities/${encodeURIComponent(id)}/locked`, {
          locked,
        }),

  listBookmarked: (caseId: number): Promise<Entity[]> =>
    isTauri
      ? invoke<Entity[]>("list_bookmarked", { caseId })
      : http<Entity[]>("GET", `/cases/${caseId}/bookmarks`),

  listTransformRunsForEntity: (entityId: string, limit = 50): Promise<TransformRun[]> =>
    isTauri
      ? invoke<TransformRun[]>("list_transform_runs_for_entity", {
          entityId,
          limit,
        })
      : http<TransformRun[]>(
          "GET",
          `/entities/${encodeURIComponent(entityId)}/runs?limit=${limit}`,
        ),

  /// Substring-search entities across every case. Used by the cross-case
  /// mode of GraphFinder ("have I seen this before?"). Bookmarked rows
  /// rank first.
  searchEntitiesAcrossCases: (
    query: string,
    limit = 50,
  ): Promise<CrossCaseMatch[]> =>
    isTauri
      ? invoke<CrossCaseMatch[]>("search_entities_across_cases", {
          query,
          limit,
        })
      : http<CrossCaseMatch[]>(
          "GET",
          `/entities/search?q=${encodeURIComponent(query)}&limit=${limit}`,
        ),

  listAuditEvents: (caseId: number, limit = 100): Promise<AuditEvent[]> =>
    isTauri
      ? invoke<AuditEvent[]>("list_audit_events", { caseId, limit })
      : http<AuditEvent[]>("GET", `/cases/${caseId}/audit?limit=${limit}`),

  listAnnotations: (caseId: number): Promise<Annotation[]> =>
    isTauri
      ? invoke<Annotation[]>("list_annotations", { caseId })
      : http<Annotation[]>("GET", `/cases/${caseId}/annotations`),

  createAnnotation: (
    caseId: number,
    text: string,
    posX: number,
    posY: number,
    color?: string | null,
  ): Promise<Annotation> =>
    isTauri
      ? invoke<Annotation>("create_annotation", {
          caseId,
          text,
          posX,
          posY,
          color: color ?? null,
        })
      : http<Annotation>("POST", `/cases/${caseId}/annotations`, {
          text,
          pos_x: posX,
          pos_y: posY,
          color: color ?? null,
        }),

  updateAnnotation: (
    id: string,
    patch: {
      text?: string;
      posX?: number;
      posY?: number;
      color?: string;
    },
  ): Promise<void> =>
    isTauri
      ? invoke<void>("update_annotation", {
          id,
          text: patch.text ?? null,
          posX: patch.posX ?? null,
          posY: patch.posY ?? null,
          color: patch.color ?? null,
        })
      : http<void>("PATCH", `/annotations/${encodeURIComponent(id)}`, {
          text: patch.text ?? null,
          pos_x: patch.posX ?? null,
          pos_y: patch.posY ?? null,
          color: patch.color ?? null,
        }),

  deleteAnnotation: (id: string): Promise<void> =>
    isTauri
      ? invoke<void>("delete_annotation", { id })
      : http<void>("DELETE", `/annotations/${encodeURIComponent(id)}`),

  mergeEntities: (keepId: string, dropId: string): Promise<void> =>
    isTauri
      ? invoke<void>("merge_entities", { keepId, dropId })
      : http<void>("POST", "/entities/merge", {
          keep_id: keepId,
          drop_id: dropId,
        }),

  deleteEntity: (id: string): Promise<void> =>
    isTauri
      ? invoke<void>("delete_entity", { id })
      : http<void>("DELETE", `/entities/${encodeURIComponent(id)}`),

  listTransforms: (): Promise<TransformMeta[]> =>
    isTauri
      ? invoke<TransformMeta[]>("list_transforms")
      : http<TransformMeta[]>("GET", "/transforms"),

  runTransform: (entityId: string, transformId: string): Promise<TransformRunResult> =>
    isTauri
      ? invoke<TransformRunResult>("run_transform", { entityId, transformId })
      : http<TransformRunResult>(
          "POST",
          `/transforms/${encodeURIComponent(transformId)}/run`,
          { entity_id: entityId },
        ),

  runTransformBatch: (
    entityIds: string[],
    transformId: string,
  ): Promise<TransformRunResult[]> =>
    isTauri
      ? invoke<TransformRunResult[]>("run_transform_batch", {
          entityIds,
          transformId,
        })
      : http<TransformRunResult[]>(
          "POST",
          `/transforms/${encodeURIComponent(transformId)}/run-batch`,
          { entity_ids: entityIds },
        ),

  runAllTransforms: (
    entityId: string,
    transformIds: string[],
  ): Promise<TransformRunResult[]> =>
    isTauri
      ? invoke<TransformRunResult[]>("run_all_transforms", {
          entityId,
          transformIds,
        })
      : http<TransformRunResult[]>(
          "POST",
          `/entities/${encodeURIComponent(entityId)}/run-all`,
          { transform_ids: transformIds },
        ),

  uploadFileToCase: (
    caseId: number,
    fileName: string,
    bytesBase64: string,
  ): Promise<Entity> =>
    isTauri
      ? invoke<Entity>("upload_file_to_case", {
          caseId,
          fileName,
          bytesBase64,
        })
      : http<Entity>("POST", `/cases/${caseId}/files`, {
          file_name: fileName,
          bytes_base64: bytesBase64,
        }),

  readEntityFileAsDataUrl: (entityId: string): Promise<string> =>
    isTauri
      ? invoke<string>("read_entity_file_as_data_url", { entityId })
      : http<string>(
          "GET",
          `/entities/${encodeURIComponent(entityId)}/file`,
        ),

  expandCollection: (collectionId: string): Promise<TransformRunResult> =>
    isTauri
      ? invoke<TransformRunResult>("expand_collection", { collectionId })
      : http<TransformRunResult>(
          "POST",
          `/collections/${encodeURIComponent(collectionId)}/expand`,
        ),

  listMachines: (): Promise<MachineMeta[]> =>
    isTauri
      ? invoke<MachineMeta[]>("list_machines")
      : http<MachineMeta[]>("GET", "/machines"),

  runMachine: (machineId: string, entityId: string): Promise<MachineRunResult> =>
    isTauri
      ? invoke<MachineRunResult>("run_machine", { machineId, entityId })
      : http<MachineRunResult>(
          "POST",
          `/machines/${encodeURIComponent(machineId)}/run`,
          { entity_id: entityId },
        ),

  listPlugins: (): Promise<PluginSummary[]> =>
    isTauri
      ? invoke<PluginSummary[]>("list_plugins")
      : http<PluginSummary[]>("GET", "/plugins"),

  listSidecarStatus: (): Promise<SidecarStatus[]> =>
    isTauri
      ? invoke<SidecarStatus[]>("list_sidecar_status")
      : http<SidecarStatus[]>("GET", "/sidecar-status"),

  aiStatus: (): Promise<AiStatus> =>
    isTauri
      ? invoke<AiStatus>("ai_status")
      : http<AiStatus>("GET", "/ai/status"),

  aiChat: (input: AiChatInput): Promise<ChatReply> =>
    isTauri
      ? invoke<ChatReply>("ai_chat", { input })
      : http<ChatReply>("POST", "/ai/chat", input),

  /// Open a URL in the user's default browser (Tauri) or a new tab (web).
  /// Rejects non-http(s) schemes so a malicious transform can't slip a
  /// `javascript:` / `file:` URL onto the graph and have it opened.
  runTransformWithParams: (
    entityId: string,
    transformId: string,
    params: Record<string, unknown> | null,
  ): Promise<TransformRunResult> =>
    isTauri
      ? invoke<TransformRunResult>("run_transform_with_params", {
          entityId,
          transformId,
          params: params ?? null,
        })
      : http<TransformRunResult>(
          "POST",
          `/transforms/${encodeURIComponent(transformId)}/run-with-params`,
          { entity_id: entityId, params },
        ),

  exportCaseStix: (caseId: number): Promise<string> =>
    isTauri
      ? invoke<string>("export_case_stix", { caseId })
      : http<string>("GET", `/cases/${caseId}/stix.json`),

  exportCaseReportHtml: (caseId: number): Promise<string> =>
    isTauri
      ? invoke<string>("export_case_report_html", { caseId })
      : http<string>("GET", `/cases/${caseId}/report.html`),

  importCaseGraphml: (caseId: number, xml: string): Promise<ImportResult> =>
    isTauri
      ? invoke<ImportResult>("import_case_graphml", { caseId, xml })
      : http<ImportResult>("POST", `/cases/${caseId}/graphml/import`, { xml }),

  getProxyConfig: (): Promise<ProxyConfig> =>
    isTauri
      ? invoke<ProxyConfig>("get_proxy_config")
      : http<ProxyConfig>("GET", "/settings/proxies"),

  setProxyConfig: (config: ProxyConfig): Promise<void> =>
    isTauri
      ? invoke<void>("set_proxy_config", { config })
      : http<void>("PUT", "/settings/proxies", config),

  getRateLimitConfig: (): Promise<RateLimitConfig> =>
    isTauri
      ? invoke<RateLimitConfig>("get_rate_limit_config")
      : http<RateLimitConfig>("GET", "/settings/rate-limits"),

  setRateLimitConfig: (config: RateLimitConfig): Promise<void> =>
    isTauri
      ? invoke<void>("set_rate_limit_config", { config })
      : http<void>("PUT", "/settings/rate-limits", config),

  listMachineSchedules: (caseId: number): Promise<MachineSchedule[]> =>
    isTauri
      ? invoke<MachineSchedule[]>("list_machine_schedules", { caseId })
      : http<MachineSchedule[]>("GET", `/cases/${caseId}/schedules`),

  createMachineSchedule: (
    caseId: number,
    machineId: string,
    entityId: string,
    intervalSeconds: number,
  ): Promise<MachineSchedule> =>
    isTauri
      ? invoke<MachineSchedule>("create_machine_schedule", {
          caseId,
          machineId,
          entityId,
          intervalSeconds,
        })
      : http<MachineSchedule>("POST", `/cases/${caseId}/schedules`, {
          machine_id: machineId,
          entity_id: entityId,
          interval_seconds: intervalSeconds,
        }),

  setMachineScheduleEnabled: (id: number, enabled: boolean): Promise<void> =>
    isTauri
      ? invoke<void>("set_machine_schedule_enabled", { id, enabled })
      : http<void>("PATCH", `/schedules/${id}`, { enabled }),

  deleteMachineSchedule: (id: number): Promise<void> =>
    isTauri
      ? invoke<void>("delete_machine_schedule", { id })
      : http<void>("DELETE", `/schedules/${id}`),

  listScheduleRuns: (scheduleId: number, limit = 50): Promise<MachineScheduleRun[]> =>
    isTauri
      ? invoke<MachineScheduleRun[]>("list_schedule_runs", { scheduleId, limit })
      : http<MachineScheduleRun[]>(
          "GET",
          `/schedules/${scheduleId}/runs?limit=${limit}`,
        ),

  // ─── Auth ─────────────────────────────────────────────────
  setAuthToken: (token: string | null) => {
    authToken = token;
    try {
      if (token) localStorage.setItem(TOKEN_KEY, token);
      else localStorage.removeItem(TOKEN_KEY);
    } catch {
      /* private mode or no DOM */
    }
  },
  hasAuthToken: (): boolean => !!authToken,

  authWhoami: (): Promise<AuthStatus> =>
    // Tauri build doesn't have an HTTP middleware — synthesize a
    // bypass-style status so the UI skips the login modal.
    isTauri
      ? Promise.resolve({
          bypass: true,
          authenticated: true,
          name: "desktop",
          role: "analyst",
        })
      : http<AuthStatus>("GET", "/auth/whoami"),

  authLogin: (token: string): Promise<AuthStatus> =>
    isTauri
      ? Promise.resolve({
          bypass: true,
          authenticated: true,
          name: "desktop",
          role: "analyst",
        })
      : http<AuthStatus>("POST", "/auth/login", { token }),

  // ─── Tags ─────────────────────────────────────────────────
  listTagsForCase: (caseId: number): Promise<Record<string, string[]>> =>
    isTauri
      ? invoke<Record<string, string[]>>("list_tags_for_case", { caseId })
      : http<Record<string, string[]>>(
          "GET",
          `/tags?case_id=${caseId}`,
        ),

  listTagsForEntity: (entityId: string): Promise<string[]> =>
    isTauri
      ? invoke<string[]>("list_tags_for_entity", { entityId })
      : http<string[]>("GET", `/entities/${encodeURIComponent(entityId)}/tags`),

  addTag: (entityId: string, tag: string): Promise<void> =>
    isTauri
      ? invoke<void>("add_tag", { entityId, tag })
      : http<void>(
          "POST",
          `/entities/${encodeURIComponent(entityId)}/tags/${encodeURIComponent(tag)}`,
        ),

  removeTag: (entityId: string, tag: string): Promise<void> =>
    isTauri
      ? invoke<void>("remove_tag", { entityId, tag })
      : http<void>(
          "DELETE",
          `/entities/${encodeURIComponent(entityId)}/tags/${encodeURIComponent(tag)}`,
        ),

  // ─── Snapshot + restore (undo for destructive) ─────────────
  snapshotEntity: (entityId: string): Promise<EntitySnapshot> =>
    isTauri
      ? invoke<EntitySnapshot>("snapshot_entity", { entityId })
      : http<EntitySnapshot>(
          "GET",
          `/entities/${encodeURIComponent(entityId)}/snapshot`,
        ),

  restoreEntities: (snapshots: EntitySnapshot[]): Promise<Entity[]> =>
    isTauri
      ? invoke<Entity[]>("restore_entities", { snapshots })
      : http<Entity[]>("POST", "/entities/restore", { snapshots }),

  // ─── Plugin install ───────────────────────────────────────
  installPluginFromDirectory: (sourceDir: string): Promise<PluginInstallResult> =>
    isTauri
      ? invoke<PluginInstallResult>("install_plugin_from_directory", {
          sourceDir,
        })
      : http<PluginInstallResult>("POST", "/plugins/install/directory", {
          source_dir: sourceDir,
        }),

  installPluginFromTarball: (bytesBase64: string): Promise<PluginInstallResult> =>
    isTauri
      ? invoke<PluginInstallResult>("install_plugin_from_tarball", {
          bytesBase64,
        })
      : http<PluginInstallResult>("POST", "/plugins/install/tarball", {
          bytes_base64: bytesBase64,
        }),

  uninstallPlugin: (pluginId: string): Promise<void> =>
    isTauri
      ? invoke<void>("uninstall_plugin", { pluginId })
      : http<void>("POST", `/plugins/${encodeURIComponent(pluginId)}/uninstall`),

  bootstrapPluginVenv: (
    pluginId: string,
    pythonBin?: string,
  ): Promise<PluginBootstrapResult> =>
    isTauri
      ? invoke<PluginBootstrapResult>("bootstrap_plugin_venv", {
          pluginId,
          pythonBin: pythonBin ?? null,
        })
      : http<PluginBootstrapResult>(
          "POST",
          `/plugins/${encodeURIComponent(pluginId)}/bootstrap-venv`,
          { python_bin: pythonBin ?? null },
        ),

  // ─── Notifications ────────────────────────────────────────
  listNotifications: (
    caseId: number,
    limit = 100,
  ): Promise<import("./types").Notification[]> =>
    isTauri
      ? invoke<import("./types").Notification[]>("list_notifications", {
          caseId,
          limit,
        })
      : http<import("./types").Notification[]>(
          "GET",
          `/cases/${caseId}/notifications?limit=${limit}`,
        ),

  unreadNotificationCount: (caseId: number): Promise<number> =>
    isTauri
      ? invoke<number>("unread_notification_count", { caseId })
      : http<{ unread: number }>(
          "GET",
          `/cases/${caseId}/notifications/unread`,
        ).then((r) => r.unread),

  markNotificationsRead: (caseId: number): Promise<void> =>
    isTauri
      ? invoke<void>("mark_notifications_read", { caseId })
      : http<void>("POST", `/cases/${caseId}/notifications`),

  // ─── Webhooks ─────────────────────────────────────────────
  getWebhookConfig: (): Promise<import("./types").WebhookConfig> =>
    isTauri
      ? invoke<import("./types").WebhookConfig>("get_webhook_config")
      : http<import("./types").WebhookConfig>("GET", "/settings/webhooks"),

  setWebhookConfig: (
    config: import("./types").WebhookConfig,
  ): Promise<void> =>
    isTauri
      ? invoke<void>("set_webhook_config", { config })
      : http<void>("PUT", "/settings/webhooks", config),

  testWebhooks: (): Promise<number> =>
    isTauri
      ? invoke<number>("test_webhooks")
      : http<{ fired: number }>("POST", "/settings/webhooks/test").then(
          (r) => r.fired,
        ),

  // ─── Typed edges ──────────────────────────────────────────
  updateEdgeRelationship: (
    edgeId: string,
    relationshipType: string | null,
  ): Promise<void> =>
    isTauri
      ? invoke<void>("update_edge_relationship", {
          edgeId,
          relationshipType,
        })
      : http<void>(
          "PATCH",
          `/edges/${encodeURIComponent(edgeId)}/relationship`,
          { relationship_type: relationshipType },
        ),

  // ─── Case backup / restore ────────────────────────────────
  exportCaseBackup: (
    caseId: number,
  ): Promise<import("./types").BackupExportResult> =>
    isTauri
      ? invoke<import("./types").BackupExportResult>("export_case_backup", {
          caseId,
        })
      : http<import("./types").BackupExportResult>(
          "GET",
          `/cases/${caseId}/backup`,
        ),

  restoreCaseBackup: (
    bytesBase64: string,
  ): Promise<import("./types").BackupRestoreResult> =>
    isTauri
      ? invoke<import("./types").BackupRestoreResult>("restore_case_backup", {
          bytesBase64,
        })
      : http<import("./types").BackupRestoreResult>(
          "POST",
          "/cases/backup/restore",
          { bytes_base64: bytesBase64 },
        ),

  // ─── Saved searches ───────────────────────────────────────
  listSavedSearches: (caseId: number): Promise<import("./types").SavedSearch[]> =>
    isTauri
      ? invoke<import("./types").SavedSearch[]>("list_saved_searches", {
          caseId,
        })
      : http<import("./types").SavedSearch[]>(
          "GET",
          `/cases/${caseId}/saved-searches`,
        ),

  createSavedSearch: (
    caseId: number,
    name: string,
    predicate: string,
  ): Promise<import("./types").SavedSearch> =>
    isTauri
      ? invoke<import("./types").SavedSearch>("create_saved_search", {
          caseId,
          name,
          predicate,
        })
      : http<import("./types").SavedSearch>(
          "POST",
          `/cases/${caseId}/saved-searches`,
          { name, predicate },
        ),

  updateSavedSearch: (
    id: number,
    name: string,
    predicate: string,
  ): Promise<void> =>
    isTauri
      ? invoke<void>("update_saved_search", { id, name, predicate })
      : http<void>("PATCH", `/saved-searches/${id}`, { name, predicate }),

  deleteSavedSearch: (id: number): Promise<void> =>
    isTauri
      ? invoke<void>("delete_saved_search", { id })
      : http<void>("DELETE", `/saved-searches/${id}`),

  // ─── Case snapshots ───────────────────────────────────────
  listCaseSnapshots: (
    caseId: number,
  ): Promise<import("./types").CaseSnapshotMeta[]> =>
    isTauri
      ? invoke<import("./types").CaseSnapshotMeta[]>("list_case_snapshots", {
          caseId,
        })
      : http<import("./types").CaseSnapshotMeta[]>(
          "GET",
          `/cases/${caseId}/snapshots`,
        ),

  createCaseSnapshot: (
    caseId: number,
    name: string,
  ): Promise<import("./types").CaseSnapshotMeta> =>
    isTauri
      ? invoke<import("./types").CaseSnapshotMeta>("create_case_snapshot", {
          caseId,
          name,
        })
      : http<import("./types").CaseSnapshotMeta>(
          "POST",
          `/cases/${caseId}/snapshots`,
          { name },
        ),

  deleteCaseSnapshot: (id: number): Promise<void> =>
    isTauri
      ? invoke<void>("delete_case_snapshot", { id })
      : http<void>("DELETE", `/snapshots/${id}`),

  diffCaseSnapshots: (
    snapshotId: number,
    otherId: number | null,
  ): Promise<import("./types").CaseDiff> => {
    if (isTauri) {
      return invoke<import("./types").CaseDiff>("diff_case_snapshots", {
        snapshotId,
        otherId,
      });
    }
    const qs = otherId != null ? `?other=${otherId}` : "";
    return http<import("./types").CaseDiff>(
      "GET",
      `/snapshots/${snapshotId}/diff${qs}`,
    );
  },

  reloadMachines: (): Promise<number> =>
    isTauri
      ? invoke<number>("reload_machines")
      : http<{ count: number }>("POST", "/machines/reload").then(
          (r) => r.count,
        ),

  // ─── Geo exports ──────────────────────────────────────────
  exportCaseGeoJson: (caseId: number): Promise<string> =>
    isTauri
      ? invoke<string>("export_case_geojson", { caseId })
      : http<string>("GET", `/cases/${caseId}/geo.geojson`),

  exportCaseKml: (caseId: number): Promise<string> =>
    isTauri
      ? invoke<string>("export_case_kml", { caseId })
      : http<string>("GET", `/cases/${caseId}/geo.kml`),

  /// Subscribe to the server's live-update SSE stream. Web build only —
  /// Tauri's single-user model doesn't need it. Returns a cleanup
  /// function that closes the EventSource.
  ///
  /// The stream emits JSON events like
  /// `{kind: "graph_changed", case_id, change}` — callers re-fetch
  /// their case data on receipt of any matching event. Auto-reconnect
  /// is handled by the EventSource itself.
  subscribeToEvents: (onEvent: (event: unknown) => void): (() => void) => {
    if (isTauri) return () => {};
    // EventSource ignores the Authorization header — token threaded
    // via query string when present. Server middleware skips
    // `/api/events` from auth (TODO if we want auth on this).
    const url = `${API_BASE}/events${
      authToken ? `?access_token=${encodeURIComponent(authToken)}` : ""
    }`;
    let source: EventSource | null;
    try {
      source = new EventSource(url);
    } catch (e) {
      // eslint-disable-next-line no-console
      console.warn("SSE subscribe failed:", e);
      return () => {};
    }
    source.onmessage = (ev) => {
      try {
        onEvent(JSON.parse(ev.data));
      } catch {
        /* malformed event; ignore */
      }
    };
    return () => {
      source?.close();
    };
  },

  openUrl: async (url: string): Promise<void> => {
    if (!isSafeUrl(url)) {
      throw new Error(`Refusing to open unsafe URL: ${url}`);
    }
    if (isTauri) {
      if (!openerFn) {
        const mod = await import("@tauri-apps/plugin-opener");
        openerFn = mod.openUrl as OpenerFn;
      }
      return openerFn(url);
    }
    const w = window.open(url, "_blank", "noopener,noreferrer");
    if (!w) {
      throw new Error("Popup blocked. Allow popups for this site.");
    }
  },
};

export const runtime = { isTauri };

/// True when the string is a syntactically valid http/https URL. Exported so
/// the inspector can decide whether to show its open button without parsing
/// the URL twice.
export function isSafeUrl(u: string): boolean {
  try {
    const parsed = new URL(u);
    return parsed.protocol === "http:" || parsed.protocol === "https:";
  } catch {
    return false;
  }
}
