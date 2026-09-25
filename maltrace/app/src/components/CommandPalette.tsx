import { useEffect, useMemo, useRef, useState } from "react";

export interface PaletteAction {
  /// Stable id, used as React key.
  id: string;
  /// Primary display name. The fuzzy matcher searches this + `keywords`.
  label: string;
  /// Short group label that lands as a chip on the right — "Case",
  /// "Graph", "Machine", "Settings". Used only for visual scanning.
  group?: string;
  /// Extra search terms not present in the label (e.g. shortcut keys,
  /// synonyms, machine ids).
  keywords?: string;
  /// Disable the row (greyed out, not selectable). When set, the action
  /// is also explanatory text; selecting it is a no-op.
  disabled?: boolean;
  /// What to do when activated.
  onRun: () => void;
}

interface Props {
  actions: PaletteAction[];
  onClose: () => void;
}

/// Global command palette. Triggered with Cmd/Ctrl+K. Lists every action
/// the app knows about — case operations, machines, settings — with
/// type-ahead filtering. Action source-of-truth lives in App.tsx so the
/// palette stays a dumb list.
///
/// Match policy mirrors GraphFinder: case-insensitive substring on
/// `label + " " + (keywords || "")`, ranked by where the match lands
/// (prefix > word-start > anywhere).
export function CommandPalette({ actions, onClose }: Props) {
  const [query, setQuery] = useState("");
  const [activeIdx, setActiveIdx] = useState(0);
  const inputRef = useRef<HTMLInputElement | null>(null);
  const listRef = useRef<HTMLDivElement | null>(null);

  const results = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return actions;
    return actions
      .map((a) => {
        const hay = (a.label + " " + (a.keywords ?? "")).toLowerCase();
        const idx = hay.indexOf(q);
        if (idx < 0) return null;
        // Score: prefer prefix matches, then word-start, then anywhere.
        const isWordStart = idx === 0 || hay[idx - 1] === " ";
        const score = idx + (isWordStart ? -50 : 0);
        return { a, score };
      })
      .filter((x): x is { a: PaletteAction; score: number } => !!x)
      .sort((x, y) => x.score - y.score)
      .map((x) => x.a);
  }, [actions, query]);

  useEffect(() => {
    setActiveIdx(0);
  }, [query]);

  useEffect(() => {
    inputRef.current?.focus();
    inputRef.current?.select();
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
        setActiveIdx((i) => Math.min(i + 1, Math.max(results.length - 1, 0)));
        return;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        setActiveIdx((i) => Math.max(i - 1, 0));
        return;
      }
      if (e.key === "Enter") {
        e.preventDefault();
        const target = results[activeIdx];
        if (target && !target.disabled) {
          target.onRun();
          onClose();
        }
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [results, activeIdx, onClose]);

  return (
    <div
      className="finder-backdrop"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="finder">
        <div className="finder-search">
          <PaletteIcon />
          <input
            ref={inputRef}
            className="finder-input"
            placeholder="Type a command…"
            value={query}
            onChange={(e) => setQuery(e.currentTarget.value)}
          />
          <span className="muted small-tag">{results.length}</span>
        </div>
        <div className="finder-list" ref={listRef}>
          {results.length === 0 && (
            <div className="finder-empty">No matching command.</div>
          )}
          {results.map((a, i) => (
            <button
              key={a.id}
              data-idx={i}
              className={`finder-item${i === activeIdx ? " active" : ""}`}
              disabled={a.disabled}
              onMouseEnter={() => !a.disabled && setActiveIdx(i)}
              onClick={() => {
                if (!a.disabled) {
                  a.onRun();
                  onClose();
                }
              }}
            >
              <span className="finder-item-label">{a.label}</span>
              {a.group && (
                <span className="sub-badge palette-group">{a.group}</span>
              )}
            </button>
          ))}
        </div>
        <div className="finder-footer muted small">
          <span>
            <span className="kbd-inline">↑↓</span> navigate
          </span>
          <span>
            <span className="kbd-inline">Enter</span> run
          </span>
          <span>
            <span className="kbd-inline">Esc</span> close
          </span>
        </div>
      </div>
    </div>
  );
}

function PaletteIcon() {
  return (
    <svg
      viewBox="0 0 16 16"
      width="14"
      height="14"
      aria-hidden="true"
      style={{ flex: "0 0 auto", color: "var(--muted)" }}
    >
      <path
        d="M3 4h10M3 8h10M3 12h6"
        stroke="currentColor"
        strokeWidth="1.4"
        strokeLinecap="round"
        fill="none"
      />
      <circle cx="13" cy="12" r="1.5" fill="currentColor" />
    </svg>
  );
}
