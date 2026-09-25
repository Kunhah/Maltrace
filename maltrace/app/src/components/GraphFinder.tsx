import { useEffect, useMemo, useRef, useState } from "react";
import type { CanonicalType, CrossCaseMatch, Entity } from "../types";
import { api } from "../api";

interface Props {
  entities: Entity[];
  canonicalTypes: CanonicalType[];
  /// Move the canvas to centre on, and select, this entity (same case).
  onPick: (entityId: string) => void;
  /// Switch to a different case and select an entity there. Called when
  /// the user picks a result from the cross-case search mode.
  onJumpToCase?: (caseId: number, entityId: string) => void;
  onClose: () => void;
}

/// Find-on-graph overlay. Maltego-style: a tight searchbox + ranked list,
/// pop-up with Ctrl/Cmd+F, dismissed with Esc.
///
/// Match policy: case-insensitive substring on entity label, with a tiny
/// weight bump when the query also matches `entity_type` so typing "domain"
/// shows Domains first.
export function GraphFinder({
  entities,
  canonicalTypes,
  onPick,
  onJumpToCase,
  onClose,
}: Props) {
  const [query, setQuery] = useState("");
  const [activeIdx, setActiveIdx] = useState(0);
  /// Cross-case mode flag. When on, hits the backend search endpoint and
  /// renders results with their originating case name.
  const [crossCase, setCrossCase] = useState(false);
  const [crossResults, setCrossResults] = useState<CrossCaseMatch[]>([]);
  const inputRef = useRef<HTMLInputElement | null>(null);
  const listRef = useRef<HTMLDivElement | null>(null);

  // Fetch cross-case results with a tiny debounce so we don't pummel the
  // backend on every keystroke.
  useEffect(() => {
    if (!crossCase) return;
    const q = query.trim();
    if (!q) {
      setCrossResults([]);
      return;
    }
    const t = window.setTimeout(() => {
      api
        .searchEntitiesAcrossCases(q, 50)
        .then(setCrossResults)
        .catch(() => setCrossResults([]));
    }, 120);
    return () => window.clearTimeout(t);
  }, [crossCase, query]);

  const typeById = useMemo(() => {
    const m = new Map<string, CanonicalType>();
    for (const t of canonicalTypes) m.set(t.id, t);
    return m;
  }, [canonicalTypes]);

  const results = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) {
      // Show the most recently added entities so the picker is still
      // useful with an empty box (jump to anything).
      return entities.slice(-30).reverse();
    }
    const scored = entities
      .map((e) => {
        const label = e.label.toLowerCase();
        const type = e.entity_type.toLowerCase();
        const labelHit = label.indexOf(q);
        const typeHit = type.indexOf(q);
        if (labelHit < 0 && typeHit < 0) return null;
        // Lower score = better match. Prefix hits beat substring hits;
        // label hits beat type-only hits.
        const score =
          (labelHit < 0 ? 1000 : labelHit) + (typeHit < 0 ? 0 : -25);
        return { e, score };
      })
      .filter((x): x is { e: Entity; score: number } => !!x)
      .sort((a, b) => a.score - b.score)
      .slice(0, 50)
      .map((x) => x.e);
    return scored;
  }, [entities, query]);

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
        const len = crossCase ? crossResults.length : results.length;
        setActiveIdx((i) => Math.min(i + 1, Math.max(len - 1, 0)));
        return;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        setActiveIdx((i) => Math.max(i - 1, 0));
        return;
      }
      if (e.key === "Enter") {
        e.preventDefault();
        if (crossCase) {
          const r = crossResults[activeIdx];
          if (r) {
            onJumpToCase?.(r.case_id, r.entity_id);
            onClose();
          }
        } else if (results[activeIdx]) {
          onPick(results[activeIdx].id);
          onClose();
        }
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [results, crossResults, crossCase, activeIdx, onPick, onJumpToCase, onClose]);

  return (
    <div
      className="finder-backdrop"
      onMouseDown={(e) => {
        // Click outside the panel dismisses; clicks on the panel are
        // stopped from propagating below.
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="finder">
        <div className="finder-search">
          <SearchIcon />
          <input
            ref={inputRef}
            className="finder-input"
            placeholder={
              crossCase
                ? "Search every case for entities…"
                : "Find on graph… (label or type)"
            }
            value={query}
            onChange={(e) => setQuery(e.currentTarget.value)}
          />
          {onJumpToCase && (
            <button
              className={`secret-row-btn${crossCase ? " danger" : ""}`}
              style={{ marginRight: 6 }}
              onClick={() => {
                setCrossCase((c) => !c);
                setActiveIdx(0);
              }}
              title={
                crossCase
                  ? "Switch back to find-on-this-case"
                  : "Search every case"
              }
            >
              {crossCase ? "This case" : "All cases"}
            </button>
          )}
          <span className="muted small-tag">
            {crossCase ? crossResults.length : results.length}
          </span>
        </div>
        <div className="finder-list" ref={listRef}>
          {crossCase ? (
            <>
              {crossResults.length === 0 && (
                <div className="finder-empty">
                  {query.trim()
                    ? "No matches across cases."
                    : "Type to search every case."}
                </div>
              )}
              {crossResults.map((r, i) => {
                const t = typeById.get(r.entity_type);
                const badgeStyle = t
                  ? { color: t.color, borderColor: t.color }
                  : undefined;
                return (
                  <button
                    key={`${r.case_id}-${r.entity_id}`}
                    data-idx={i}
                    className={`finder-item${i === activeIdx ? " active" : ""}`}
                    onMouseEnter={() => setActiveIdx(i)}
                    onClick={() => {
                      onJumpToCase?.(r.case_id, r.entity_id);
                      onClose();
                    }}
                    title={`${r.label} (in ${r.case_name})`}
                  >
                    <span className="badge" style={badgeStyle}>
                      {t?.label ?? r.entity_type}
                    </span>
                    <span className="finder-item-label">{r.label}</span>
                    {r.bookmarked && (
                      <span style={{ color: "#fbbf24" }} aria-hidden="true">
                        ★
                      </span>
                    )}
                    <span className="sub-badge">in {r.case_name}</span>
                  </button>
                );
              })}
            </>
          ) : (
          <>
          {results.length === 0 && (
            <div className="finder-empty">No matches.</div>
          )}
          {results.map((e, i) => {
            const t = typeById.get(e.entity_type);
            const badgeStyle = t
              ? { color: t.color, borderColor: t.color }
              : undefined;
            return (
              <button
                key={e.id}
                data-idx={i}
                className={`finder-item${i === activeIdx ? " active" : ""}`}
                onMouseEnter={() => setActiveIdx(i)}
                onClick={() => {
                  onPick(e.id);
                  onClose();
                }}
                title={e.label}
              >
                <span className="badge" style={badgeStyle}>
                  {t?.label ?? e.entity_type}
                </span>
                <span className="finder-item-label">{e.label}</span>
                {e.source_event_type && (
                  <span className="sub-badge">{e.source_event_type}</span>
                )}
              </button>
            );
          })}
          </>
          )}
        </div>
        <div className="finder-footer muted small">
          <span>
            <span className="kbd-inline">↑↓</span> navigate
          </span>
          <span>
            <span className="kbd-inline">Enter</span> jump
          </span>
          <span>
            <span className="kbd-inline">Esc</span> close
          </span>
        </div>
      </div>
    </div>
  );
}

function SearchIcon() {
  return (
    <svg
      viewBox="0 0 16 16"
      width="14"
      height="14"
      aria-hidden="true"
      style={{ flex: "0 0 auto", color: "var(--muted)" }}
    >
      <circle
        cx="7"
        cy="7"
        r="4.5"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.5"
      />
      <path
        d="M10.5 10.5L13 13"
        stroke="currentColor"
        strokeWidth="1.5"
        strokeLinecap="round"
      />
    </svg>
  );
}
