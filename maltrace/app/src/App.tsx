import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api, runtime } from "./api";
import type {
  Annotation,
  CanonicalType,
  Case,
  Edge,
  Entity,
  MachineMeta,
  MachineStepResult,
  TransformMeta,
  TransformRunResult,
} from "./types";
import {
  GraphCanvas,
  type GraphHandle,
  type LayoutKind,
} from "./components/GraphCanvas";
import { EntityInspector } from "./components/EntityInspector";
import { AddEntityForm } from "./components/AddEntityForm";
import { CanvasControls } from "./components/CanvasControls";
import { ShortcutsHint } from "./components/ShortcutsHint";
import { WelcomeModal } from "./components/WelcomeModal";
import { EXAMPLE_TARGETS } from "./recommendations";

const WELCOME_SEEN_KEY = "maltrace.welcomeSeen.v1";
import { TransformContextMenu } from "./components/TransformContextMenu";
import { MachinesPanel } from "./components/MachinesPanel";
import { AiPanel } from "./components/AiPanel";
import { CaseTabs } from "./components/CaseTabs";
import { GraphFinder } from "./components/GraphFinder";
import { SettingsPanel } from "./components/SettingsPanel";
import { RunLogPanel } from "./components/RunLogPanel";
import { ImportPanel } from "./components/ImportPanel";
import { FilterStrip } from "./components/FilterStrip";
import { CommandPalette, type PaletteAction } from "./components/CommandPalette";
import { AnnotationEditor } from "./components/AnnotationEditor";
import { BookmarksPanel } from "./components/BookmarksPanel";
import { AuditPanel } from "./components/AuditPanel";
import { detectEntityType, colorForTransform } from "./lib/detectEntityType";
import { PluginsPopover } from "./components/PluginsPopover";
import { EdgeInspector } from "./components/EdgeInspector";
import { GeoView } from "./components/GeoView";
import { TimelineView } from "./components/TimelineView";
import { ViewSwitcher, type CaseView } from "./components/ViewSwitcher";
import { TransformParamsDialog } from "./components/TransformParamsDialog";
import { NotificationsBell } from "./components/NotificationsBell";
import {
  SavedSearchesPopover,
  evaluatePredicate,
} from "./components/SavedSearchesPopover";
import { SnapshotsPanel } from "./components/SnapshotsPanel";
import type { SearchPredicate } from "./types";
import "./App.css";

interface Toast {
  id: number;
  kind: "info" | "error" | "success";
  text: string;
}

export default function App() {
  const [currentCase, setCurrentCase] = useState<Case | null>(null);
  const [cases, setCases] = useState<Case[]>([]);
  const [entities, setEntities] = useState<Entity[]>([]);
  const [edges, setEdges] = useState<Edge[]>([]);
  const [annotations, setAnnotations] = useState<Annotation[]>([]);
  const [selectedAnnotationIds, setSelectedAnnotationIds] = useState<string[]>([]);
  const [bookmarksOpen, setBookmarksOpen] = useState(false);
  const [savedSearchesOpen, setSavedSearchesOpen] = useState(false);
  /// Force performance mode on the graph canvas. `undefined` lets the
  /// canvas auto-enable past 1000 entities; setting it explicitly is
  /// a power-user override. Persisted in localStorage so a session's
  /// preference survives reload.
  const [perfModeOverride, setPerfModeOverride] = useState<boolean | null>(
    () => {
      try {
        const v = localStorage.getItem("maltrace_perf_mode");
        return v == null ? null : v === "true";
      } catch {
        return null;
      }
    },
  );
  useEffect(() => {
    try {
      if (perfModeOverride === null) {
        localStorage.removeItem("maltrace_perf_mode");
      } else {
        localStorage.setItem("maltrace_perf_mode", String(perfModeOverride));
      }
    } catch {
      /* noop */
    }
  }, [perfModeOverride]);
  /// Graph renderer. Canvas2D is the default — it renders node labels and
  /// the hover styling correctly and is already 60fps for typical graph
  /// sizes. WebGL is an opt-in for *very large* graphs (thousands of nodes)
  /// where Canvas2D pan/zoom stutters; it's Cytoscape's experimental
  /// renderer and currently drops node labels, so it's off unless asked
  /// for. (New storage key so anyone who tried the old WebGL-default build
  /// resets cleanly to Canvas2D.)
  const [webglEnabled, setWebglEnabled] = useState<boolean>(() => {
    try {
      return localStorage.getItem("maltrace_renderer") === "webgl";
    } catch {
      return false;
    }
  });
  useEffect(() => {
    try {
      localStorage.setItem(
        "maltrace_renderer",
        webglEnabled ? "webgl" : "canvas",
      );
    } catch {
      /* noop */
    }
  }, [webglEnabled]);
  /// OPSEC stealth mode — when on, every transform list / context menu
  /// hides non-`passive` transforms. The investigator opts in for
  /// surveillance / red-team work where target awareness matters.
  const [stealthOnly, setStealthOnly] = useState(false);
  const [auditOpen, setAuditOpen] = useState(false);
  const [snapshotsOpen, setSnapshotsOpen] = useState(false);
  const [canonicalTypes, setCanonicalTypes] = useState<CanonicalType[]>([]);
  const [transforms, setTransforms] = useState<TransformMeta[]>([]);
  const [transformsLoading, setTransformsLoading] = useState(true);
  const [machines, setMachines] = useState<MachineMeta[]>([]);
  // Authoritative selection set, sorted. Empty = nothing selected.
  const [selectedIds, setSelectedIds] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const [toasts, setToasts] = useState<Toast[]>([]);
  const [shortcutsOpen, setShortcutsOpen] = useState(false);
  const [welcomeOpen, setWelcomeOpen] = useState(() => {
    try {
      return localStorage.getItem(WELCOME_SEEN_KEY) !== "1";
    } catch {
      return false;
    }
  });
  const closeWelcome = useCallback(() => {
    setWelcomeOpen(false);
    try {
      localStorage.setItem(WELCOME_SEEN_KEY, "1");
    } catch {
      /* ignore — private mode */
    }
  }, []);
  const [machinesOpen, setMachinesOpen] = useState(false);
  const [aiOpen, setAiOpen] = useState(false);
  const [pluginsOpen, setPluginsOpen] = useState(false);
  const [finderOpen, setFinderOpen] = useState(false);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [importOpen, setImportOpen] = useState(false);
  const [runLogOpen, setRunLogOpen] = useState(false);
  /// Bumped after each transform/machine run so the run-log panel refreshes
  /// without having to poll.
  const [runLogRefreshKey, setRunLogRefreshKey] = useState(0);
  const [lastMachineRun, setLastMachineRun] = useState<
    { machineId: string; steps: MachineStepResult[] } | null
  >(null);
  const [contextMenu, setContextMenu] = useState<
    { x: number; y: number; entityIds: string[] } | null
  >(null);
  const [dragHover, setDragHover] = useState(false);
  const [hiddenTypes, setHiddenTypes] = useState<string[]>([]);
  const [bookmarkedOnly, setBookmarkedOnly] = useState(false);
  /// Selected edge id. Tapping an edge on the canvas fires this; the
  /// edge inspector replaces the entity inspector while non-null.
  /// Node selection (or background click) clears it back to null.
  const [selectedEdgeId, setSelectedEdgeId] = useState<string | null>(null);
  /// Which case view is active: graph (default), map, or timeline.
  /// Selection / state are shared across views — switching tabs only
  /// changes what's rendered.
  const [view, setView] = useState<CaseView>("graph");
  const [inspectorCollapsed, setInspectorCollapsed] = useState<boolean>(() => {
    try {
      return localStorage.getItem("maltrace_inspector_collapsed") === "1";
    } catch {
      return false;
    }
  });
  useEffect(() => {
    try {
      localStorage.setItem(
        "maltrace_inspector_collapsed",
        inspectorCollapsed ? "1" : "0",
      );
    } catch {
      /* noop */
    }
  }, [inspectorCollapsed]);
  /// When a transform with parameters is invoked we open a small modal
  /// to collect values before firing the actual call. `targetIds`
  /// captures whether the run came from single-select or multi-select.
  const [paramsDialog, setParamsDialog] = useState<{
    transform: TransformMeta;
    targetIds: string[];
  } | null>(null);
  /// Undo entry. The `kind` discriminator carries enough info to replay
  /// the action on redo — for transforms that means the same transform_id
  /// + target entity; for imports the list of (type, label) pairs. Some
  /// actions (uploadFiles) are undo-only because reproducing them would
  /// need to keep the raw file bytes around.
  type UndoEntry =
    | {
        kind: "addEntity";
        label: string;
        entityIds: string[];
        replay: { entityType: string; entityLabel: string };
      }
    | {
        kind: "transform";
        label: string;
        entityIds: string[];
        replay: { entityId: string; transformId: string };
      }
    | {
        kind: "transformBatch";
        label: string;
        entityIds: string[];
        replay: { entityIds: string[]; transformId: string };
      }
    | {
        kind: "runAll";
        label: string;
        entityIds: string[];
        replay: { entityId: string; transformIds: string[] };
      }
    | {
        kind: "machine";
        label: string;
        entityIds: string[];
        replay: { machineId: string; entityId: string };
      }
    | {
        kind: "import";
        label: string;
        entityIds: string[];
        replay: { items: Array<{ entityType: string; entityLabel: string }> };
      }
    | {
        /// Destructive delete — the undo action *restores* the snapshot
        /// instead of deleting. `entityIds` stays for compat (the redo
        /// path deletes by id), but the bulk of the data is in
        /// `snapshots`.
        kind: "delete";
        label: string;
        entityIds: string[];
        replay: { snapshots: import("./types").EntitySnapshot[] };
      }
    | { kind: "noRedo"; label: string; entityIds: string[] };

  const [undoStack, setUndoStack] = useState<UndoEntry[]>([]);
  /// Redo stack — populated when an undo runs, drained when redo runs,
  /// and *cleared* whenever the user performs a new action (standard
  /// undo/redo semantics — otherwise redo could resurrect entities the
  /// user already moved past).
  const [redoStack, setRedoStack] = useState<UndoEntry[]>([]);
  const pushUndo = useCallback((entry: UndoEntry) => {
    if (entry.entityIds.length === 0) return;
    // Cap stack so a long session can't bloat panel state.
    setUndoStack((cur) => [...cur.slice(-49), entry]);
    setRedoStack([]);
  }, []);
  const toggleHiddenType = useCallback((id: string) => {
    setHiddenTypes((cur) =>
      cur.includes(id) ? cur.filter((x) => x !== id) : [...cur, id],
    );
  }, []);
  const showAllTypes = useCallback(() => setHiddenTypes([]), []);
  const graphRef = useRef<GraphHandle | null>(null);
  const filterInputRef = useRef<HTMLInputElement | null>(null);
  const toastSeq = useRef(0);

  /// Lazily request notification permission the first time a slow
  /// transform completes. Browser API works in both Tauri's webview and
  /// the web build, so no extra plugin is needed. `null` when the
  /// browser doesn't expose the API at all (rare).
  const maybeNotify = useCallback((title: string, body: string) => {
    if (typeof Notification === "undefined") return;
    if (document.hasFocus()) return; // user is here, toast is enough
    const fire = () => {
      try {
        new Notification(title, { body, silent: true });
      } catch {
        /* some browsers throw on iframes / packaged apps — fall through */
      }
    };
    if (Notification.permission === "granted") {
      fire();
    } else if (Notification.permission !== "denied") {
      Notification.requestPermission().then((p) => {
        if (p === "granted") fire();
      });
    }
  }, []);

  const pushToast = useCallback((kind: Toast["kind"], text: string) => {
    const id = ++toastSeq.current;
    setToasts((prev) => [...prev, { id, kind, text }]);
    if (kind !== "error") {
      window.setTimeout(() => {
        setToasts((prev) => prev.filter((t) => t.id !== id));
      }, 3200);
    }
  }, []);
  const dismissToast = useCallback((id: number) => {
    setToasts((prev) => prev.filter((t) => t.id !== id));
  }, []);

  /// Apply a saved-search predicate: fetch tags for the case, evaluate
  /// the predicate against every entity, set the selection to the
  /// matching ids. Closes the popover on success. Selection-as-result
  /// reuses the existing visual treatment for selected nodes.
  const applySavedSearch = useCallback(
    async (predicate: SearchPredicate) => {
      if (!currentCase) return;
      try {
        const tagsByEntity =
          predicate.all_tags || predicate.any_tag
            ? await api.listTagsForCase(currentCase.id)
            : {};
        const matchIds = entities
          .filter((e) =>
            evaluatePredicate(
              { ...e, _tags: tagsByEntity[e.id] ?? [] },
              predicate,
            ),
          )
          .map((e) => e.id);
        setSelectedIds(matchIds.sort());
        setSelectedEdgeId(null);
        setSavedSearchesOpen(false);
        pushToast(
          "success",
          `Selected ${matchIds.length} matching ${
            matchIds.length === 1 ? "entity" : "entities"
          }`,
        );
      } catch (err) {
        pushToast("error", String(err));
      }
    },
    [currentCase, entities, pushToast],
  );

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const [c, ct, cs] = await Promise.all([
          api.currentCase(),
          api.listCanonicalTypes(),
          api.listCases().catch(() => [] as Case[]),
        ]);
        if (cancelled) return;
        setCurrentCase(c);
        setCanonicalTypes(ct);
        setCases(cs.length > 0 ? cs : [c]);
        const [ents, eds, anns] = await Promise.all([
          api.listEntities(c.id),
          api.listEdges(c.id),
          api.listAnnotations(c.id).catch(() => [] as Annotation[]),
        ]);
        if (cancelled) return;
        setEntities(ents);
        setEdges(eds);
        setAnnotations(anns);
        const [tfs, ms] = await Promise.all([
          api.listTransforms(),
          api.listMachines().catch(() => [] as MachineMeta[]),
        ]);
        if (cancelled) return;
        setTransforms(tfs);
        setMachines(ms);
      } catch (e) {
        if (!cancelled) pushToast("error", String(e));
      } finally {
        if (!cancelled) setTransformsLoading(false);
      }
      // Surface sidecars that failed to load (broken venv, adapter error)
      // instead of silently dropping them. Shown once on startup; wrapped so
      // a failure here can never break the rest of the app load.
      // NOTE: keyless-API-key detection is out of scope here.
      try {
        const statuses = await api.listSidecarStatus();
        const failed = statuses.filter((s) => s.ok === false);
        if (!cancelled && failed.length > 0) {
          pushToast(
            "error",
            `${failed.length} sidecar(s) failed to load: ${failed
              .map((s) => s.id)
              .join(", ")}`,
          );
        }
      } catch {
        // Non-fatal: status is a diagnostic nicety, not a hard dependency.
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [pushToast]);

  // Derived: transform_id → sidecar id (kept on edges as data but no
  // longer drives styling — see `transformColors` below).
  const transformSources = useMemo(() => {
    const m: Record<string, string> = {};
    for (const t of transforms) {
      if (t.source) m[t.id] = t.source;
    }
    return m;
  }, [transforms]);

  // Derived: transform_id → HSL colour, stable across sessions because the
  // hash is deterministic. Each individual transform gets its own line
  // colour so an investigator can tell "this edge came from sfp_whois"
  // vs "sfp_dnsresolve" at a glance, the way Maltego does.
  const transformColors = useMemo(() => {
    const m: Record<string, string> = {};
    for (const t of transforms) m[t.id] = colorForTransform(t.id);
    return m;
  }, [transforms]);

  // ─── Live updates (SSE on the web build) ────────────────────────
  // When another user mutates the same case (server-build, multi-user
  // mode), the backend publishes a `graph_changed` event over SSE; we
  // refetch the current case's entities + edges so the canvas stays
  // in sync without F5. Tauri build has no HTTP server — the API
  // wrapper short-circuits and the cleanup is a no-op.
  //
  // Per-case filter happens client-side: events carry `case_id` and
  // we ignore anything that isn't this tab's active case.
  useEffect(() => {
    if (!currentCase) return;
    const activeCaseId = currentCase.id;
    let refetchPending = false;
    const refetch = async () => {
      if (refetchPending) return;
      refetchPending = true;
      // Tiny coalesce window so a burst of events (transform run
      // with many entities) collapses to one fetch.
      await new Promise((r) => window.setTimeout(r, 150));
      refetchPending = false;
      try {
        const [ents, eds] = await Promise.all([
          api.listEntities(activeCaseId),
          api.listEdges(activeCaseId),
        ]);
        setEntities(ents);
        setEdges(eds);
      } catch {
        /* swallow — next event will try again */
      }
    };
    const cleanup = api.subscribeToEvents((event) => {
      const e = event as {
        kind?: string;
        case_id?: number;
      };
      if (
        (e.kind === "graph_changed" && e.case_id === activeCaseId) ||
        e.kind === "notification_created"
      ) {
        void refetch();
      }
    });
    return cleanup;
  }, [currentCase]);

  // ─── Case CRUD ──────────────────────────────────────────────────
  const switchCase = useCallback(
    async (caseId: number) => {
      const target = cases.find((c) => c.id === caseId);
      if (!target || target.id === currentCase?.id) return;
      setBusy(true);
      try {
        const [ents, eds, anns] = await Promise.all([
          api.listEntities(caseId),
          api.listEdges(caseId),
          api.listAnnotations(caseId).catch(() => [] as Annotation[]),
        ]);
        setCurrentCase(target);
        setEntities(ents);
        setEdges(eds);
        setAnnotations(anns);
        setSelectedIds([]);
        setSelectedAnnotationIds([]);
      } catch (err) {
        pushToast("error", String(err));
      } finally {
        setBusy(false);
      }
    },
    [cases, currentCase, pushToast],
  );

  // ─── Entity merge ──────────────────────────────────────────────
  /// Merge the secondary selected entity into the primary one. Edges from
  /// the merged side get re-pointed at the primary; data/notes/bookmark
  /// fields on the primary win.
  const mergeSelectedEntities = useCallback(async () => {
    // Look up the selection here (rather than via the memoised
    // `selectedEntities` which is declared below) so this callback can
    // live near the other top-level handlers without forcing a hoist.
    const pair = selectedIds
      .map((id) => entities.find((e) => e.id === id))
      .filter((e): e is Entity => !!e);
    if (pair.length !== 2) return;
    const [keep, drop] = pair;
    if (
      !window.confirm(
        `Merge "${drop.label}" into "${keep.label}"?\n\nEvery edge touching "${drop.label}" will be re-pointed at "${keep.label}", then "${drop.label}" will be deleted. Notes and data on "${keep.label}" are kept; the other side's are lost.`,
      )
    )
      return;
    setBusy(true);
    try {
      await api.mergeEntities(keep.id, drop.id);
      // Reload entities + edges for the case (cheaper than reconciling
      // edge-rewrites client-side).
      if (currentCase) {
        const [ents, eds] = await Promise.all([
          api.listEntities(currentCase.id),
          api.listEdges(currentCase.id),
        ]);
        setEntities(ents);
        setEdges(eds);
      }
      setSelectedIds([keep.id]);
      pushToast("success", `Merged "${drop.label}" into "${keep.label}"`);
    } catch (err) {
      pushToast("error", String(err));
    } finally {
      setBusy(false);
    }
  }, [selectedIds, entities, currentCase, pushToast]);

  // ─── Annotations ───────────────────────────────────────────────
  /// Drop a new note onto the canvas. Default position is the centre of
  /// the current viewport so the user doesn't have to hunt for it. Empty
  /// text is fine — the note opens in edit mode in the inspector.
  const addAnnotation = useCallback(
    async (atPos?: { x: number; y: number }) => {
      if (!currentCase) return;
      // Defaults to the canvas centre when no position is supplied. The
      // GraphHandle doesn't expose viewport-centre directly, so we just
      // pick (0,0) graph-space — the user can drag it after.
      const pos = atPos ?? { x: 0, y: 0 };
      try {
        const a = await api.createAnnotation(
          currentCase.id,
          "",
          pos.x,
          pos.y,
          null,
        );
        setAnnotations((prev) => [...prev, a]);
        setSelectedAnnotationIds([a.id]);
        setSelectedIds([]);
      } catch (err) {
        pushToast("error", String(err));
      }
    },
    [currentCase, pushToast],
  );

  const onAnnotationMoved = useCallback(
    (id: string, x: number, y: number) => {
      setAnnotations((prev) =>
        prev.map((a) => (a.id === id ? { ...a, pos_x: x, pos_y: y } : a)),
      );
      api
        .updateAnnotation(id, { posX: x, posY: y })
        .catch((e) => pushToast("error", String(e)));
    },
    [pushToast],
  );

  const updateAnnotationText = useCallback(
    async (id: string, text: string) => {
      // Optimistic update first.
      setAnnotations((prev) =>
        prev.map((a) => (a.id === id ? { ...a, text } : a)),
      );
      try {
        await api.updateAnnotation(id, { text });
      } catch (e) {
        pushToast("error", String(e));
      }
    },
    [pushToast],
  );

  const updateAnnotationColor = useCallback(
    async (id: string, color: string) => {
      setAnnotations((prev) =>
        prev.map((a) => (a.id === id ? { ...a, color } : a)),
      );
      try {
        await api.updateAnnotation(id, { color });
      } catch (e) {
        pushToast("error", String(e));
      }
    },
    [pushToast],
  );

  const deleteAnnotation = useCallback(
    async (id: string) => {
      try {
        await api.deleteAnnotation(id);
        setAnnotations((prev) => prev.filter((a) => a.id !== id));
        setSelectedAnnotationIds((cur) => cur.filter((x) => x !== id));
      } catch (e) {
        pushToast("error", String(e));
      }
    },
    [pushToast],
  );

  const loadSampleGraph = useCallback(async () => {
    if (!currentCase) return;
    setBusy(true);
    try {
      const res = await api.seedSampleGraph(currentCase.id);
      setEntities((prev) => {
        const byId = new Map(prev.map((p) => [p.id, p]));
        for (const e of res.entities) byId.set(e.id, e);
        return Array.from(byId.values());
      });
      setEdges((prev) => {
        const byId = new Map(prev.map((p) => [p.id, p]));
        for (const ed of res.edges) byId.set(ed.id, ed);
        return Array.from(byId.values());
      });
      // Seed counts as a creation action — make it undoable so a user
      // who clicks "load sample" can immediately Cmd+Z it away.
      pushUndo({
        kind: "noRedo",
        label: "Load sample graph",
        entityIds: res.entities.map((e) => e.id),
      });
      pushToast("success", `Loaded ${res.entities.length} sample entities`);
    } catch (err) {
      pushToast("error", String(err));
    } finally {
      setBusy(false);
    }
  }, [currentCase, pushUndo, pushToast]);

  const createCase = useCallback(async () => {
    // Generate an auto-name like "Investigation 3" from the existing count
    // so the user doesn't have to think; they can rename inline after.
    const next = cases.length + 1;
    try {
      const c = await api.createCase(`Investigation ${next}`);
      setCases((prev) => [...prev, c]);
      // Switch to the new case immediately.
      setCurrentCase(c);
      setEntities([]);
      setEdges([]);
      setAnnotations([]);
      setSelectedIds([]);
      setSelectedAnnotationIds([]);
    } catch (err) {
      pushToast("error", String(err));
    }
  }, [cases, pushToast]);

  const renameCase = useCallback(
    async (id: number, name: string) => {
      try {
        await api.renameCase(id, name);
        setCases((prev) => prev.map((c) => (c.id === id ? { ...c, name } : c)));
        if (currentCase?.id === id) {
          setCurrentCase((cur) => (cur ? { ...cur, name } : cur));
        }
      } catch (err) {
        pushToast("error", String(err));
      }
    },
    [currentCase, pushToast],
  );

  const deleteCase = useCallback(
    async (id: number) => {
      try {
        await api.deleteCase(id);
        setCases((prev) => {
          const next = prev.filter((c) => c.id !== id);
          // Drop selection if we're killing the active case; let the next
          // case become the active one.
          if (currentCase?.id === id && next.length > 0) {
            const newActive = next[0];
            setCurrentCase(newActive);
            Promise.all([
              api.listEntities(newActive.id),
              api.listEdges(newActive.id),
            ])
              .then(([ents, eds]) => {
                setEntities(ents);
                setEdges(eds);
                setSelectedIds([]);
              })
              .catch((e) => pushToast("error", String(e)));
          }
          return next;
        });
      } catch (err) {
        pushToast("error", String(err));
      }
    },
    [currentCase, pushToast],
  );

  const toggleCaseLock = useCallback(
    async (id: number, next: boolean) => {
      try {
        const updated = await api.setCaseLocked(id, next);
        setCases((prev) =>
          prev.map((c) => (c.id === id ? { ...c, ...updated } : c)),
        );
        if (currentCase?.id === id) {
          setCurrentCase((cur) => (cur ? { ...cur, ...updated } : cur));
        }
        pushToast(
          "success",
          next ? `Locked "${updated.name}"` : `Unlocked "${updated.name}"`,
        );
      } catch (err) {
        pushToast("error", String(err));
      }
    },
    [currentCase, pushToast],
  );

  /// True when the active case is locked. Threaded down into every
  /// affordance that mutates the graph (inspector buttons, context
  /// menu, add-entity form, drop-to-upload) so the UI hides write
  /// operations rather than letting them fail at the backend.
  const caseLocked = !!currentCase?.locked;

  const downloadBlob = useCallback(
    (filename: string, content: string, mime: string) => {
      const blob = new Blob([content], { type: mime });
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = filename;
      document.body.appendChild(a);
      a.click();
      document.body.removeChild(a);
      URL.revokeObjectURL(url);
    },
    [],
  );

  const exportCaseGraphml = useCallback(
    async (id: number) => {
      try {
        const xml = await api.exportCaseGraphml(id);
        const name = cases.find((c) => c.id === id)?.name ?? "case";
        const safe = name.replace(/[^\w.-]+/g, "_") || "case";
        downloadBlob(`${safe}.graphml`, xml, "application/xml");
        pushToast("success", `Exported ${safe}.graphml`);
      } catch (err) {
        pushToast("error", String(err));
      }
    },
    [cases, pushToast, downloadBlob],
  );

  const exportCasePng = useCallback(
    (id: number) => {
      const dataUrl = graphRef.current?.exportPng() ?? null;
      if (!dataUrl) {
        pushToast("info", "Nothing on the canvas to export.");
        return;
      }
      const name = cases.find((c) => c.id === id)?.name ?? "case";
      const safe = name.replace(/[^\w.-]+/g, "_") || "case";
      // dataUrl is `data:image/png;base64,...`; convert to a real Blob so
      // the download lands as a binary PNG, not a base64 text blob.
      const commaIdx = dataUrl.indexOf(",");
      const b64 = commaIdx >= 0 ? dataUrl.slice(commaIdx + 1) : dataUrl;
      const bytes = Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));
      const blob = new Blob([bytes], { type: "image/png" });
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = `${safe}.png`;
      document.body.appendChild(a);
      a.click();
      document.body.removeChild(a);
      URL.revokeObjectURL(url);
      pushToast("success", `Exported ${safe}.png`);
    },
    [cases, pushToast],
  );

  /// Wrap the same `cy.png()` output in a single-page landscape PDF with
  /// a small header (case name + entity count + date). Dynamic-imported
  /// so the jspdf bundle only loads when actually exporting.
  const exportCasePdf = useCallback(
    async (id: number) => {
      const dataUrl = graphRef.current?.exportPng() ?? null;
      if (!dataUrl) {
        pushToast("info", "Nothing on the canvas to export.");
        return;
      }
      const caseName = cases.find((c) => c.id === id)?.name ?? "case";
      const safe = caseName.replace(/[^\w.-]+/g, "_") || "case";
      try {
        const { jsPDF } = await import("jspdf");
        const pdf = new jsPDF({ orientation: "landscape", unit: "pt" });
        const pageW = pdf.internal.pageSize.getWidth();
        const pageH = pdf.internal.pageSize.getHeight();
        const margin = 36;
        // Header.
        pdf.setFontSize(14);
        pdf.text(caseName, margin, margin);
        pdf.setFontSize(9);
        pdf.setTextColor(120);
        pdf.text(
          `${entities.length} entities · ${edges.length} edges · ${new Date().toLocaleString()}`,
          margin,
          margin + 14,
        );
        pdf.setTextColor(0);
        // Compute the largest size for the image that fits in the page
        // below the header, preserving the PNG's aspect ratio.
        const img = new Image();
        await new Promise<void>((res, rej) => {
          img.onload = () => res();
          img.onerror = () => rej(new Error("PNG decode failed"));
          img.src = dataUrl;
        });
        const headerH = 36;
        const availW = pageW - margin * 2;
        const availH = pageH - margin * 2 - headerH;
        const scale = Math.min(availW / img.width, availH / img.height);
        const drawW = img.width * scale;
        const drawH = img.height * scale;
        const drawX = (pageW - drawW) / 2;
        const drawY = margin + headerH + (availH - drawH) / 2;
        pdf.addImage(dataUrl, "PNG", drawX, drawY, drawW, drawH);
        pdf.save(`${safe}.pdf`);
        pushToast("success", `Exported ${safe}.pdf`);
      } catch (err) {
        pushToast("error", String(err));
      }
    },
    [cases, edges.length, entities.length, pushToast],
  );

  const exportCaseCsv = useCallback(
    async (id: number) => {
      try {
        const csv = await api.exportCaseEntitiesCsv(id);
        const name = cases.find((c) => c.id === id)?.name ?? "case";
        const safe = name.replace(/[^\w.-]+/g, "_") || "case";
        downloadBlob(`${safe}.csv`, csv, "text/csv;charset=utf-8");
        pushToast("success", `Exported ${safe}.csv`);
      } catch (err) {
        pushToast("error", String(err));
      }
    },
    [cases, pushToast, downloadBlob],
  );

  const exportCaseStix = useCallback(
    async (id: number) => {
      try {
        const json = await api.exportCaseStix(id);
        const name = cases.find((c) => c.id === id)?.name ?? "case";
        const safe = name.replace(/[^\w.-]+/g, "_") || "case";
        downloadBlob(`${safe}-stix.json`, json, "application/json");
        pushToast("success", `Exported ${safe}-stix.json`);
      } catch (err) {
        pushToast("error", String(err));
      }
    },
    [cases, pushToast, downloadBlob],
  );

  const exportCaseReport = useCallback(
    async (id: number) => {
      try {
        const html = await api.exportCaseReportHtml(id);
        const name = cases.find((c) => c.id === id)?.name ?? "case";
        const safe = name.replace(/[^\w.-]+/g, "_") || "case";
        downloadBlob(`${safe}-report.html`, html, "text/html;charset=utf-8");
        pushToast("success", `Exported ${safe}-report.html`);
      } catch (err) {
        pushToast("error", String(err));
      }
    },
    [cases, pushToast, downloadBlob],
  );

  const exportCaseGeoJson = useCallback(
    async (id: number) => {
      try {
        const json = await api.exportCaseGeoJson(id);
        const name = cases.find((c) => c.id === id)?.name ?? "case";
        const safe = name.replace(/[^\w.-]+/g, "_") || "case";
        downloadBlob(`${safe}.geojson`, json, "application/geo+json");
        pushToast("success", `Exported ${safe}.geojson`);
      } catch (err) {
        pushToast("error", String(err));
      }
    },
    [cases, pushToast, downloadBlob],
  );

  const exportCaseKml = useCallback(
    async (id: number) => {
      try {
        const kml = await api.exportCaseKml(id);
        const name = cases.find((c) => c.id === id)?.name ?? "case";
        const safe = name.replace(/[^\w.-]+/g, "_") || "case";
        downloadBlob(`${safe}.kml`, kml, "application/vnd.google-earth.kml+xml");
        pushToast("success", `Exported ${safe}.kml`);
      } catch (err) {
        pushToast("error", String(err));
      }
    },
    [cases, pushToast, downloadBlob],
  );

  /// Export the whole case (entities + edges + tags + annotations +
  /// audit + runs + files) as a single `.tar.gz` so it can be handed
  /// off / archived.
  const exportCaseBackup = useCallback(
    async (id: number) => {
      try {
        const res = await api.exportCaseBackup(id);
        const name = cases.find((c) => c.id === id)?.name ?? "case";
        const safe = name.replace(/[^\w.-]+/g, "_") || "case";
        // Build the tar.gz blob from the base64 payload and trigger
        // a regular file download. Doing it client-side keeps the
        // bytes on the user's machine — the backend never streams
        // an attachment URL.
        const bytes = Uint8Array.from(atob(res.bytes_base64), (c) =>
          c.charCodeAt(0),
        );
        const blob = new Blob([bytes], { type: "application/gzip" });
        const url = URL.createObjectURL(blob);
        const a = document.createElement("a");
        a.href = url;
        a.download = `${safe}.tar.gz`;
        document.body.appendChild(a);
        a.click();
        document.body.removeChild(a);
        URL.revokeObjectURL(url);
        pushToast(
          "success",
          `Exported ${safe}.tar.gz (${res.manifest.entity_count} entities, ${res.manifest.file_count} files)`,
        );
      } catch (err) {
        pushToast("error", String(err));
      }
    },
    [cases, pushToast],
  );

  /// Pick a `.tar.gz` and restore it into a brand-new case.
  const restoreCaseBackup = useCallback(() => {
    const input = document.createElement("input");
    input.type = "file";
    input.accept = ".tar.gz,.tgz,application/gzip,application/x-gzip";
    input.onchange = async () => {
      const f = input.files?.[0];
      if (!f) return;
      try {
        const dataUrl = await new Promise<string>((resolve, reject) => {
          const r = new FileReader();
          r.onload = () => resolve(String(r.result ?? ""));
          r.onerror = () => reject(r.error ?? new Error("read failed"));
          r.readAsDataURL(f);
        });
        const res = await api.restoreCaseBackup(dataUrl);
        // Refresh the case list and jump to the restored case.
        try {
          const cs = await api.listCases();
          setCases(cs);
          const newCase = cs.find((c) => c.id === res.new_case_id);
          if (newCase) {
            setCurrentCase(newCase);
            const [ents, eds] = await Promise.all([
              api.listEntities(res.new_case_id),
              api.listEdges(res.new_case_id),
            ]);
            setEntities(ents);
            setEdges(eds);
            setSelectedIds([]);
          }
        } catch {
          /* keep going — restore succeeded, just couldn't switch */
        }
        pushToast(
          "success",
          `Restored "${res.new_case_name}" (${res.entity_count} entities, ${res.edge_count} edges)`,
        );
      } catch (err) {
        pushToast("error", String(err));
      }
    };
    input.click();
  }, [pushToast]);

  /// Splice imported entities into local state. Called from ImportPanel after
  /// it has POSTed each entity through the backend.
  const onImported = useCallback(
    (added: Entity[]) => {
      if (added.length === 0) return;
      setEntities((prev) => {
        const byId = new Map(prev.map((p) => [p.id, p]));
        for (const e of added) byId.set(e.id, e);
        return Array.from(byId.values());
      });
      pushUndo({
        kind: "import",
        label: `Import ${added.length}`,
        entityIds: added.map((e) => e.id),
        replay: {
          items: added.map((e) => ({
            entityType: e.entity_type,
            entityLabel: e.label,
          })),
        },
      });
      pushToast(
        "success",
        `Imported ${added.length} ${added.length === 1 ? "entity" : "entities"}`,
      );
    },
    [pushToast, pushUndo],
  );

  /// Upload a list of File objects: each becomes a new Image/File entity in
  /// the current case. Drop-anywhere on the canvas + a topbar Upload button
  /// both call this.
  const uploadFiles = useCallback(
    async (fileList: FileList | File[]) => {
      if (!currentCase) return;
      const files = Array.from(fileList);
      if (files.length === 0) return;
      setBusy(true);
      const added: Entity[] = [];
      try {
        for (const f of files) {
          // FileReader.readAsDataURL gives us `data:<mime>;base64,...`; the
          // backend accepts either form (it strips the prefix).
          const dataUrl = await new Promise<string>((resolve, reject) => {
            const r = new FileReader();
            r.onload = () => resolve(String(r.result ?? ""));
            r.onerror = () => reject(r.error ?? new Error("file read failed"));
            r.readAsDataURL(f);
          });
          const e = await api.uploadFileToCase(currentCase.id, f.name, dataUrl);
          added.push(e);
        }
        if (added.length > 0) {
          setEntities((prev) => {
            const byId = new Map(prev.map((p) => [p.id, p]));
            for (const e of added) byId.set(e.id, e);
            return Array.from(byId.values());
          });
          setSelectedIds(added.length === 1 ? [added[0].id] : added.map((e) => e.id));
          // Uploads aren't redoable — the raw file bytes are gone after
          // the upload; replaying would just create empty placeholders.
          // Undo still works (deletes the entities + on-disk file).
          pushUndo({
            kind: "noRedo",
            label:
              added.length === 1
                ? `Upload ${added[0].label}`
                : `Upload ${added.length} files`,
            entityIds: added.map((e) => e.id),
          });
          pushToast(
            "success",
            added.length === 1
              ? `Uploaded ${added[0].label}`
              : `Uploaded ${added.length} files`,
          );
        }
      } catch (err) {
        pushToast("error", String(err));
      } finally {
        setBusy(false);
      }
    },
    [currentCase, pushToast, pushUndo],
  );

  /// File-picker entry point (topbar Upload button).
  const pickAndUpload = useCallback(() => {
    if (!currentCase) return;
    const input = document.createElement("input");
    input.type = "file";
    input.multiple = true;
    input.accept = "image/*,.pdf,.txt,.csv,.json,.html,.xml,.zip,.log";
    input.onchange = () => {
      if (input.files && input.files.length > 0) {
        uploadFiles(input.files);
      }
    };
    input.click();
  }, [currentCase, uploadFiles]);

  const runLayout = useCallback((kind: LayoutKind) => {
    graphRef.current?.runLayout(kind);
  }, []);

  // ─── Selection tools ───────────────────────────────────────────
  /// Adjacency snapshot, rebuilt only when edges change. Each entry maps
  /// an entity id to the set of entities one hop away (in either
  /// direction). Investigators reach for "neighbors of this" + "connected
  /// component of this" constantly — caching cheaply makes both O(touched).
  const adjacency = useMemo(() => {
    const m = new Map<string, Set<string>>();
    const add = (a: string, b: string) => {
      let set = m.get(a);
      if (!set) {
        set = new Set();
        m.set(a, set);
      }
      set.add(b);
    };
    for (const e of edges) {
      add(e.source_id, e.target_id);
      add(e.target_id, e.source_id);
    }
    return m;
  }, [edges]);

  const selectNeighbors = useCallback(() => {
    if (selectedIds.length === 0) return;
    const next = new Set(selectedIds);
    for (const id of selectedIds) {
      const neigh = adjacency.get(id);
      if (neigh) for (const n of neigh) next.add(n);
    }
    setSelectedIds(Array.from(next).sort());
  }, [selectedIds, adjacency]);

  const selectConnectedComponent = useCallback(() => {
    if (selectedIds.length === 0) return;
    // BFS from each seed across the undirected adjacency.
    const seen = new Set<string>(selectedIds);
    const queue = [...selectedIds];
    while (queue.length > 0) {
      const id = queue.shift()!;
      const neigh = adjacency.get(id);
      if (!neigh) continue;
      for (const n of neigh) {
        if (!seen.has(n)) {
          seen.add(n);
          queue.push(n);
        }
      }
    }
    setSelectedIds(Array.from(seen).sort());
  }, [selectedIds, adjacency]);

  const selectByType = useCallback(
    (typeId: string) => {
      const ids = entities
        .filter((e) => e.entity_type === typeId)
        .map((e) => e.id)
        .sort();
      setSelectedIds(ids);
    },
    [entities],
  );

  const invertSelection = useCallback(() => {
    const cur = new Set(selectedIds);
    setSelectedIds(
      entities
        .filter((e) => !cur.has(e.id))
        .map((e) => e.id)
        .sort(),
    );
  }, [selectedIds, entities]);

  const undo = useCallback(async () => {
    if (undoStack.length === 0) return;
    const last = undoStack[undoStack.length - 1];
    setBusy(true);
    try {
      if (last.kind === "delete") {
        // Destructive undo: replay the snapshot back into the DB and
        // splice the resurrected entities + edges into local state.
        const restored = await api.restoreEntities(last.replay.snapshots);
        const restoredEdges = last.replay.snapshots.flatMap(
          (s) => s.incident_edges,
        );
        setEntities((prev) => {
          const byId = new Map(prev.map((p) => [p.id, p]));
          for (const e of restored) byId.set(e.id, e);
          return Array.from(byId.values());
        });
        setEdges((prev) => {
          const byId = new Map(prev.map((p) => [p.id, p]));
          for (const e of restoredEdges) byId.set(e.id, e);
          return Array.from(byId.values());
        });
      } else {
        // Constructive undo: bulk-delete what the action created.
        // Backend handles edge + on-disk-file cleanup via cascade.
        await Promise.all(last.entityIds.map((id) => api.deleteEntity(id)));
        const idSet = new Set(last.entityIds);
        setEntities((prev) => prev.filter((e) => !idSet.has(e.id)));
        setEdges((prev) =>
          prev.filter(
            (ed) => !idSet.has(ed.source_id) && !idSet.has(ed.target_id),
          ),
        );
        setSelectedIds((cur) => cur.filter((id) => !idSet.has(id)));
      }
      setUndoStack((cur) => cur.slice(0, -1));
      // Push to redo only when this action knows how to replay itself.
      // No-redo entries (uploads) are simply discarded — undo deleted
      // them and there's nothing to put back.
      if (last.kind !== "noRedo") {
        setRedoStack((cur) => [...cur.slice(-49), last]);
      }
      pushToast("info", `Undone: ${last.label}`);
    } catch (err) {
      pushToast("error", String(err));
    } finally {
      setBusy(false);
    }
  }, [undoStack, pushToast]);

  /// Apply a transform-run result to the local graph state: insert new
  /// entities/edges (skipping dupes by id), drop any entities the backend
  /// declares removed (e.g. a Collection that was just expanded). Also
  /// pings the run-log panel so it refreshes if open.
  const mergeRunResult = useCallback((res: TransformRunResult) => {
    setRunLogRefreshKey((k) => k + 1);
    setEntities((prev) => {
      const byId = new Map(prev.map((p) => [p.id, p]));
      for (const r of res.removed_entity_ids) byId.delete(r);
      for (const e of res.new_entities) byId.set(e.id, e);
      return Array.from(byId.values());
    });
    setEdges((prev) => {
      const removedIds = new Set(res.removed_entity_ids);
      const next = removedIds.size
        ? prev.filter(
            (ed) =>
              !removedIds.has(ed.source_id) && !removedIds.has(ed.target_id),
          )
        : prev.slice();
      const byId = new Map(next.map((p) => [p.id, p]));
      for (const ed of res.new_edges) byId.set(ed.id, ed);
      return Array.from(byId.values());
    });
  }, []);

  /// Redo: pop the redo stack, re-execute the action by kind. The action
  /// hits the backend exactly the same way it did the first time, so for
  /// non-idempotent calls (live transforms) the results may differ from
  /// the original — that's expected and consistent with how Maltego
  /// handles it ("redo" is "do again", not "restore the old result").
  const redo = useCallback(async () => {
    if (redoStack.length === 0) return;
    const entry = redoStack[redoStack.length - 1];
    setRedoStack((cur) => cur.slice(0, -1));
    setBusy(true);
    try {
      switch (entry.kind) {
        case "addEntity": {
          if (!currentCase) break;
          const e = await api.addEntity(
            currentCase.id,
            entry.replay.entityType,
            entry.replay.entityLabel,
            {
              x: Math.random() * 200 - 100,
              y: Math.random() * 200 - 100,
            },
          );
          setEntities((prev) =>
            prev.some((p) => p.id === e.id) ? prev : [...prev, e],
          );
          setSelectedIds([e.id]);
          // Re-record into undo so the user can chain undo→redo→undo.
          setUndoStack((cur) => [
            ...cur.slice(-49),
            {
              kind: "addEntity",
              label: entry.label,
              entityIds: [e.id],
              replay: entry.replay,
            },
          ]);
          break;
        }
        case "transform": {
          const res = await api.runTransform(
            entry.replay.entityId,
            entry.replay.transformId,
          );
          mergeRunResult(res);
          setUndoStack((cur) => [
            ...cur.slice(-49),
            {
              kind: "transform",
              label: entry.label,
              entityIds: res.new_entities.map((e) => e.id),
              replay: entry.replay,
            },
          ]);
          break;
        }
        case "transformBatch": {
          const results = await api.runTransformBatch(
            entry.replay.entityIds,
            entry.replay.transformId,
          );
          const aggIds: string[] = [];
          for (const r of results) {
            mergeRunResult(r);
            for (const e of r.new_entities) aggIds.push(e.id);
          }
          setUndoStack((cur) => [
            ...cur.slice(-49),
            {
              kind: "transformBatch",
              label: entry.label,
              entityIds: aggIds,
              replay: entry.replay,
            },
          ]);
          break;
        }
        case "runAll": {
          const results = await api.runAllTransforms(
            entry.replay.entityId,
            entry.replay.transformIds,
          );
          const aggIds: string[] = [];
          for (const r of results) {
            mergeRunResult(r);
            for (const e of r.new_entities) aggIds.push(e.id);
          }
          setUndoStack((cur) => [
            ...cur.slice(-49),
            {
              kind: "runAll",
              label: entry.label,
              entityIds: aggIds,
              replay: entry.replay,
            },
          ]);
          break;
        }
        case "machine": {
          const res = await api.runMachine(
            entry.replay.machineId,
            entry.replay.entityId,
          );
          mergeRunResult({
            source_entity: res.source_entity,
            new_entities: res.new_entities,
            new_edges: res.new_edges,
            transform_id: entry.replay.machineId,
            skipped: [],
            removed_entity_ids: res.removed_entity_ids,
          });
          setUndoStack((cur) => [
            ...cur.slice(-49),
            {
              kind: "machine",
              label: entry.label,
              entityIds: res.new_entities.map((e) => e.id),
              replay: entry.replay,
            },
          ]);
          break;
        }
        case "import": {
          if (!currentCase) break;
          const created: Entity[] = [];
          for (const it of entry.replay.items) {
            const e = await api.addEntity(
              currentCase.id,
              it.entityType,
              it.entityLabel,
              { x: Math.random() * 200 - 100, y: Math.random() * 200 - 100 },
            );
            created.push(e);
          }
          setEntities((prev) => {
            const byId = new Map(prev.map((p) => [p.id, p]));
            for (const e of created) byId.set(e.id, e);
            return Array.from(byId.values());
          });
          setUndoStack((cur) => [
            ...cur.slice(-49),
            {
              kind: "import",
              label: entry.label,
              entityIds: created.map((e) => e.id),
              replay: entry.replay,
            },
          ]);
          break;
        }
        case "noRedo":
          // Unreachable — noRedo entries are filtered out of redoStack.
          break;
      }
      pushToast("info", `Redone: ${entry.label}`);
    } catch (err) {
      pushToast("error", String(err));
    } finally {
      setBusy(false);
    }
  }, [redoStack, currentCase, mergeRunResult, pushToast]);


  const entityById = useMemo(() => {
    const m = new Map<string, Entity>();
    for (const e of entities) m.set(e.id, e);
    return m;
  }, [entities]);

  const selectedEntities = useMemo(
    () =>
      selectedIds
        .map((id) => entityById.get(id))
        .filter((e): e is Entity => !!e),
    [selectedIds, entityById],
  );

  const primaryEntity = selectedEntities[0] ?? null;

  const contextMenuEntities = useMemo(
    () =>
      contextMenu
        ? contextMenu.entityIds
            .map((id) => entityById.get(id))
            .filter((e): e is Entity => !!e)
        : [],
    [contextMenu, entityById],
  );

  /// Open the floating context menu. If the right-clicked node is already
  /// in the selection, act on the whole selection; otherwise switch
  /// selection to just it and act on that.
  const openContextMenu = useCallback(
    (entityId: string, x: number, y: number) => {
      setSelectedIds((current) => {
        if (current.includes(entityId)) {
          setContextMenu({ entityIds: [...current], x, y });
          return current;
        }
        setContextMenu({ entityIds: [entityId], x, y });
        return [entityId];
      });
    },
    [],
  );
  const closeContextMenu = useCallback(() => setContextMenu(null), []);

  const addEntity = useCallback(
    async (entityType: string, label: string) => {
      if (!currentCase) return;
      try {
        const e = await api.addEntity(currentCase.id, entityType, label, {
          x: Math.random() * 200 - 100,
          y: Math.random() * 200 - 100,
        });
        setEntities((prev) => {
          if (prev.some((p) => p.id === e.id)) return prev;
          return [...prev, e];
        });
        setSelectedIds([e.id]);
        pushUndo({
          kind: "addEntity",
          label: `Add ${e.entity_type} "${e.label}"`,
          entityIds: [e.id],
          replay: { entityType: e.entity_type, entityLabel: e.label },
        });
      } catch (err) {
        pushToast("error", String(err));
      }
    },
    [currentCase, pushToast, pushUndo],
  );

  const onPositionChange = useCallback(
    (id: string, x: number, y: number) => {
      setEntities((prev) =>
        prev.map((e) => (e.id === id ? { ...e, pos_x: x, pos_y: y } : e)),
      );
      api.updateEntityPosition(id, x, y).catch((e) => pushToast("error", String(e)));
    },
    [pushToast],
  );

  const deleteSelected = useCallback(async () => {
    if (selectedEntities.length === 0) return;
    const targets = selectedEntities;
    try {
      // Snapshot every target (+ incident edges + tags) before the
      // delete so undo can restore byte-for-byte. Runs in parallel
      // because each snapshot is independent.
      const snapshots = await Promise.all(
        targets.map((t) => api.snapshotEntity(t.id)),
      );
      await Promise.all(targets.map((t) => api.deleteEntity(t.id)));
      const idSet = new Set(targets.map((t) => t.id));
      setEntities((prev) => prev.filter((e) => !idSet.has(e.id)));
      setEdges((prev) =>
        prev.filter(
          (ed) => !idSet.has(ed.source_id) && !idSet.has(ed.target_id),
        ),
      );
      setSelectedIds([]);
      pushUndo({
        kind: "delete",
        label:
          targets.length === 1
            ? `Delete ${targets[0].label}`
            : `Delete ${targets.length} entities`,
        entityIds: targets.map((t) => t.id),
        replay: { snapshots },
      });
      pushToast(
        "success",
        targets.length === 1
          ? `Deleted ${targets[0].label}`
          : `Deleted ${targets.length} entities`,
      );
    } catch (err) {
      pushToast("error", String(err));
    }
  }, [selectedEntities, pushToast, pushUndo]);

  /// Actually fire the transform. Split out from the entry-point
  /// `runTransform` so the parameter dialog can call straight into it
  /// after collecting values. `params` is the user's parameter map (or
  /// null for the no-params fast path).
  const fireTransform = useCallback(
    async (
      transformId: string,
      ids: string[],
      params: Record<string, unknown> | null,
    ) => {
      setBusy(true);
      pushToast(
        "info",
        ids.length === 1
          ? `Running ${transformId} on ${primaryEntity?.label ?? "entity"}…`
          : `Running ${transformId} on ${ids.length} entities…`,
      );
      // Capture start time so we can decide whether to fire a system
      // notification on completion (only when the run was slow enough
      // that the user likely tabbed away).
      const t0 = performance.now();
      try {
        if (ids.length === 1) {
          const res = params
            ? await api.runTransformWithParams(ids[0], transformId, params)
            : await api.runTransform(ids[0], transformId);
          mergeRunResult(res);
          pushUndo({
            kind: "transform",
            label: `Run ${transformId}`,
            entityIds: res.new_entities.map((e) => e.id),
            replay: { entityId: ids[0], transformId },
          });
          pushToast(
            res.new_entities.length === 0 ? "info" : "success",
            res.new_entities.length === 0
              ? `${transformId}: no results`
              : `${transformId}: +${res.new_entities.length} ${
                  res.new_entities.length === 1 ? "entity" : "entities"
                }`,
          );
        } else {
          // Batch + params: fire one-by-one so the params reach each
          // call. Sequential, like batch transforms already are.
          const results = params
            ? await Promise.all(
                ids.map((id) =>
                  api.runTransformWithParams(id, transformId, params),
                ),
              )
            : await api.runTransformBatch(ids, transformId);
          let totalNew = 0;
          const aggIds: string[] = [];
          for (const r of results) {
            mergeRunResult(r);
            totalNew += r.new_entities.length;
            for (const e of r.new_entities) aggIds.push(e.id);
          }
          pushUndo({
            kind: "transformBatch",
            label: `Run ${transformId} × ${ids.length}`,
            entityIds: aggIds,
            replay: { entityIds: ids, transformId },
          });
          pushToast(
            totalNew === 0 ? "info" : "success",
            totalNew === 0
              ? `${transformId} × ${ids.length}: no results`
              : `${transformId} × ${ids.length}: +${totalNew} ${
                  totalNew === 1 ? "entity" : "entities"
                }`,
          );
        }
      } catch (err) {
        pushToast("error", String(err));
      } finally {
        setBusy(false);
        // Slow runs ping the OS-level notification too, so a user who
        // tabbed away comes back informed. Threshold matches "long
        // enough that someone gives up watching" — not too short or
        // every run becomes annoying.
        if (performance.now() - t0 > 5000) {
          maybeNotify(
            "Transform finished",
            ids.length === 1
              ? `${transformId} completed`
              : `${transformId} × ${ids.length} completed`,
          );
        }
      }
    },
    [primaryEntity, pushToast, mergeRunResult, pushUndo, maybeNotify],
  );

  /// Entry point used by inspector + context menu + palette. Opens the
  /// parameter dialog when the transform declares any; otherwise fires
  /// straight through to `fireTransform`.
  const runTransform = useCallback(
    (transformId: string, targetIds?: string[]) => {
      const ids = targetIds ?? selectedIds;
      if (ids.length === 0) return;
      const meta = transforms.find((t) => t.id === transformId);
      if (meta && meta.parameters && meta.parameters.length > 0) {
        setParamsDialog({ transform: meta, targetIds: ids });
        return;
      }
      void fireTransform(transformId, ids, null);
    },
    [selectedIds, transforms, fireTransform],
  );

  /// Run every compatible transform on one entity, sequentially. The
  /// backend swallows per-transform failures so a single rate-limited
  /// service doesn't stop the others; the run-log panel surfaces them.
  const runAllTransforms = useCallback(
    async (entityId: string, transformIds: string[]) => {
      if (transformIds.length === 0) return;
      const entity = entityById.get(entityId);
      setBusy(true);
      pushToast(
        "info",
        `Running ${transformIds.length} transforms on ${entity?.label ?? "entity"}…`,
      );
      try {
        const results = await api.runAllTransforms(entityId, transformIds);
        let totalNew = 0;
        const aggIds: string[] = [];
        for (const r of results) {
          mergeRunResult(r);
          totalNew += r.new_entities.length;
          for (const e of r.new_entities) aggIds.push(e.id);
        }
        pushUndo({
          kind: "runAll",
          label: `Run all (${results.length})`,
          entityIds: aggIds,
          replay: { entityId, transformIds },
        });
        pushToast(
          totalNew === 0 ? "info" : "success",
          totalNew === 0
            ? `Run all: ${results.length} transforms ran, no new entities (check the run log)`
            : `Run all: +${totalNew} ${totalNew === 1 ? "entity" : "entities"} from ${results.length} transforms`,
        );
      } catch (err) {
        pushToast("error", String(err));
      } finally {
        setBusy(false);
      }
    },
    [entityById, mergeRunResult, pushToast, pushUndo],
  );


  const expandCollection = useCallback(
    async (collectionId: string) => {
      setBusy(true);
      pushToast("info", "Expanding collection…");
      try {
        const res = await api.expandCollection(collectionId);
        mergeRunResult(res);
        // Drop the collection from selection.
        setSelectedIds((cur) => cur.filter((id) => id !== collectionId));
        pushToast(
          "success",
          `Expanded into ${res.new_entities.length} ${
            res.new_entities.length === 1 ? "entity" : "entities"
          }`,
        );
      } catch (err) {
        pushToast("error", String(err));
      } finally {
        setBusy(false);
      }
    },
    [pushToast, mergeRunResult],
  );

  const runMachine = useCallback(
    async (machineId: string) => {
      if (!primaryEntity) return;
      const machine = machines.find((m) => m.id === machineId);
      const t0 = performance.now();
      setBusy(true);
      pushToast(
        "info",
        `Running ${machine?.name ?? machineId} on ${primaryEntity.label}…`,
      );
      try {
        const res = await api.runMachine(machineId, primaryEntity.id);
        // Reuse mergeRunResult by reshaping; machine result has same shape
        // for the bits that matter (new_entities, new_edges, removed).
        mergeRunResult({
          source_entity: res.source_entity,
          new_entities: res.new_entities,
          new_edges: res.new_edges,
          transform_id: machineId,
          skipped: [],
          removed_entity_ids: res.removed_entity_ids,
        });
        pushUndo({
          kind: "machine",
          label: `Machine ${machine?.name ?? machineId}`,
          entityIds: res.new_entities.map((e) => e.id),
          replay: { machineId, entityId: primaryEntity.id },
        });
        setLastMachineRun({ machineId: res.machine_id, steps: res.steps });
        const totalFailures = res.steps.reduce(
          (n, s) => n + s.failures.length,
          0,
        );
        pushToast(
          res.new_entities.length === 0 ? "info" : "success",
          `Machine ${machine?.name ?? machineId}: +${res.new_entities.length} ${
            res.new_entities.length === 1 ? "entity" : "entities"
          }${totalFailures > 0 ? ` (${totalFailures} step failures)` : ""}`,
        );
      } catch (err) {
        pushToast("error", String(err));
      } finally {
        setBusy(false);
        if (performance.now() - t0 > 5000) {
          maybeNotify(
            "Machine finished",
            `${machine?.name ?? machineId} completed`,
          );
        }
      }
    },
    [primaryEntity, machines, pushToast, mergeRunResult, pushUndo, maybeNotify],
  );

  // Global keyboard shortcuts.
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      const target = e.target as HTMLElement | null;
      const inEditable =
        target &&
        (target.tagName === "INPUT" ||
          target.tagName === "TEXTAREA" ||
          target.isContentEditable);

      if (e.key === "?" && !inEditable) {
        setShortcutsOpen((o) => !o);
        e.preventDefault();
        return;
      }
      // Ctrl+F / Cmd+F → find-on-graph overlay. Capture even inside text
      // inputs so the system's browser find doesn't fight us.
      if ((e.ctrlKey || e.metaKey) && (e.key === "f" || e.key === "F")) {
        e.preventDefault();
        setFinderOpen(true);
        return;
      }
      // Ctrl+K / Cmd+K → command palette. Same precedence over inputs.
      if ((e.ctrlKey || e.metaKey) && (e.key === "k" || e.key === "K")) {
        e.preventDefault();
        setPaletteOpen(true);
        return;
      }
      // Ctrl+Z / Cmd+Z → undo. Skip when an editable element is focused so
      // it doesn't fight the native text-editing undo in inputs.
      if (
        (e.ctrlKey || e.metaKey) &&
        !e.shiftKey &&
        (e.key === "z" || e.key === "Z") &&
        !inEditable
      ) {
        e.preventDefault();
        undo();
        return;
      }
      // Redo: Cmd+Shift+Z (mac convention) or Ctrl+Y (Windows convention).
      if (
        !inEditable &&
        ((e.ctrlKey || e.metaKey) && e.shiftKey && (e.key === "z" || e.key === "Z")) ||
        ((e.ctrlKey || e.metaKey) && (e.key === "y" || e.key === "Y") && !inEditable)
      ) {
        e.preventDefault();
        redo();
        return;
      }
      if (e.key === "Escape") {
        if (paletteOpen) {
          setPaletteOpen(false);
          return;
        }
        if (bookmarksOpen) {
          setBookmarksOpen(false);
          return;
        }
        if (auditOpen) {
          setAuditOpen(false);
          return;
        }
        if (finderOpen) {
          setFinderOpen(false);
          return;
        }
        if (settingsOpen) {
          setSettingsOpen(false);
          return;
        }
        if (importOpen) {
          setImportOpen(false);
          return;
        }
        if (shortcutsOpen) {
          setShortcutsOpen(false);
          return;
        }
        if (machinesOpen) {
          setMachinesOpen(false);
          return;
        }
        if (selectedIds.length > 0) {
          setSelectedIds([]);
          (document.activeElement as HTMLElement | null)?.blur();
        }
        return;
      }
      if (e.key === "/" && !inEditable) {
        e.preventDefault();
        filterInputRef.current?.focus();
        filterInputRef.current?.select();
        return;
      }
      if (
        (e.key === "Delete" || e.key === "Backspace") &&
        !inEditable &&
        selectedIds.length > 0
      ) {
        e.preventDefault();
        deleteSelected();
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [
    selectedIds,
    deleteSelected,
    shortcutsOpen,
    machinesOpen,
    finderOpen,
    settingsOpen,
    importOpen,
    paletteOpen,
    undo,
    redo,
  ]);

  /// Clipboard-paste: auto-detect URL/email/IP/domain and drop the
  /// matching entity on the canvas. Multi-line pastes route through the
  /// ImportPanel so the user can pick the type and review before commit.
  /// Skipped when the paste lands in an editable element (text inputs,
  /// textareas) — those need normal browser paste behaviour.
  useEffect(() => {
    function onPaste(e: ClipboardEvent) {
      if (!currentCase) return;
      const target = e.target as HTMLElement | null;
      if (
        target &&
        (target.tagName === "INPUT" ||
          target.tagName === "TEXTAREA" ||
          target.isContentEditable)
      ) {
        return;
      }
      // Files on the clipboard → route through the existing upload path.
      // (User screenshot, drag from file manager via Cmd+C, etc.)
      const files = e.clipboardData?.files;
      if (files && files.length > 0) {
        e.preventDefault();
        uploadFiles(files);
        return;
      }
      const text = e.clipboardData?.getData("text") ?? "";
      const lines = text
        .split(/\r?\n/)
        .map((l) => l.trim())
        .filter((l) => l && !l.startsWith("#"));
      if (lines.length === 0) return;
      e.preventDefault();
      if (lines.length === 1) {
        const { type, label } = detectEntityType(lines[0]);
        addEntity(type, label);
        pushToast("info", `Pasted ${type}: ${label}`);
      } else {
        // Multi-line: don't auto-commit. Open the import modal so the
        // user can pick a type once and review before splatting N
        // entities. The user pastes again into the modal's textarea.
        setImportOpen(true);
        pushToast(
          "info",
          `${lines.length} lines — opening Import so you can pick a type`,
        );
      }
    }
    document.addEventListener("paste", onPaste);
    return () => document.removeEventListener("paste", onPaste);
  }, [currentCase, addEntity, uploadFiles, pushToast]);

  /// Assemble the full action catalogue the command palette searches over.
  /// We rebuild on dependency changes — actions are cheap (just closures
  /// over already-memoized state) so building per-render is fine.
  const paletteActions = useMemo<PaletteAction[]>(() => {
    const acts: PaletteAction[] = [
      {
        id: "case-new",
        label: "New case",
        group: "Case",
        keywords: "create investigation",
        onRun: createCase,
      },
      {
        id: "sample-graph",
        label: "Load sample graph",
        group: "Case",
        keywords: "onboarding demo example seed",
        disabled: !currentCase,
        onRun: loadSampleGraph,
      },
      {
        id: "add-note",
        label: "Add canvas note",
        group: "Graph",
        keywords: "annotation sticky comment label",
        disabled: !currentCase,
        onRun: () => addAnnotation(),
      },
      {
        id: "bookmarks",
        label: "Open bookmarks",
        group: "Panel",
        keywords: "starred favourite favorite list",
        disabled: !currentCase,
        onRun: () => setBookmarksOpen(true),
      },
      {
        id: "audit",
        label: "Open audit log",
        group: "Panel",
        keywords: "history evidence chain provenance trail",
        disabled: !currentCase,
        onRun: () => setAuditOpen(true),
      },
      {
        id: "snapshots",
        label: "Open snapshots",
        group: "Panel",
        keywords: "snapshot diff compare baseline before after restore point",
        disabled: !currentCase,
        onRun: () => setSnapshotsOpen(true),
      },
      {
        id: "snapshot-now",
        label: "Snapshot case now",
        group: "Case",
        keywords: "snapshot baseline capture freeze save point",
        disabled: !currentCase,
        onRun: async () => {
          if (!currentCase) return;
          const name = window.prompt(
            "Snapshot name",
            `Snapshot ${new Date().toLocaleString()}`,
          );
          if (!name?.trim()) return;
          try {
            await api.createCaseSnapshot(currentCase.id, name.trim());
            pushToast("success", "Snapshot captured");
          } catch (e) {
            pushToast("error", `Snapshot failed: ${e}`);
          }
        },
      },
      {
        id: "stealth",
        label: stealthOnly
          ? "Disable stealth mode (show every transform)"
          : "Enable stealth mode (passive transforms only)",
        group: "Edit",
        keywords: "opsec passive active surveillance red team",
        onRun: () => setStealthOnly((s) => !s),
      },
      {
        id: "merge-selected",
        label:
          selectedEntities.length === 2
            ? `Merge "${selectedEntities[1].label}" into "${selectedEntities[0].label}"`
            : "Merge entities (need exactly 2 selected)",
        group: "Edit",
        keywords: "combine dedupe deduplicate",
        disabled: selectedEntities.length !== 2,
        onRun: mergeSelectedEntities,
      },
      {
        id: "import",
        label: "Import entities (bulk / CSV)",
        group: "Case",
        keywords: "paste add",
        disabled: !currentCase,
        onRun: () => setImportOpen(true),
      },
      {
        id: "upload",
        label: "Upload file or image",
        group: "Case",
        keywords: "drop attach",
        disabled: !currentCase,
        onRun: pickAndUpload,
      },
      {
        id: "find",
        label: "Find on graph",
        group: "Graph",
        keywords: "search cmd+f ctrl+f",
        onRun: () => setFinderOpen(true),
      },
      {
        id: "undo",
        label:
          undoStack.length === 0
            ? "Undo (nothing to undo)"
            : `Undo: ${undoStack[undoStack.length - 1].label}`,
        group: "Edit",
        keywords: "back revert cmd+z ctrl+z",
        disabled: undoStack.length === 0,
        onRun: undo,
      },
      {
        id: "redo",
        label:
          redoStack.length === 0
            ? "Redo (nothing to redo)"
            : `Redo: ${redoStack[redoStack.length - 1].label}`,
        group: "Edit",
        keywords: "forward replay cmd+shift+z ctrl+y",
        disabled: redoStack.length === 0,
        onRun: redo,
      },
      {
        id: "select-neighbors",
        label: "Select: add neighbors",
        group: "Select",
        keywords: "expand adjacent connected",
        disabled: selectedIds.length === 0,
        onRun: selectNeighbors,
      },
      {
        id: "select-component",
        label: "Select: connected component",
        group: "Select",
        keywords: "subgraph reachable",
        disabled: selectedIds.length === 0,
        onRun: selectConnectedComponent,
      },
      {
        id: "select-invert",
        label: "Select: invert",
        group: "Select",
        keywords: "everything else",
        disabled: entities.length === 0,
        onRun: invertSelection,
      },
      {
        id: "layout-fcose",
        label: "Layout: Organic",
        group: "Graph",
        keywords: "fcose force directed",
        onRun: () => runLayout("fcose"),
      },
      {
        id: "layout-dagre",
        label: "Layout: Hierarchical",
        group: "Graph",
        keywords: "dagre tree top down",
        onRun: () => runLayout("dagre"),
      },
      {
        id: "layout-concentric",
        label: "Layout: Concentric",
        group: "Graph",
        onRun: () => runLayout("concentric"),
      },
      {
        id: "layout-circle",
        label: "Layout: Circle",
        group: "Graph",
        onRun: () => runLayout("circle"),
      },
      {
        id: "layout-grid",
        label: "Layout: Grid",
        group: "Graph",
        onRun: () => runLayout("grid"),
      },
      {
        id: "fit",
        label: "Fit to view",
        group: "Graph",
        onRun: () => graphRef.current?.fit(),
      },
      {
        id: "panel-machines",
        label: "Open Machines",
        group: "Panel",
        disabled: machines.length === 0,
        onRun: () => setMachinesOpen(true),
      },
      {
        id: "panel-runlog",
        label: "Open Run log",
        group: "Panel",
        onRun: () => setRunLogOpen(true),
      },
      {
        id: "panel-settings",
        label: "Open Settings (API keys)",
        group: "Panel",
        onRun: () => setSettingsOpen(true),
      },
      {
        id: "panel-plugins",
        label: "Open Plugins",
        group: "Panel",
        onRun: () => setPluginsOpen(true),
      },
      {
        id: "panel-ai",
        label: "Toggle AI Analyst",
        group: "Panel",
        onRun: () => setAiOpen((o) => !o),
      },
      {
        id: "shortcuts",
        label: "Keyboard shortcuts",
        group: "Help",
        keywords: "help ?",
        onRun: () => setShortcutsOpen(true),
      },
      {
        id: "perf-mode-toggle",
        label:
          perfModeOverride === true
            ? "Disable performance mode (use auto / quality)"
            : perfModeOverride === false
              ? "Re-enable performance mode (currently forced off)"
              : "Force performance mode on (faster pan/zoom for big graphs)",
        group: "Graph",
        keywords: "perf performance fps fast big graph virtualize",
        onRun: () =>
          setPerfModeOverride((cur) =>
            cur === true ? null : cur === false ? true : true,
          ),
      },
      {
        id: "webgl-toggle",
        label: webglEnabled
          ? "Graph renderer: back to Canvas2D (restores labels)"
          : "Graph renderer: WebGL (experimental — for huge graphs, hides labels)",
        group: "Graph",
        keywords: "webgl gpu renderer canvas2d performance smooth fps gl",
        onRun: () => setWebglEnabled((v) => !v),
      },
      {
        id: "welcome-tour",
        label: "Show welcome tour",
        group: "Help",
        keywords: "onboarding intro getting started first run",
        onRun: () => setWelcomeOpen(true),
      },
    ];
    // Per-case "Switch to …" entries so jumping cases doesn't require
    // tab-clicking through a long strip.
    for (const c of cases) {
      if (c.id === currentCase?.id) continue;
      acts.push({
        id: `case-switch-${c.id}`,
        label: `Switch to: ${c.name}`,
        group: "Case",
        onRun: () => switchCase(c.id),
      });
    }
    // "Select all of type X" — one entry per type that's actually on
    // the graph. Lets the user grab e.g. every Email in a few keystrokes.
    {
      const counts = new Map<string, number>();
      for (const e of entities) counts.set(e.entity_type, (counts.get(e.entity_type) ?? 0) + 1);
      const ordered = [...counts.entries()].sort((a, b) => b[1] - a[1]);
      for (const [typeId, n] of ordered) {
        acts.push({
          id: `select-type-${typeId}`,
          label: `Select all ${typeId} (${n})`,
          group: "Select",
          onRun: () => selectByType(typeId),
        });
      }
    }
    // Per-machine "Run …" entries. Only enabled when the selected entity
    // matches the machine's input_type — otherwise show as disabled with
    // an explanatory label.
    for (const m of machines) {
      const ok =
        !!primaryEntity && primaryEntity.entity_type === m.input_type;
      acts.push({
        id: `machine-${m.id}`,
        label: `Run machine: ${m.name}`,
        group: "Machine",
        keywords: `${m.id} ${m.input_type}`,
        disabled: !ok,
        onRun: () => {
          if (ok) runMachine(m.id);
        },
      });
    }
    // Export entries (current case).
    if (currentCase) {
      acts.push(
        {
          id: "export-graphml",
          label: "Export current case as GraphML",
          group: "Export",
          onRun: () => exportCaseGraphml(currentCase.id),
        },
        {
          id: "export-csv",
          label: "Export current case as CSV",
          group: "Export",
          onRun: () => exportCaseCsv(currentCase.id),
        },
        {
          id: "export-png",
          label: "Export graph as PNG",
          group: "Export",
          onRun: () => exportCasePng(currentCase.id),
        },
        {
          id: "export-pdf",
          label: "Export graph as PDF",
          group: "Export",
          onRun: () => exportCasePdf(currentCase.id),
        },
        {
          id: "export-stix",
          label: "Export STIX 2.1 bundle (.json)",
          group: "Export",
          keywords: "tip threat intel handoff",
          onRun: () => exportCaseStix(currentCase.id),
        },
        {
          id: "export-report",
          label: "Export full HTML report",
          group: "Export",
          keywords: "summary findings provenance",
          onRun: () => exportCaseReport(currentCase.id),
        },
      );
    }
    return acts;
  }, [
    cases,
    currentCase,
    machines,
    primaryEntity,
    createCase,
    loadSampleGraph,
    addAnnotation,
    mergeSelectedEntities,
    stealthOnly,
    selectedEntities,
    switchCase,
    runMachine,
    runLayout,
    pickAndUpload,
    exportCaseGraphml,
    exportCaseCsv,
    exportCasePng,
    exportCasePdf,
    exportCaseStix,
    exportCaseReport,
    perfModeOverride,
    webglEnabled,
    undoStack,
    undo,
    redoStack,
    redo,
    selectedIds,
    selectNeighbors,
    selectConnectedComponent,
    invertSelection,
    selectByType,
    entities,
  ]);


  return (
    <div className="app">
      <header className="topbar">
        <div className="brand">
          <BrandMark />
          <h1>maltrace</h1>
          <span className="brand-tag">OSINT</span>
          {stealthOnly && (
            <span
              className="brand-mode stealth-badge"
              title="Stealth mode on — only passive transforms are listed. Cmd+K → 'stealth' to toggle."
            >
              STEALTH
            </span>
          )}
          <span className="brand-mode">
            {runtime.isTauri ? "Desktop" : "Web"}
          </span>
        </div>
        <div className="divider-v" />
        <CaseTabs
          cases={cases}
          currentCaseId={currentCase?.id ?? null}
          busy={busy}
          onSelect={switchCase}
          onCreate={createCase}
          onRename={renameCase}
          onDelete={deleteCase}
          onExport={exportCaseGraphml}
          onExportCsv={exportCaseCsv}
          onExportPng={exportCasePng}
          onExportPdf={exportCasePdf}
          onExportStix={exportCaseStix}
          onExportReport={exportCaseReport}
          onExportGeoJson={exportCaseGeoJson}
          onExportKml={exportCaseKml}
          onExportBackup={exportCaseBackup}
          onRestoreBackup={restoreCaseBackup}
          onToggleLock={toggleCaseLock}
        />
        <div className="divider-v" />
        <AddEntityForm
          canonicalTypes={canonicalTypes}
          onAdd={addEntity}
          disabled={!currentCase || caseLocked}
        />
        <div className="status">
          <NotificationsBell
            caseId={currentCase?.id ?? null}
            refreshKey={runLogRefreshKey}
          />
          <button
            className={`topbar-action${aiOpen ? " is-active" : ""}`}
            onClick={() => setAiOpen((o) => !o)}
            title="Toggle AI analyst panel"
          >
            <AiIcon /> Analyst
          </button>
          <button
            className="topbar-action"
            disabled={machines.length === 0}
            onClick={() => setMachinesOpen(true)}
            title={
              machines.length === 0
                ? "No machines available"
                : "Open machines panel"
            }
          >
            <MachineIcon /> Machines
            {machines.length > 0 && (
              <span className="small-tag" style={{ marginLeft: 6 }}>
                {machines.length}
              </span>
            )}
          </button>
          <button
            className={`topbar-action${pluginsOpen ? " is-active" : ""}`}
            onClick={() => setPluginsOpen((o) => !o)}
            title="Loaded plugins"
          >
            <PluginIcon /> Plugins
          </button>
          <button
            className={`topbar-action${savedSearchesOpen ? " is-active" : ""}`}
            onClick={() => setSavedSearchesOpen((o) => !o)}
            disabled={!currentCase}
            title="Saved searches — recall a filter / selection from one click"
          >
            🔎 Searches
          </button>
          <button
            className="topbar-action"
            disabled={!currentCase}
            onClick={() => setImportOpen(true)}
            title="Import entities (bulk paste or CSV)"
          >
            <ImportIcon /> Import
          </button>
          <button
            className="topbar-action"
            disabled={!currentCase}
            onClick={pickAndUpload}
            title="Upload an image or file — or drop one onto the canvas"
          >
            <UploadIcon /> Upload
          </button>
          <button
            className="topbar-action"
            disabled={!currentCase}
            onClick={() => addAnnotation()}
            title="Drop a sticky-note annotation on the canvas"
          >
            <StickyIcon /> Note
          </button>
          <button
            className={`topbar-action${bookmarksOpen ? " is-active" : ""}`}
            disabled={!currentCase}
            onClick={() => setBookmarksOpen((b) => !b)}
            title="List of bookmarked entities"
          >
            <StarIcon /> Bookmarks
          </button>
          <button
            className={`topbar-action${runLogOpen ? " is-active" : ""}`}
            onClick={() => setRunLogOpen((o) => !o)}
            title="Transform run log"
          >
            <LogIcon /> Log
          </button>
          <button
            className={`topbar-action${settingsOpen ? " is-active" : ""}`}
            onClick={() => setSettingsOpen((o) => !o)}
            title="Settings (API keys)"
          >
            <SettingsIcon />
          </button>
          {busy && <span className="spinner" aria-label="working" />}
          {transformsLoading && (
            <span className="muted small-tag">Loading transforms…</span>
          )}
          {!transformsLoading && transforms.length > 0 && (
            <button
              className="status-btn"
              onClick={() => setPluginsOpen(true)}
              title="View loaded plugins"
            >
              {transforms.length} transforms
            </button>
          )}
          {entities.length > 0 && (
            <span className="muted">
              {entities.length} {entities.length === 1 ? "entity" : "entities"}
              {selectedIds.length > 0 && (
                <>
                  {" · "}
                  <span className="select-count">
                    {selectedIds.length} selected
                  </span>
                </>
              )}
            </span>
          )}
          <button
            className="icon-btn"
            title="Keyboard shortcuts (?)"
            aria-label="Keyboard shortcuts"
            onClick={() => setShortcutsOpen((o) => !o)}
          >
            ?
          </button>
        </div>
      </header>

      <div className={`main${inspectorCollapsed ? " inspector-collapsed" : ""}`}>
        <div
          className={`canvas${dragHover ? " drag-hover" : ""}`}
          onDragEnter={(e) => {
            if (e.dataTransfer.types.includes("Files")) {
              e.preventDefault();
              setDragHover(true);
            }
          }}
          onDragOver={(e) => {
            if (e.dataTransfer.types.includes("Files")) {
              e.preventDefault();
              e.dataTransfer.dropEffect = "copy";
            }
          }}
          onDragLeave={(e) => {
            // Only clear when we've actually left the canvas, not on child
            // transitions (which fire dragleave even though the cursor is
            // still over the canvas).
            if (e.currentTarget === e.target) setDragHover(false);
          }}
          onDrop={(e) => {
            const files = e.dataTransfer.files;
            if (files && files.length > 0) {
              e.preventDefault();
              setDragHover(false);
              uploadFiles(files);
            }
          }}
        >
          <ViewSwitcher current={view} onChange={setView} />
          {view === "graph" && (
            <GraphCanvas
              ref={graphRef}
              entities={entities}
              edges={edges}
              annotations={annotations}
              selectedIds={selectedIds}
              canonicalTypes={canonicalTypes}
              transformSources={transformSources}
              transformColors={transformColors}
              hiddenTypes={hiddenTypes}
              bookmarkedOnly={bookmarkedOnly}
              performanceMode={perfModeOverride ?? undefined}
              webgl={webglEnabled}
              onSelectionChange={(ids) => {
                setSelectedIds(ids);
                if (ids.length > 0) setSelectedEdgeId(null);
              }}
              onAnnotationSelectionChange={setSelectedAnnotationIds}
              onEdgeSelectionChange={(eid) => {
                setSelectedEdgeId(eid);
                if (eid) {
                  setSelectedIds([]);
                  setSelectedAnnotationIds([]);
                }
              }}
              onPositionChange={onPositionChange}
              onAnnotationMoved={onAnnotationMoved}
              onContextMenu={openContextMenu}
              onContextMenuBackground={closeContextMenu}
            />
          )}
          {view === "map" && (
            <GeoView
              entities={entities}
              canonicalTypes={canonicalTypes}
              selectedIds={selectedIds}
              onSelect={(id) => {
                setSelectedIds([id]);
                setSelectedEdgeId(null);
              }}
            />
          )}
          {view === "timeline" && (
            <TimelineView
              caseId={currentCase?.id ?? null}
              entities={entities}
              canonicalTypes={canonicalTypes}
              selectedIds={selectedIds}
              onSelectEntity={(id) => {
                setSelectedIds([id]);
                setSelectedEdgeId(null);
                // Re-focus on the graph tab so the user sees what they
                // picked. Keeping the timeline open would hide it.
                setView("graph");
                window.requestAnimationFrame(() => {
                  graphRef.current?.focusEntity(id);
                });
              }}
              refreshKey={runLogRefreshKey}
            />
          )}
          <FilterStrip
            entities={entities}
            canonicalTypes={canonicalTypes}
            hiddenTypes={hiddenTypes}
            onToggleType={toggleHiddenType}
            onShowAll={() => {
              showAllTypes();
              setBookmarkedOnly(false);
            }}
            bookmarkedOnly={bookmarkedOnly}
            onToggleBookmarkedOnly={() => setBookmarkedOnly((b) => !b)}
          />
          {entities.length === 0 && !transformsLoading && (
            <div className="canvas-empty">
              <div className="canvas-empty-title">
                Drop your first entity to start an investigation
              </div>
              <div className="canvas-empty-sub">
                Type a value in the topbar (the type is auto-detected), drop
                an image here, or jump in with one of these:
              </div>
              <div
                className="canvas-empty-examples"
                role="group"
                aria-label="Example starting points"
              >
                {EXAMPLE_TARGETS.map((ex) => (
                  <button
                    key={ex.entityType + ex.label}
                    className="canvas-empty-example"
                    disabled={!currentCase || busy || caseLocked}
                    onClick={() => addEntity(ex.entityType, ex.label)}
                    title={ex.hint}
                  >
                    <span className="canvas-empty-example-type">
                      {ex.entityType.replace(/([a-z])([A-Z])/g, "$1 $2")}
                    </span>
                    <span className="canvas-empty-example-label">
                      {ex.label}
                    </span>
                  </button>
                ))}
              </div>
              <div className="canvas-empty-actions">
                <button
                  className="canvas-empty-sample"
                  disabled={!currentCase || busy}
                  onClick={loadSampleGraph}
                >
                  Load sample graph
                </button>
                <button
                  className="canvas-empty-link"
                  onClick={() => setPaletteOpen(true)}
                >
                  Browse all actions →
                </button>
              </div>
              <div className="canvas-empty-hints">
                <span className="kbd">Ctrl/Cmd</span>
                <span className="kbd">K</span> for the command palette
                <span className="canvas-empty-hint-sep">·</span>
                <span className="kbd">?</span> for shortcuts
              </div>
            </div>
          )}
          {dragHover && (
            <div className="canvas-drop-overlay" aria-hidden="true">
              <div className="canvas-drop-card">
                <UploadIcon />
                <span>Drop to upload</span>
              </div>
            </div>
          )}
          {view === "graph" && (
            <CanvasControls
              onFit={() => graphRef.current?.fit()}
              onCenter={() => graphRef.current?.center()}
              onCenterAverage={() => graphRef.current?.centerAverage()}
              onZoomIn={() => graphRef.current?.zoomBy(1.2)}
              onZoomOut={() => graphRef.current?.zoomBy(1 / 1.2)}
              onResetZoom={() => graphRef.current?.resetZoom()}
              onLayout={runLayout}
            />
          )}
        </div>
        <button
          type="button"
          className={`inspector-toggle${inspectorCollapsed ? " is-collapsed" : ""}`}
          onClick={() => setInspectorCollapsed((c) => !c)}
          aria-label={inspectorCollapsed ? "Show inspector" : "Hide inspector"}
          aria-expanded={!inspectorCollapsed}
          title={inspectorCollapsed ? "Show inspector" : "Hide inspector"}
        >
          <svg viewBox="0 0 16 16" width="12" height="12" aria-hidden="true">
            <path
              d={inspectorCollapsed ? "M10 3L5 8l5 5" : "M6 3l5 5-5 5"}
              fill="none"
              stroke="currentColor"
              strokeWidth="1.6"
              strokeLinecap="round"
              strokeLinejoin="round"
            />
          </svg>
        </button>
        {selectedEdgeId &&
        selectedEntities.length === 0 &&
        selectedAnnotationIds.length === 0 ? (
          <EdgeInspector
            edgeId={selectedEdgeId}
            onFocusEntity={(id) => {
              setSelectedIds([id]);
              setSelectedEdgeId(null);
              setView("graph");
              // The GraphCanvas may not be mounted (or Cytoscape may
              // not have done its first render) at the next frame —
              // retry a few times before giving up so jumping from the
              // timeline / inspector lands on the right node.
              let attempts = 0;
              const focus = () => {
                if (graphRef.current) {
                  graphRef.current.focusEntity(id);
                  return;
                }
                if (attempts++ < 20) window.requestAnimationFrame(focus);
              };
              window.requestAnimationFrame(focus);
            }}
            onClose={() => setSelectedEdgeId(null)}
          />
        ) : selectedAnnotationIds.length === 1 &&
          selectedEntities.length === 0 ? (
          <AnnotationEditor
            annotations={annotations}
            selectedId={selectedAnnotationIds[0]}
            onTextChange={updateAnnotationText}
            onColorChange={updateAnnotationColor}
            onDelete={deleteAnnotation}
          />
        ) : (
          <EntityInspector
            entities={selectedEntities}
            canonicalTypes={canonicalTypes}
            transforms={transforms}
            transformsLoading={transformsLoading}
            machines={machines}
            onRunMachine={runMachine}
            busy={busy}
            readOnly={caseLocked}
            onRunTransform={(tid) => runTransform(tid)}
            onRunAll={runAllTransforms}
            onDelete={deleteSelected}
            onExpandCollection={expandCollection}
            onFocusEntity={(id) => {
              setSelectedIds([id]);
              setSelectedEdgeId(null);
              setView("graph");
              // The GraphCanvas may not be mounted (or Cytoscape may
              // not have done its first render) at the next frame —
              // retry a few times before giving up so jumping from the
              // timeline / inspector lands on the right node.
              let attempts = 0;
              const focus = () => {
                if (graphRef.current) {
                  graphRef.current.focusEntity(id);
                  return;
                }
                if (attempts++ < 20) window.requestAnimationFrame(focus);
              };
              window.requestAnimationFrame(focus);
            }}
            onPatchEntity={(id, patch) => {
              setEntities((prev) =>
                prev.map((e) => (e.id === id ? { ...e, ...patch } : e)),
              );
            }}
            filterInputRef={filterInputRef}
          />
        )}
      </div>

      <div className="toasts" role="status" aria-live="polite">
        {toasts.map((t) => (
          <div key={t.id} className={`toast toast-${t.kind}`}>
            <span className="toast-dot" />
            <span className="toast-text">{t.text}</span>
            <button
              className="toast-close"
              onClick={() => dismissToast(t.id)}
              aria-label="Dismiss"
            >
              ×
            </button>
          </div>
        ))}
      </div>

      {shortcutsOpen && (
        <ShortcutsHint onClose={() => setShortcutsOpen(false)} />
      )}

      {welcomeOpen && (
        <WelcomeModal
          onClose={closeWelcome}
          onLoadSample={loadSampleGraph}
          sampleDisabled={!currentCase || busy}
        />
      )}

      {machinesOpen && (
        <MachinesPanel
          machines={machines}
          selectedEntity={primaryEntity}
          busy={busy}
          onRun={runMachine}
          onClose={() => setMachinesOpen(false)}
          lastRun={lastMachineRun}
          caseId={currentCase?.id ?? null}
          readOnly={caseLocked}
        />
      )}

      {paramsDialog && (
        <TransformParamsDialog
          transform={paramsDialog.transform}
          entityLabel={
            paramsDialog.targetIds.length === 1
              ? entityById.get(paramsDialog.targetIds[0])?.label ?? "entity"
              : `${paramsDialog.targetIds.length} entities`
          }
          onCancel={() => setParamsDialog(null)}
          onRun={(params) => {
            const dlg = paramsDialog;
            setParamsDialog(null);
            void fireTransform(dlg.transform.id, dlg.targetIds, params);
          }}
        />
      )}

      {aiOpen && (
        <AiPanel
          caseId={currentCase?.id ?? null}
          selectedEntity={primaryEntity}
          onClose={() => setAiOpen(false)}
        />
      )}

      {pluginsOpen && <PluginsPopover onClose={() => setPluginsOpen(false)} />}

      {settingsOpen && (
        <SettingsPanel
          onClose={() => setSettingsOpen(false)}
          canonicalTypes={canonicalTypes}
          onCanonicalTypesChanged={() => {
            api.listCanonicalTypes().then(setCanonicalTypes).catch(() => {});
          }}
        />
      )}

      {importOpen && currentCase && (
        <ImportPanel
          caseId={currentCase.id}
          canonicalTypes={canonicalTypes}
          onClose={() => setImportOpen(false)}
          onImported={onImported}
        />
      )}

      {runLogOpen && (
        <RunLogPanel
          caseId={currentCase?.id ?? null}
          refreshKey={runLogRefreshKey}
          onClose={() => setRunLogOpen(false)}
        />
      )}

      {pluginsOpen && (
        <PluginsPopover onClose={() => setPluginsOpen(false)} />
      )}

      {contextMenu && contextMenuEntities.length > 0 && (
        <TransformContextMenu
          entities={contextMenuEntities}
          canonicalTypes={canonicalTypes}
          transforms={transforms}
          busy={busy}
          x={contextMenu.x}
          y={contextMenu.y}
          onRunTransform={(tid) =>
            runTransform(
              tid,
              contextMenuEntities.map((e) => e.id),
            )
          }
          onRunAll={runAllTransforms}
          onMerge={
            contextMenuEntities.length === 2 ? mergeSelectedEntities : undefined
          }
          stealthOnly={stealthOnly}
          onClose={closeContextMenu}
        />
      )}

      {finderOpen && (
        <GraphFinder
          entities={entities}
          canonicalTypes={canonicalTypes}
          onPick={(id) => {
            setSelectedIds([id]);
            graphRef.current?.focusEntity(id);
          }}
          onJumpToCase={async (caseId, entityId) => {
            // Cross-case jump: switch the active case, then select +
            // centre on the target once its case has loaded. The
            // setTimeout lets React apply the case-switch before we ask
            // Cytoscape to focus the newly-loaded node.
            await switchCase(caseId);
            setTimeout(() => {
              setSelectedIds([entityId]);
              graphRef.current?.focusEntity(entityId);
            }, 80);
          }}
          onClose={() => setFinderOpen(false)}
        />
      )}

      {bookmarksOpen && (
        <BookmarksPanel
          entities={entities}
          canonicalTypes={canonicalTypes}
          onPick={(id) => {
            setSelectedIds([id]);
            graphRef.current?.focusEntity(id);
          }}
          onClose={() => setBookmarksOpen(false)}
        />
      )}

      {savedSearchesOpen && currentCase && (
        <SavedSearchesPopover
          caseId={currentCase.id}
          entities={entities}
          canonicalTypes={canonicalTypes}
          readOnly={caseLocked}
          onApply={applySavedSearch}
          onClose={() => setSavedSearchesOpen(false)}
        />
      )}

      {auditOpen && (
        <AuditPanel
          caseId={currentCase?.id ?? null}
          onClose={() => setAuditOpen(false)}
        />
      )}

      {snapshotsOpen && currentCase && (
        <SnapshotsPanel
          caseId={currentCase.id}
          readOnly={currentCase.locked}
          onClose={() => setSnapshotsOpen(false)}
        />
      )}

      {paletteOpen && (
        <CommandPalette
          actions={paletteActions}
          onClose={() => setPaletteOpen(false)}
        />
      )}
    </div>
  );
}

function BrandMark() {
  return (
    <span className="brand-mark" aria-hidden="true">
      <svg viewBox="0 0 24 24" width="22" height="22">
        <defs>
          <linearGradient id="bm-g" x1="0" y1="0" x2="1" y2="1">
            <stop offset="0%" stopColor="#5eead4" />
            <stop offset="100%" stopColor="#3b82f6" />
          </linearGradient>
        </defs>
        <g
          stroke="url(#bm-g)"
          strokeWidth="1.4"
          fill="none"
          strokeLinecap="round"
        >
          <line x1="6" y1="7" x2="12" y2="12" />
          <line x1="18" y1="7" x2="12" y2="12" />
          <line x1="12" y1="12" x2="12" y2="18" />
        </g>
        <g fill="url(#bm-g)">
          <circle cx="6" cy="7" r="1.9" />
          <circle cx="18" cy="7" r="1.9" />
          <circle cx="12" cy="18" r="1.9" />
          <circle cx="12" cy="12" r="2.4" />
        </g>
      </svg>
    </span>
  );
}

function AiIcon() {
  return (
    <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true">
      <path
        d="M8 2l1.5 3.5L13 7l-3.5 1.5L8 12l-1.5-3.5L3 7l3.5-1.5L8 2z"
        fill="currentColor"
        opacity="0.85"
      />
    </svg>
  );
}

function MachineIcon() {
  return (
    <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true">
      <path
        d="M3 4h10M3 8h10M3 12h6"
        stroke="currentColor"
        strokeWidth="1.5"
        strokeLinecap="round"
        fill="none"
      />
      <circle cx="13" cy="12" r="1.5" fill="currentColor" />
    </svg>
  );
}

function PluginIcon() {
  return (
    <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true">
      <path
        d="M5 2v3M11 2v3M3 5h10v4a4 4 0 01-4 4H7a4 4 0 01-4-4V5z"
        stroke="currentColor"
        strokeWidth="1.4"
        strokeLinejoin="round"
        fill="none"
      />
      <path d="M8 13v2" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" />
    </svg>
  );
}

function ImportIcon() {
  return (
    <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true">
      <path
        d="M8 2v8m0 0l-3-3m3 3l3-3M3 12v1a1 1 0 001 1h8a1 1 0 001-1v-1"
        stroke="currentColor"
        strokeWidth="1.4"
        strokeLinecap="round"
        strokeLinejoin="round"
        fill="none"
      />
    </svg>
  );
}

function UploadIcon() {
  return (
    <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true">
      <path
        d="M8 14V6m0 0L5 9m3-3l3 3M4 4h8"
        stroke="currentColor"
        strokeWidth="1.4"
        strokeLinecap="round"
        strokeLinejoin="round"
        fill="none"
      />
    </svg>
  );
}

function StickyIcon() {
  return (
    <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true">
      <path
        d="M3 3h7l3 3v7H3z M10 3v3h3"
        stroke="currentColor"
        strokeWidth="1.3"
        strokeLinejoin="round"
        fill="none"
      />
    </svg>
  );
}

function StarIcon() {
  return (
    <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true">
      <path
        d="M8 1.5l1.95 3.95 4.35.63-3.15 3.07.74 4.32L8 11.47l-3.89 2 .74-4.32L1.7 6.08l4.35-.63L8 1.5z"
        fill="currentColor"
        opacity="0.85"
      />
    </svg>
  );
}

function LogIcon() {
  return (
    <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true">
      <path
        d="M3 3h10M3 8h10M3 13h6"
        stroke="currentColor"
        strokeWidth="1.5"
        strokeLinecap="round"
        fill="none"
      />
    </svg>
  );
}

function SettingsIcon() {
  return (
    <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true">
      <circle cx="8" cy="8" r="2.4" fill="none" stroke="currentColor" strokeWidth="1.3" />
      <path
        d="M8 1.6v1.8M8 12.6v1.8M14.4 8h-1.8M3.4 8H1.6M12.5 3.5l-1.3 1.3M4.8 11.2l-1.3 1.3M12.5 12.5l-1.3-1.3M4.8 4.8L3.5 3.5"
        stroke="currentColor"
        strokeWidth="1.3"
        strokeLinecap="round"
      />
    </svg>
  );
}

// colorForTransform moved to ./lib/detectEntityType.ts

// detectEntityType + colorForTransform moved to ./lib/detectEntityType.ts
// so they can be unit-tested without booting the React tree.
