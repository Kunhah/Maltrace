import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import type { CanonicalType, Entity, TransformMeta } from "../types";
import {
  useCompatibleTransforms,
  useCompatibleTransformsForMany,
} from "../hooks/useCompatibleTransforms";

interface Props {
  /// One or more entities under the right-click. Length 1 is the common
  /// case (single right-click); length >1 fires when the user right-clicks
  /// inside an existing multi-selection.
  entities: Entity[];
  canonicalTypes: CanonicalType[];
  transforms: TransformMeta[];
  busy: boolean;
  /// Where the user right-clicked (viewport coordinates).
  x: number;
  y: number;
  onRunTransform: (transformId: string) => void;
  /// Single-entity only — run every compatible transform on the right-
  /// clicked entity. The menu only shows the button when `entities.length`
  /// is 1; running fan-outs against a multi-selection is too footgun-y.
  onRunAll?: (entityId: string, transformIds: string[]) => void;
  /// Two-entity only — merge the second selected entity into the first
  /// (the right-clicked one). Edges from the merged side get re-pointed.
  onMerge?: () => void;
  /// Stealth mode: when true, the compatible-transforms hook filters out
  /// anything not classified as `passive`. The menu also shows a small
  /// banner so the user knows why options may be missing.
  stealthOnly?: boolean;
  onClose: () => void;
}

// Approximate menu footprint, used for auto-flipping when near a viewport edge.
const MENU_WIDTH = 340;
const MENU_MAX_HEIGHT = 380;
const VIEWPORT_PAD = 8;

export function TransformContextMenu({
  entities,
  canonicalTypes,
  transforms,
  busy,
  x,
  y,
  onRunTransform,
  onRunAll,
  onMerge,
  stealthOnly,
  onClose,
}: Props) {
  const [filter, setFilter] = useState("");
  const [sourceFilter, setSourceFilter] = useState<string>("all");
  const [activeIdx, setActiveIdx] = useState(0);
  const menuRef = useRef<HTMLDivElement | null>(null);
  const filterRef = useRef<HTMLInputElement | null>(null);
  const listRef = useRef<HTMLDivElement | null>(null);

  const isMulti = entities.length > 1;
  const primary = entities[0];

  // Single-entity branch uses the precise hook (preserves source_event_type
  // narrowing); multi-entity branch uses the intersection variant.
  const single = useCompatibleTransforms(
    isMulti ? null : primary ?? null,
    canonicalTypes,
    transforms,
    filter,
    !!stealthOnly,
  );
  const many = useCompatibleTransformsForMany(
    isMulti ? entities : [],
    canonicalTypes,
    transforms,
    filter,
    !!stealthOnly,
  );
  const compatible = isMulti ? many.compatible : single.compatible;
  const canonical = single.canonical;

  // Distinct sources present in the compatible list, with a count each.
  // Sorted alphabetically for a stable chip order.
  const sources = useMemo(() => {
    const counts = new Map<string, number>();
    for (const t of compatible) {
      const s = t.source || "unknown";
      counts.set(s, (counts.get(s) ?? 0) + 1);
    }
    return Array.from(counts.entries())
      .map(([source, count]) => ({ source, count }))
      .sort((a, b) => a.source.localeCompare(b.source));
  }, [compatible]);

  // The list the user actually sees + navigates: `compatible` narrowed by the
  // active source chip. Keyboard nav and "run all" both key off this.
  const visible = useMemo(() => {
    if (sourceFilter === "all") return compatible;
    return compatible.filter((t) => (t.source || "unknown") === sourceFilter);
  }, [compatible, sourceFilter]);

  useEffect(() => {
    setActiveIdx(0);
  }, [filter, sourceFilter, entities.map((e) => e.id).join(",")]);

  useLayoutEffect(() => {
    filterRef.current?.focus();
  }, []);

  useEffect(() => {
    const list = listRef.current;
    if (!list) return;
    const item = list.querySelector<HTMLElement>(`[data-idx="${activeIdx}"]`);
    item?.scrollIntoView({ block: "nearest" });
  }, [activeIdx]);

  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
        return;
      }
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setActiveIdx((i) => Math.min(i + 1, Math.max(visible.length - 1, 0)));
        return;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        setActiveIdx((i) => Math.max(i - 1, 0));
        return;
      }
      if (e.key === "Enter" && visible[activeIdx] && !busy) {
        e.preventDefault();
        onRunTransform(visible[activeIdx].id);
        onClose();
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [visible, activeIdx, busy, onRunTransform, onClose]);

  useEffect(() => {
    let alive = false;
    const id = window.setTimeout(() => {
      alive = true;
    }, 0);
    function onDown(e: MouseEvent) {
      if (!alive) return;
      if (!menuRef.current?.contains(e.target as Node)) onClose();
    }
    document.addEventListener("mousedown", onDown);
    return () => {
      window.clearTimeout(id);
      document.removeEventListener("mousedown", onDown);
    };
  }, [onClose]);

  const position = useMemo(() => {
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    const left =
      x + MENU_WIDTH > vw - VIEWPORT_PAD
        ? Math.max(VIEWPORT_PAD, x - MENU_WIDTH)
        : x;
    const top =
      y + MENU_MAX_HEIGHT > vh - VIEWPORT_PAD
        ? Math.max(VIEWPORT_PAD, y - MENU_MAX_HEIGHT)
        : y;
    return { left, top };
  }, [x, y]);

  const badgeStyle = canonical
    ? { color: canonical.color, borderColor: canonical.color }
    : undefined;

  // Header text: in batch mode show count + the mix of canonical types;
  // in single mode show the entity label.
  const header = isMulti ? (
    <span className="context-menu-label">
      {entities.length} selected
      <span className="muted small" style={{ marginLeft: 6 }}>
        {summariseTypes(entities)}
      </span>
    </span>
  ) : primary ? (
    <span className="context-menu-label" title={primary.label}>
      {primary.label}
    </span>
  ) : null;

  const emptyText = isMulti
    ? "No transform accepts every selected entity."
    : primary?.entity_type === "Collection"
      ? "Collections must be expanded before running transforms."
      : `No transforms accept ${canonical?.label ?? primary?.entity_type ?? "this"}.`;

  return (
    <div
      ref={menuRef}
      className="context-menu"
      role="menu"
      aria-label={
        isMulti
          ? `Transforms for ${entities.length} selected entities`
          : `Transforms for ${primary?.label ?? ""}`
      }
      style={{ position: "fixed", ...position, zIndex: 1000 }}
      onContextMenu={(e) => e.preventDefault()}
    >
      <div className="context-menu-header">
        {isMulti ? (
          <span className="badge multi-badge">×{entities.length}</span>
        ) : (
          <span className="badge" style={badgeStyle}>
            {canonical?.label ?? primary?.entity_type}
          </span>
        )}
        {header}
        <span className="muted small-tag">{visible.length}</span>
      </div>
      <input
        ref={filterRef}
        className="context-menu-filter"
        placeholder={
          isMulti
            ? "Filter transforms compatible with all… (↑↓ Enter, Esc)"
            : "Filter transforms… (↑↓ Enter, Esc to close)"
        }
        value={filter}
        onChange={(e) => setFilter(e.currentTarget.value)}
      />
      {sources.length > 1 && (
        <div className="context-menu-sources">
          <button
            className={`context-menu-source-chip${sourceFilter === "all" ? " active" : ""}`}
            onClick={() => setSourceFilter("all")}
          >
            All ({compatible.length})
          </button>
          {sources.map(({ source, count }) => (
            <button
              key={source}
              className={`context-menu-source-chip${sourceFilter === source ? " active" : ""}`}
              onClick={() => setSourceFilter(source)}
            >
              {source} ({count})
            </button>
          ))}
        </div>
      )}
      {isMulti && entities.length === 2 && onMerge && (
        <button
          className="run-all-btn context-menu-run-all"
          title={`Merge "${entities[1].label}" into "${entities[0].label}"`}
          onClick={() => {
            onMerge();
            onClose();
          }}
        >
          <span>⇄ Merge into {entities[0].label}</span>
        </button>
      )}
      {!isMulti && primary && onRunAll && visible.length > 1 && (
        <button
          className="run-all-btn context-menu-run-all"
          disabled={busy}
          title="Run every compatible transform on this entity"
          onClick={() => {
            if (
              visible.length > 6 &&
              !window.confirm(
                `Run all ${visible.length} transforms on "${primary.label}"?`,
              )
            )
              return;
            onRunAll(
              primary.id,
              visible.map((t) => t.id),
            );
            onClose();
          }}
        >
          <span>▶ Run all</span>
          <span className="run-all-count">{visible.length}</span>
        </button>
      )}
      <div className="context-menu-list" ref={listRef}>
        {visible.length === 0 && (
          <div className="context-menu-empty">{emptyText}</div>
        )}
        {visible.map((t, i) => (
          <button
            key={t.id}
            data-idx={i}
            className={`context-menu-item${i === activeIdx ? " active" : ""}`}
            disabled={busy}
            onMouseEnter={() => setActiveIdx(i)}
            onClick={() => {
              onRunTransform(t.id);
              onClose();
            }}
            title={t.description || t.id}
          >
            <span className="t-main">
              <span className="t-name-row">
                <span className="t-name">{t.name || t.id}</span>
                {t.source && <span className="t-source">{t.source}</span>}
                {t.passivity === "active" && (
                  <span
                    className="t-passivity active"
                    title="Active — touches the target directly"
                  >
                    ⚡
                  </span>
                )}
                {t.passivity === "passive" && (
                  <span
                    className="t-passivity passive"
                    title="Passive — third-party indices only"
                  >
                    ◌
                  </span>
                )}
              </span>
              {t.description && (
                <span className="t-desc">{t.description}</span>
              )}
              <span className="t-meta">
                <span className="t-id" title="Module id">
                  {t.id}
                </span>
                <span className="t-produces">
                  {isMulti ? `× ${entities.length} · ` : "→ "}
                  {t.produces.slice(0, 3).join(", ") || "—"}
                </span>
              </span>
            </span>
          </button>
        ))}
      </div>
    </div>
  );
}

/// Compact summary string like "2 Domain, 1 IpAddress" for the multi-select
/// header.
function summariseTypes(entities: Entity[]): string {
  const counts = new Map<string, number>();
  for (const e of entities) {
    counts.set(e.entity_type, (counts.get(e.entity_type) ?? 0) + 1);
  }
  return [...counts.entries()]
    .sort((a, b) => b[1] - a[1])
    .map(([t, n]) => `${n} ${t}`)
    .join(", ");
}
