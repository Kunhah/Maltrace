import { useCallback, useEffect, useMemo, useState, type RefObject } from "react";
import type {
  CanonicalType,
  CollectionPayload,
  Entity,
  EntityProvenance,
  MachineMeta,
  TransformMeta,
  TransformRun,
} from "../types";
import {
  useCompatibleTransforms,
  useCompatibleTransformsForMany,
} from "../hooks/useCompatibleTransforms";
import { api, isSafeUrl } from "../api";
import { recommendedTransformIds } from "../recommendations";
import { TagsRow } from "./TagsRow";

interface Props {
  /// Current selection. Empty = nothing selected; 1 = full inspector; N>1 =
  /// batch view.
  entities: Entity[];
  canonicalTypes: CanonicalType[];
  transforms: TransformMeta[];
  transformsLoading: boolean;
  /// Available machines. Used to surface a one-click CTA at the top of
  /// the inspector when a machine's input type matches the selected
  /// entity — "next move on a fresh Domain" should be obvious.
  machines?: MachineMeta[];
  onRunMachine?: (machineId: string) => void;
  busy: boolean;
  onRunTransform: (transformId: string) => void;
  /// Run every compatible transform for the (single) selected entity.
  /// Called with the list of transform ids the inspector currently shows
  /// so the backend doesn't have to recompute compatibility.
  onRunAll?: (entityId: string, transformIds: string[]) => void;
  onDelete: () => void;
  onExpandCollection?: (collectionId: string) => void;
  /// Optimistically update the entity in the parent's state map. Used so
  /// note edits / bookmark toggles / renames re-render instantly without
  /// a full reload. Implementation: same `{id, ...patch}` merge App.tsx
  /// already uses on transform results.
  onPatchEntity?: (id: string, patch: Partial<Entity>) => void;
  filterInputRef: RefObject<HTMLInputElement | null>;
  /// Jump the canvas / selection to a different entity id. Used by the
  /// provenance section's edge list — clicking the other endpoint
  /// selects + focuses it on the graph.
  onFocusEntity?: (entityId: string) => void;
  /// Case is locked / read-only — disable every mutation affordance
  /// (delete, rename, notes, bookmark, run, expand). Reads still work.
  readOnly?: boolean;
}

const SOURCE_LABELS: Record<string, string> = {
  spiderfoot: "SpiderFoot",
  "face-search": "Face search",
  brazuca: "Brazuca",
  "": "Other",
};

export function EntityInspector(props: Props) {
  if (props.entities.length === 0) {
    return (
      <aside className="inspector">
        <div className="inspector-empty">
          <span className="inspector-empty-mark" aria-hidden="true" />
          <div>Select a node to inspect it</div>
          <div className="muted">
            Run transforms to pivot from any selected entity.{" "}
            <span className="kbd">Ctrl+Click</span> or drag to select more
            than one.
          </div>
        </div>
      </aside>
    );
  }
  if (props.entities.length > 1) {
    return <MultiSelectInspector {...props} />;
  }
  return <SingleInspector {...props} />;
}

// ─── Single-entity inspector ────────────────────────────────────────────

function SingleInspector({
  entities,
  canonicalTypes,
  transforms,
  transformsLoading,
  machines,
  onRunMachine,
  busy,
  onRunTransform,
  onRunAll,
  onDelete,
  onExpandCollection,
  onPatchEntity,
  filterInputRef,
  onFocusEntity,
  readOnly,
}: Props) {
  const entity = entities[0];
  const [filter, setFilter] = useState("");

  const { compatible: compatibleTransforms, canonical } =
    useCompatibleTransforms(entity, canonicalTypes, transforms, filter);

  /// Curated "best first move" transforms for this entity's type,
  /// intersected with what's actually loaded. Drives the "Recommended"
  /// group at the top of the transform list.
  const recommended = useMemo(() => {
    const ids = recommendedTransformIds(entity.entity_type);
    if (ids.length === 0) return [] as TransformMeta[];
    const byId = new Map(compatibleTransforms.map((t) => [t.id, t]));
    const out: TransformMeta[] = [];
    for (const id of ids) {
      const t = byId.get(id);
      if (t) out.push(t);
    }
    return out;
  }, [entity.entity_type, compatibleTransforms]);
  const recommendedIds = useMemo(
    () => new Set(recommended.map((t) => t.id)),
    [recommended],
  );

  /// The first machine whose input type matches this entity. Shown as a
  /// big one-click CTA so a brand-new user knows the "right" next move
  /// after dropping a Domain / IP / Email on the canvas.
  const matchingMachine = useMemo(() => {
    if (!machines || !onRunMachine) return null;
    return machines.find((m) => m.input_type === entity.entity_type) ?? null;
  }, [machines, onRunMachine, entity.entity_type]);

  const grouped = useMemo(() => {
    const out = new Map<string, TransformMeta[]>();
    for (const t of compatibleTransforms) {
      // Recommended transforms have their own dedicated section above —
      // don't duplicate them in the source-grouped list.
      if (recommendedIds.has(t.id)) continue;
      const key = t.source ?? "";
      if (!out.has(key)) out.set(key, []);
      out.get(key)!.push(t);
    }
    const order = (k: string) =>
      k === "spiderfoot" ? 0 : k === "face-search" ? 1 : 2;
    return [...out.entries()].sort(([a], [b]) => {
      const oa = order(a);
      const ob = order(b);
      return oa === ob ? a.localeCompare(b) : oa - ob;
    });
  }, [compatibleTransforms, recommendedIds]);

  const fullData = useMemo(() => {
    if (!entity.data) return null;
    try {
      return JSON.parse(entity.data) as {
        full_data?: string;
        source_event_type?: string;
        confidence?: number;
        extra?: Record<string, unknown> | null;
      };
    } catch {
      return null;
    }
  }, [entity]);

  const collectionPayload = useMemo<CollectionPayload | null>(() => {
    if (entity.entity_type !== "Collection" || !entity.data) return null;
    try {
      return JSON.parse(entity.data) as CollectionPayload;
    } catch {
      return null;
    }
  }, [entity]);

  const openableUrl = useMemo<string | null>(() => {
    const extra = fullData?.extra;
    if (extra && typeof extra === "object" && "url" in extra) {
      const u = (extra as { url?: unknown }).url;
      if (typeof u === "string" && isSafeUrl(u)) return u;
    }
    if (entity.entity_type === "Url" && isSafeUrl(entity.label)) {
      return entity.label;
    }
    return null;
  }, [entity, fullData]);

  const onOpen = useCallback(() => {
    if (!openableUrl) return;
    api.openUrl(openableUrl).catch((err) => {
      // eslint-disable-next-line no-console
      console.error("openUrl failed:", err);
    });
  }, [openableUrl]);

  const badgeStyle = canonical
    ? { color: canonical.color, borderColor: canonical.color }
    : undefined;

  const isCollection = entity.entity_type === "Collection";
  const isImage = entity.entity_type === "Image";

  // ─── Rename ────────────────────────────────────────────────────
  const [renaming, setRenaming] = useState(false);
  const [renameDraft, setRenameDraft] = useState("");
  const [renameError, setRenameError] = useState<string | null>(null);
  // Drop rename state when the selected entity changes — otherwise the
  // draft from the previous entity leaks into the next one's edit box.
  useEffect(() => {
    setRenaming(false);
    setRenameDraft("");
    setRenameError(null);
  }, [entity.id]);
  const commitRename = useCallback(async () => {
    const next = renameDraft.trim();
    if (!next || next === entity.label) {
      setRenaming(false);
      setRenameError(null);
      return;
    }
    try {
      await api.updateEntityLabel(entity.id, next);
      onPatchEntity?.(entity.id, { label: next });
      setRenaming(false);
      setRenameError(null);
    } catch (e) {
      setRenameError(String(e));
    }
  }, [renameDraft, entity.id, entity.label, onPatchEntity]);

  // ─── Notes (debounced save on blur) ────────────────────────────
  const [notes, setNotes] = useState(entity.notes ?? "");
  // Reset notes draft to match the entity on selection change so notes
  // from a previously-selected entity don't bleed over.
  useEffect(() => {
    setNotes(entity.notes ?? "");
  }, [entity.id, entity.notes]);
  const saveNotes = useCallback(async () => {
    if ((entity.notes ?? "") === notes) return;
    try {
      await api.updateEntityNotes(entity.id, notes || null);
      onPatchEntity?.(entity.id, { notes: notes || null });
    } catch (e) {
      // eslint-disable-next-line no-console
      console.error("save notes:", e);
    }
  }, [entity.id, entity.notes, notes, onPatchEntity]);

  // ─── Lock / pin ───────────────────────────────────────────────
  const toggleLocked = useCallback(async () => {
    const next = !entity.locked;
    onPatchEntity?.(entity.id, { locked: next });
    try {
      await api.updateEntityLocked(entity.id, next);
    } catch (e) {
      onPatchEntity?.(entity.id, { locked: !next });
      // eslint-disable-next-line no-console
      console.error("toggle locked:", e);
    }
  }, [entity.id, entity.locked, onPatchEntity]);

  // ─── Per-entity transform-run history ─────────────────────────
  const [history, setHistory] = useState<TransformRun[] | null>(null);
  useEffect(() => {
    let cancelled = false;
    setHistory(null);
    api
      .listTransformRunsForEntity(entity.id, 25)
      .then((rows) => {
        if (!cancelled) setHistory(rows);
      })
      .catch(() => {
        if (!cancelled) setHistory([]);
      });
    return () => {
      cancelled = true;
    };
  }, [entity.id]);

  // ─── Bookmark ──────────────────────────────────────────────────
  const toggleBookmark = useCallback(async () => {
    const next = !entity.bookmarked;
    // Optimistic update first so the click feels instant.
    onPatchEntity?.(entity.id, { bookmarked: next });
    try {
      await api.updateEntityBookmark(entity.id, next);
    } catch (e) {
      // Roll back on failure.
      onPatchEntity?.(entity.id, { bookmarked: !next });
      // eslint-disable-next-line no-console
      console.error("toggle bookmark:", e);
    }
  }, [entity.id, entity.bookmarked, onPatchEntity]);

  return (
    <aside className="inspector">
      <header className="inspector-header">
        <div className="inspector-header-top">
          <span className="badge" style={badgeStyle}>
            {canonical?.label ?? entity.entity_type}
          </span>
          {entity.source_event_type && (
            <span className="sub-badge" title="Original SpiderFoot event type">
              {entity.source_event_type}
            </span>
          )}
          {openableUrl && (
            <button
              className="icon-btn"
              onClick={onOpen}
              title={`Open ${openableUrl}`}
              aria-label="Open URL in browser"
            >
              <svg
                viewBox="0 0 16 16"
                width="14"
                height="14"
                aria-hidden="true"
                fill="none"
                stroke="currentColor"
                strokeWidth="1.5"
                strokeLinecap="round"
                strokeLinejoin="round"
              >
                <path d="M10 3h3v3" />
                <path d="M13 3L7 9" />
                <path d="M13 9.5V12a1 1 0 01-1 1H4a1 1 0 01-1-1V4a1 1 0 011-1h2.5" />
              </svg>
            </button>
          )}
          <button
            className={`icon-btn${entity.locked ? " lock-on" : ""}`}
            onClick={toggleLocked}
            title={
              entity.locked
                ? "Unlock — node moves with layouts"
                : "Pin position — locked nodes don't move during layouts"
            }
            aria-label="Toggle pinned position"
            aria-pressed={!!entity.locked}
          >
            <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
              {entity.locked ? (
                <path
                  d="M5 7V5a3 3 0 016 0v2m-7 0h8v6a1 1 0 01-1 1H5a1 1 0 01-1-1V7z"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="1.4"
                  strokeLinejoin="round"
                />
              ) : (
                <path
                  d="M5 7V5a3 3 0 015.6-1.4M4 7h8v6a1 1 0 01-1 1H5a1 1 0 01-1-1V7z"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="1.4"
                  strokeLinejoin="round"
                />
              )}
            </svg>
          </button>
          <button
            className={`icon-btn${entity.bookmarked ? " bookmark-on" : ""}`}
            onClick={toggleBookmark}
            disabled={!!readOnly}
            title={
              readOnly
                ? "Case locked — read-only"
                : entity.bookmarked
                  ? "Remove bookmark"
                  : "Bookmark this entity"
            }
            aria-label="Toggle bookmark"
            aria-pressed={!!entity.bookmarked}
          >
            <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
              <path
                d="M8 1.3l1.95 3.95 4.35.63-3.15 3.07.74 4.32L8 11.27l-3.89 2.0.74-4.32L1.7 5.88l4.35-.63L8 1.3z"
                fill={entity.bookmarked ? "currentColor" : "none"}
                stroke="currentColor"
                strokeWidth="1.3"
                strokeLinejoin="round"
              />
            </svg>
          </button>
          <button
            className="icon-btn danger"
            onClick={onDelete}
            disabled={busy || !!readOnly}
            title={readOnly ? "Case locked — read-only" : "Delete entity (Del)"}
            aria-label="Delete entity"
          >
            <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
              <path
                d="M5 4V3a1 1 0 011-1h4a1 1 0 011 1v1m-7 0h10m-9 0v9a2 2 0 002 2h4a2 2 0 002-2V4M7 7v6m2-6v6"
                stroke="currentColor"
                strokeWidth="1.4"
                fill="none"
                strokeLinecap="round"
              />
            </svg>
          </button>
        </div>
        {renaming ? (
          <div className="inspector-rename">
            <input
              className="inspector-rename-input"
              autoFocus
              value={renameDraft}
              onChange={(e) => setRenameDraft(e.currentTarget.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  e.preventDefault();
                  commitRename();
                } else if (e.key === "Escape") {
                  e.preventDefault();
                  setRenaming(false);
                  setRenameError(null);
                }
              }}
              onBlur={commitRename}
            />
            {renameError && (
              <span className="error-banner" role="alert">
                {renameError}
              </span>
            )}
          </div>
        ) : (
          <h2
            title={
              readOnly
                ? entity.label
                : `${entity.label} (double-click to rename)`
            }
            onDoubleClick={() => {
              if (readOnly) return;
              setRenameDraft(entity.label);
              setRenameError(null);
              setRenaming(true);
            }}
          >
            {entity.label}
          </h2>
        )}
      </header>

      {!isCollection && matchingMachine && onRunMachine && (
        <section className="inspector-machine-cta">
          <div className="inspector-machine-cta-head">
            <span className="inspector-machine-cta-tag">Recommended</span>
            <span className="inspector-machine-cta-name">
              {matchingMachine.name}
            </span>
            <span className="muted small">
              {matchingMachine.step_count}{" "}
              {matchingMachine.step_count === 1 ? "step" : "steps"}
            </span>
          </div>
          <div className="inspector-machine-cta-desc muted small">
            {matchingMachine.description}
          </div>
          <button
            className="inspector-machine-cta-btn"
            disabled={busy || !!readOnly}
            onClick={() => onRunMachine(matchingMachine.id)}
            title={
              readOnly
                ? "Case locked — read-only"
                : `Run the ${matchingMachine.name} machine on ${entity.label}`
            }
          >
            ▶ Run machine on {entity.label}
          </button>
        </section>
      )}

      {isCollection && collectionPayload && (
        <CollectionView
          collection={entity}
          payload={collectionPayload}
          busy={busy || !!readOnly}
          onExpand={() => onExpandCollection?.(entity.id)}
        />
      )}

      {isImage && <ImagePreview entityId={entity.id} alt={entity.label} />}

      {typeof fullData?.confidence === "number" && (
        <ConfidenceBadge value={fullData.confidence} />
      )}

      <section className="inspector-notes">
        <h3>Notes</h3>
        <textarea
          className="notes-textarea"
          placeholder={
            readOnly
              ? "Case is locked (read-only)"
              : "What did you learn about this entity? (free-form, persists across sessions)"
          }
          value={notes}
          onChange={(e) => setNotes(e.currentTarget.value)}
          onBlur={saveNotes}
          readOnly={!!readOnly}
          rows={3}
        />
      </section>

      <EntityTagsSection entityId={entity.id} readOnly={readOnly} />

      <ProvenanceSection
        entity={entity}
        onFocusEntity={onFocusEntity}
      />

      <PropertiesSection entity={entity} />


      {!isCollection && fullData?.full_data && (
        <section>
          <h3>Payload</h3>
          <pre className="payload">{fullData.full_data}</pre>
        </section>
      )}

      {history && history.length > 0 && (
        <section className="entity-history">
          <h3>Run history ({history.length})</h3>
          <ul className="entity-history-list">
            {history.slice(0, 8).map((r) => (
              <li
                key={r.id}
                className={`entity-history-row${r.status !== "ok" ? " failed" : ""}`}
                title={r.error_message ?? r.stderr_excerpt ?? r.transform_id}
              >
                <span className={`runlog-status ${r.status}`}>
                  {r.status === "ok" ? "✓" : "✗"}
                </span>
                <code className="t-id">{r.transform_id}</code>
                <span className="muted small">
                  +{r.new_entity_count} ·{" "}
                  {new Date(r.started_at).toLocaleTimeString(undefined, {
                    hour: "2-digit",
                    minute: "2-digit",
                  })}
                </span>
              </li>
            ))}
            {history.length > 8 && (
              <li className="muted small" style={{ padding: "4px 8px" }}>
                …open the Run log for the rest
              </li>
            )}
          </ul>
        </section>
      )}

      {!isCollection && (
        <section className="inspector-transforms">
          <div className="inspector-transforms-head">
            <h3>Transforms ({compatibleTransforms.length})</h3>
            <span className="kbd-inline">/</span>
          </div>
          {!transformsLoading &&
            compatibleTransforms.length > 1 &&
            onRunAll &&
            !readOnly && (
              <button
                className="run-all-btn"
                disabled={busy}
                title={`Run every compatible transform sequentially on ${entity.label}`}
                onClick={() => {
                  // Big batches can run for a while and hit external rate
                  // limits; confirm so the user doesn't fat-finger a 60-
                  // transform fan-out.
                  if (
                    compatibleTransforms.length > 6 &&
                    !window.confirm(
                      `Run all ${compatibleTransforms.length} transforms on "${entity.label}"? This may take a while.`,
                    )
                  )
                    return;
                  onRunAll(
                    entity.id,
                    compatibleTransforms.map((t) => t.id),
                  );
                }}
              >
                <span>▶ Run all</span>
                <span className="run-all-count">
                  {compatibleTransforms.length}
                </span>
              </button>
            )}
          <input
            ref={filterInputRef}
            className="transform-filter"
            placeholder="Filter transforms…"
            value={filter}
            onChange={(e) => setFilter(e.currentTarget.value)}
          />
          {transformsLoading && (
            <div className="transform-skeleton">
              <span /> <span /> <span /> <span />
            </div>
          )}
          {!transformsLoading && compatibleTransforms.length === 0 && (
            <p className="empty">
              No transforms accept {canonical?.label ?? entity.entity_type}.
            </p>
          )}
          {recommended.length > 0 && (
            <div className="transform-group transform-group-recommended">
              <div className="transform-group-label">
                <span className="recommended-star" aria-hidden="true">
                  ★
                </span>
                Recommended for {canonical?.label ?? entity.entity_type}
                <span className="muted small">{recommended.length}</span>
              </div>
              <ul className="transform-list">
                {recommended.map((t) => (
                  <li key={t.id}>
                    <button
                      disabled={busy || !!readOnly}
                      onClick={() => onRunTransform(t.id)}
                      title={
                        readOnly
                          ? "Case locked — read-only"
                          : `${t.name}\n${t.description}`
                      }
                    >
                      <span className="t-id">{t.id}</span>
                      <span className="t-arrow">→</span>
                      <span className="t-produces">
                        {t.produces.slice(0, 3).join(", ") || "(no output)"}
                      </span>
                    </button>
                  </li>
                ))}
              </ul>
            </div>
          )}
          {grouped.map(([source, items]) => (
            <div key={source || "_"} className="transform-group">
              <div className="transform-group-label">
                {SOURCE_LABELS[source] ?? source ?? "Other"}
                <span className="muted small">{items.length}</span>
              </div>
              <ul className="transform-list">
                {items.map((t) => (
                  <li key={t.id}>
                    <button
                      disabled={busy || !!readOnly}
                      onClick={() => onRunTransform(t.id)}
                      title={
                        readOnly
                          ? "Case locked — read-only"
                          : `${t.name}\n${t.description}`
                      }
                    >
                      <span className="t-id">{t.id}</span>
                      <span className="t-arrow">→</span>
                      <span className="t-produces">
                        {t.produces.slice(0, 3).join(", ") || "(no output)"}
                      </span>
                    </button>
                  </li>
                ))}
              </ul>
            </div>
          ))}
        </section>
      )}
    </aside>
  );
}

// ─── Collection child preview ───────────────────────────────────────────

function CollectionView({
  collection: _collection,
  payload,
  busy,
  onExpand,
}: {
  collection: Entity;
  payload: CollectionPayload;
  busy: boolean;
  onExpand: () => void;
}) {
  const previewItems = payload.items.slice(0, 8);
  const remaining = payload.items.length - previewItems.length;
  return (
    <section className="collection-view">
      <div className="collection-summary">
        <div>
          <strong>{payload.items.length}</strong>{" "}
          {payload.collection_of} items
        </div>
        <div className="muted small">
          from <code>{payload.transform_id}</code>
        </div>
      </div>
      <button
        className="collection-expand"
        disabled={busy}
        onClick={onExpand}
        title="Materialise the children as real entities on the graph"
      >
        Expand into {payload.items.length} entities
      </button>
      <h3 style={{ marginTop: 18 }}>Preview</h3>
      <ul className="collection-preview">
        {previewItems.map((it, i) => (
          <li key={i} title={it.data}>
            <span className="collection-preview-label">
              {truncate(it.data, 60)}
            </span>
            {it.source_event_type && (
              <span className="sub-badge">{it.source_event_type}</span>
            )}
          </li>
        ))}
        {remaining > 0 && (
          <li className="muted small" style={{ fontStyle: "italic" }}>
            …and {remaining} more.
          </li>
        )}
      </ul>
    </section>
  );
}

function truncate(s: string, n: number): string {
  if (s.length <= n) return s;
  return `${s.slice(0, n)}…`;
}

// ─── Multi-select inspector ─────────────────────────────────────────────

function MultiSelectInspector({
  entities,
  canonicalTypes,
  transforms,
  transformsLoading,
  busy,
  onRunTransform,
  onDelete,
  onPatchEntity,
  filterInputRef,
  readOnly,
}: Props) {
  const [filter, setFilter] = useState("");
  const { compatible } = useCompatibleTransformsForMany(
    entities,
    canonicalTypes,
    transforms,
    filter,
  );
  const typeCounts = useMemo(() => {
    const m = new Map<string, number>();
    for (const e of entities) {
      m.set(e.entity_type, (m.get(e.entity_type) ?? 0) + 1);
    }
    return [...m.entries()].sort((a, b) => b[1] - a[1]);
  }, [entities]);
  const hasCollection = entities.some((e) => e.entity_type === "Collection");

  return (
    <aside className="inspector">
      <header className="inspector-header">
        <div className="inspector-header-top">
          <span className="badge multi-badge">×{entities.length}</span>
          <button
            className="icon-btn danger"
            onClick={onDelete}
            disabled={busy || !!readOnly}
            title={
              readOnly
                ? "Case locked — read-only"
                : `Delete ${entities.length} entities`
            }
            aria-label={`Delete ${entities.length} entities`}
          >
            <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
              <path
                d="M5 4V3a1 1 0 011-1h4a1 1 0 011 1v1m-7 0h10m-9 0v9a2 2 0 002 2h4a2 2 0 002-2V4M7 7v6m2-6v6"
                stroke="currentColor"
                strokeWidth="1.4"
                fill="none"
                strokeLinecap="round"
              />
            </svg>
          </button>
        </div>
        <h2>{entities.length} entities selected</h2>
        <div className="multi-type-mix">
          {typeCounts.map(([type, n]) => (
            <span key={type} className="multi-type-chip">
              <strong>{n}</strong> {type}
            </span>
          ))}
        </div>
      </header>

      <BulkActions
        entities={entities}
        readOnly={!!readOnly}
        onPatchEntity={onPatchEntity}
      />

      <section className="inspector-transforms">
        <div className="inspector-transforms-head">
          <h3>Run on all ({compatible.length})</h3>
          <span className="kbd-inline">/</span>
        </div>
        {hasCollection && (
          <p className="empty">
            Selection includes a Collection — expand it first, then re-select.
          </p>
        )}
        {!hasCollection && (
          <>
            <input
              ref={filterInputRef}
              className="transform-filter"
              placeholder="Filter transforms compatible with every selected…"
              value={filter}
              onChange={(e) => setFilter(e.currentTarget.value)}
            />
            {transformsLoading && (
              <div className="transform-skeleton">
                <span /> <span /> <span />
              </div>
            )}
            {!transformsLoading && compatible.length === 0 && (
              <p className="empty">No transform accepts every selected entity.</p>
            )}
            <ul className="transform-list">
              {compatible.map((t) => (
                <li key={t.id}>
                  <button
                    disabled={busy || !!readOnly}
                    onClick={() => onRunTransform(t.id)}
                    title={
                      readOnly
                        ? "Case locked — read-only"
                        : `${t.name}\n${t.description}`
                    }
                  >
                    <span className="t-id">{t.id}</span>
                    <span className="t-arrow">×</span>
                    <span className="t-produces">
                      run on {entities.length}
                    </span>
                  </button>
                </li>
              ))}
            </ul>
          </>
        )}
      </section>
    </aside>
  );
}

// ─── Confidence badge ──────────────────────────────────────────────

/// SpiderFoot emits a 0–100 confidence on each ProducedEvent. We surface
/// it in the inspector as a small bar so the user can eyeball "is this a
/// solid hit or a maybe?" — colour switches at 80 / 50.
function ConfidenceBadge({ value }: { value: number }) {
  const pct = Math.max(0, Math.min(100, Math.round(value)));
  const tone = pct >= 80 ? "high" : pct >= 50 ? "med" : "low";
  return (
    <section className="confidence">
      <h3>Confidence</h3>
      <div className={`confidence-row tone-${tone}`}>
        <div
          className="confidence-bar"
          role="progressbar"
          aria-valuenow={pct}
          aria-valuemin={0}
          aria-valuemax={100}
        >
          <span
            className="confidence-bar-fill"
            style={{ width: `${pct}%` }}
          />
        </div>
        <span className="confidence-value">{pct}%</span>
      </div>
    </section>
  );
}

// ─── Image preview ──────────────────────────────────────────────────

/// Inline thumbnail for an Image entity. Fetches a `data:image/...;base64,…`
/// URL via the backend (which caps the read at 8 MB). The fetch is
/// re-triggered when the entityId changes, so reselecting a different
/// image doesn't show the previous one.
function ImagePreview({ entityId, alt }: { entityId: string; alt: string }) {
  const [src, setSrc] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    let cancelled = false;
    setSrc(null);
    setError(null);
    api
      .readEntityFileAsDataUrl(entityId)
      .then((url) => {
        if (!cancelled) setSrc(url);
      })
      .catch((e) => {
        if (!cancelled) setError(String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [entityId]);
  if (error) {
    return (
      <section>
        <h3>Preview</h3>
        <p className="muted small">Preview unavailable: {error}</p>
      </section>
    );
  }
  return (
    <section className="image-preview">
      <h3>Preview</h3>
      {src ? (
        <img src={src} alt={alt} className="image-preview-img" />
      ) : (
        <div className="image-preview-skeleton" />
      )}
    </section>
  );
}

// ─── Provenance section ─────────────────────────────────────────────
//
// Maltrace's evidentiary integrity story lives here. For any entity the
// section answers:
//
//   • Was this manually added, or did a transform produce it?
//   • Which transform, at what time, with what stderr?
//   • What source URL did the originating service claim?
//   • What edges touch it, and which run produced each edge?
//
// All of that is one fetch per entity (`getEntityProvenance`); a click on
// any of the related-entity rows pivots the canvas via `onFocusEntity`.

function ProvenanceSection({
  entity,
  onFocusEntity,
}: {
  entity: Entity;
  onFocusEntity?: (entityId: string) => void;
}) {
  const [prov, setProv] = useState<EntityProvenance | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setProv(null);
    setError(null);
    api
      .getEntityProvenance(entity.id)
      .then((p) => {
        if (!cancelled) setProv(p);
      })
      .catch((e) => {
        if (!cancelled) setError(String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [entity.id]);

  const sourceUrlSafe =
    prov?.source_url && isSafeUrl(prov.source_url) ? prov.source_url : null;
  const openSource = useCallback(() => {
    if (!sourceUrlSafe) return;
    api.openUrl(sourceUrlSafe).catch((err) => {
      // eslint-disable-next-line no-console
      console.error("openUrl failed:", err);
    });
  }, [sourceUrlSafe]);

  return (
    <section className="provenance">
      <h3>Provenance</h3>
      {error && (
        <p className="error-banner" role="alert">
          {error}
        </p>
      )}
      {!prov && !error && <p className="muted small">Loading provenance…</p>}
      {prov && (
        <>
          <dl className="provenance-grid">
            <dt>Created</dt>
            <dd title={prov.created_at}>{formatTimestamp(prov.created_at)}</dd>

            <dt>Origin</dt>
            <dd>
              {prov.run ? (
                <span>
                  Transform run{" "}
                  <code className="prov-run-id">#{prov.run.id}</code>{" "}
                  <span className="muted small">
                    ({prov.run.status})
                  </span>
                </span>
              ) : (
                <span className="muted">
                  Manually added (no transform attribution)
                </span>
              )}
            </dd>

            {prov.transform_id && (
              <>
                <dt>Transform</dt>
                <dd>
                  <code>{prov.transform_id}</code>
                </dd>
              </>
            )}

            {prov.run?.source_label && (
              <>
                <dt>Seed</dt>
                <dd title={prov.run.source_label}>
                  <span className="prov-seed">{prov.run.source_label}</span>
                </dd>
              </>
            )}

            {prov.run?.duration_ms != null && (
              <>
                <dt>Duration</dt>
                <dd>{formatDurationMs(prov.run.duration_ms)}</dd>
              </>
            )}

            {prov.source_url && (
              <>
                <dt>Source</dt>
                <dd>
                  {sourceUrlSafe ? (
                    <button
                      className="prov-source-link"
                      onClick={openSource}
                      title={`Open ${sourceUrlSafe}`}
                    >
                      {truncateMid(sourceUrlSafe, 48)}
                    </button>
                  ) : (
                    <span title={prov.source_url}>
                      {truncateMid(prov.source_url, 48)}
                    </span>
                  )}
                </dd>
              </>
            )}
          </dl>

          {prov.run?.error_message && (
            <div className="prov-error">
              <div className="muted small">Run error</div>
              <pre className="runlog-pre runlog-pre-err">
                {prov.run.error_message}
              </pre>
            </div>
          )}

          <ProvenanceEdgeList
            title={`From (${prov.incoming.length})`}
            edges={prov.incoming}
            onFocusEntity={onFocusEntity}
          />
          <ProvenanceEdgeList
            title={`To (${prov.outgoing.length})`}
            edges={prov.outgoing}
            onFocusEntity={onFocusEntity}
          />
        </>
      )}
    </section>
  );
}

function ProvenanceEdgeList({
  title,
  edges,
  onFocusEntity,
}: {
  title: string;
  edges: Array<{
    edge_id: string;
    other_entity_id: string;
    other_entity_label: string;
    other_entity_type: string;
    transform_id: string | null;
    run_id: number | null;
  }>;
  onFocusEntity?: (entityId: string) => void;
}) {
  if (edges.length === 0) return null;
  return (
    <div className="prov-edges">
      <div className="prov-edges-title">{title}</div>
      <ul className="prov-edges-list">
        {edges.slice(0, 10).map((e) => (
          <li key={e.edge_id}>
            <button
              className="prov-edge-row"
              onClick={() => onFocusEntity?.(e.other_entity_id)}
              title={`${e.other_entity_type} — focus on graph`}
            >
              <span className="prov-edge-type">{e.other_entity_type}</span>
              <span className="prov-edge-label">{e.other_entity_label}</span>
              <span className="prov-edge-meta muted small">
                {e.transform_id ? <code>{e.transform_id}</code> : "manual"}
                {e.run_id != null && (
                  <>
                    {" "}
                    <span className="prov-run-pill">#{e.run_id}</span>
                  </>
                )}
              </span>
            </button>
          </li>
        ))}
        {edges.length > 10 && (
          <li className="muted small">…and {edges.length - 10} more</li>
        )}
      </ul>
    </div>
  );
}

// ─── Properties table ───────────────────────────────────────────────
//
// Investigators read the property table more than the graph. Flattens
// the entity's `data` JSON into a dense key/value grid (only the
// scalar/leaf fields — nested objects show as JSON). Each row is
// copy-able via click.

function PropertiesSection({ entity }: { entity: Entity }) {
  const rows = useMemo(() => collectProperties(entity), [entity]);
  if (rows.length === 0) return null;
  return (
    <section className="properties">
      <h3>Properties ({rows.length})</h3>
      <dl className="properties-grid">
        {rows.map((r, i) => (
          <PropertyRow key={`${r.key}-${i}`} k={r.key} v={r.value} />
        ))}
      </dl>
    </section>
  );
}

function PropertyRow({ k, v }: { k: string; v: string }) {
  const [copied, setCopied] = useState(false);
  const isUrl = isSafeUrl(v);
  const onCopy = useCallback(() => {
    navigator.clipboard?.writeText(v).then(
      () => {
        setCopied(true);
        window.setTimeout(() => setCopied(false), 1100);
      },
      // Quiet clipboard failures (e.g. in non-secure contexts) so this
      // doesn't bubble to the toast layer.
      () => {},
    );
  }, [v]);
  return (
    <>
      <dt title={k}>{k}</dt>
      <dd>
        {isUrl ? (
          <button
            className="prov-source-link"
            onClick={() => api.openUrl(v).catch(() => {})}
            title={`Open ${v}`}
          >
            {truncateMid(v, 56)}
          </button>
        ) : (
          <button
            className="prop-value"
            onClick={onCopy}
            title={copied ? "Copied!" : "Click to copy"}
          >
            {truncateMid(v, 56)}
          </button>
        )}
      </dd>
    </>
  );
}

/// Flatten the entity into a property list. Surfaces:
///   - Top-level scalar fields of `entity.data` (excluding `extra`
///     which gets unpacked one level deeper, where most sidecars stash
///     their structured metadata).
///   - `extra.*` scalar fields (brazuca's title/description/category,
///     face_detect's bbox/det_score, etc.).
///   - The originating `source_event_type` and `entity_type` as well so
///     the table is self-contained — investigators reading this view
///     often want every fact in one place.
function collectProperties(entity: Entity): Array<{ key: string; value: string }> {
  const out: Array<{ key: string; value: string }> = [];
  out.push({ key: "Type", value: entity.entity_type });
  if (entity.source_event_type) {
    out.push({ key: "Source event", value: entity.source_event_type });
  }

  let parsed: Record<string, unknown> | null = null;
  if (entity.data) {
    try {
      const p = JSON.parse(entity.data);
      if (p && typeof p === "object") parsed = p as Record<string, unknown>;
    } catch {
      // Non-JSON payload (rare — used to happen with the old face
      // scraper). Surface as a single Payload row.
      out.push({ key: "Payload", value: entity.data });
      return out;
    }
  }
  if (!parsed) return out;

  // Top-level scalars first.
  for (const [k, v] of Object.entries(parsed)) {
    if (k === "extra") continue;
    pushScalar(out, k, v);
  }

  // Extra fields are sidecar-specific structured metadata — most useful
  // to unpack one level.
  const extra = parsed.extra;
  if (extra && typeof extra === "object" && !Array.isArray(extra)) {
    for (const [k, v] of Object.entries(extra as Record<string, unknown>)) {
      pushScalar(out, k, v);
    }
  }

  return out;
}

function pushScalar(
  out: Array<{ key: string; value: string }>,
  k: string,
  v: unknown,
) {
  if (v == null) return;
  if (
    typeof v === "string" ||
    typeof v === "number" ||
    typeof v === "boolean"
  ) {
    const s = String(v);
    if (s.length === 0) return;
    out.push({ key: k, value: s });
    return;
  }
  // Nested objects / arrays: stringify; the property row truncates so a
  // long JSON blob doesn't blow out the inspector. Skip the duplicate
  // `full_data` we already pulled into the Payload section.
  if (k === "full_data") return;
  try {
    out.push({ key: k, value: JSON.stringify(v) });
  } catch {
    // Unreachable for plain JSON.parse output, but defensive.
  }
}

function formatTimestamp(iso: string): string {
  try {
    const d = new Date(iso);
    return d.toLocaleString();
  } catch {
    return iso;
  }
}

function formatDurationMs(ms: number): string {
  if (ms < 1000) return `${ms} ms`;
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)} s`;
  const m = Math.floor(ms / 60_000);
  const s = Math.floor((ms % 60_000) / 1000);
  return `${m}m${s}s`;
}

/// Middle-ellipsis truncator — preserves the start AND end of long URLs
/// so the protocol + path tail stay visible at a glance.
function truncateMid(s: string, max: number): string {
  if (s.length <= max) return s;
  const head = Math.ceil((max - 1) / 2);
  const tail = Math.floor((max - 1) / 2);
  return `${s.slice(0, head)}…${s.slice(s.length - tail)}`;
}

function EntityTagsSection({
  entityId,
  readOnly,
}: {
  entityId: string;
  readOnly?: boolean;
}) {
  const [tags, setTags] = useState<string[] | null>(null);

  useEffect(() => {
    let cancelled = false;
    setTags(null);
    api
      .listTagsForEntity(entityId)
      .then((t) => {
        if (!cancelled) setTags(t);
      })
      .catch(() => {
        if (!cancelled) setTags([]);
      });
    return () => {
      cancelled = true;
    };
  }, [entityId]);

  const onAdd = useCallback(
    async (tag: string) => {
      const optimistic = Array.from(new Set([...(tags ?? []), tag.toLowerCase()])).sort();
      setTags(optimistic);
      try {
        await api.addTag(entityId, tag);
      } catch {
        // Roll back on failure.
        setTags(tags);
      }
    },
    [entityId, tags],
  );

  const onRemove = useCallback(
    async (tag: string) => {
      const next = (tags ?? []).filter((t) => t !== tag);
      setTags(next);
      try {
        await api.removeTag(entityId, tag);
      } catch {
        setTags(tags);
      }
    },
    [entityId, tags],
  );

  if (tags == null) {
    return (
      <section className="entity-tags">
        <h3>Tags</h3>
        <p className="muted small">Loading…</p>
      </section>
    );
  }
  return (
    <section className="entity-tags">
      <h3>Tags</h3>
      <TagsRow tags={tags} readOnly={readOnly} onAdd={onAdd} onRemove={onRemove} />
    </section>
  );
}

/// Bulk actions for the multi-select inspector — tag, bookmark, and
/// (eventually) merge across the whole selection. Each operation fans
/// out client-side so the optimistic UI update lands before any
/// network round-trip.
function BulkActions({
  entities,
  readOnly,
  onPatchEntity,
}: {
  entities: Entity[];
  readOnly: boolean;
  onPatchEntity?: (id: string, patch: Partial<Entity>) => void;
}) {
  const [tagDraft, setTagDraft] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const bulkTag = useCallback(async () => {
    const t = tagDraft.trim();
    if (!t) return;
    setBusy(true);
    setError(null);
    try {
      await Promise.all(entities.map((e) => api.addTag(e.id, t)));
      setTagDraft("");
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  }, [tagDraft, entities]);

  const allBookmarked = entities.every((e) => e.bookmarked);
  const bulkBookmark = useCallback(
    async (next: boolean) => {
      setBusy(true);
      setError(null);
      // Optimistic: flip them all in parent state first.
      for (const e of entities) onPatchEntity?.(e.id, { bookmarked: next });
      try {
        await Promise.all(entities.map((e) => api.updateEntityBookmark(e.id, next)));
      } catch (err) {
        // Roll back.
        for (const e of entities) onPatchEntity?.(e.id, { bookmarked: !next });
        setError(String(err));
      } finally {
        setBusy(false);
      }
    },
    [entities, onPatchEntity],
  );

  if (readOnly) return null;

  return (
    <section className="bulk-actions">
      <h3>Bulk actions ({entities.length})</h3>
      <div className="bulk-row">
        <input
          type="text"
          className="tag-add-input"
          placeholder="tag to apply to all"
          value={tagDraft}
          onChange={(e) => setTagDraft(e.currentTarget.value)}
          disabled={busy}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              void bulkTag();
            }
          }}
        />
        <button
          onClick={() => void bulkTag()}
          disabled={busy || !tagDraft.trim()}
        >
          Apply tag
        </button>
      </div>
      <div className="bulk-row">
        <button
          onClick={() => void bulkBookmark(!allBookmarked)}
          disabled={busy}
        >
          {allBookmarked
            ? `Remove bookmark from all (${entities.length})`
            : `Bookmark all (${entities.length})`}
        </button>
      </div>
      {error && (
        <p className="error-banner" role="alert">
          {error}
        </p>
      )}
    </section>
  );
}
