import { useMemo } from "react";
import type { CanonicalType, Entity } from "../types";

interface Props {
  entities: Entity[];
  canonicalTypes: CanonicalType[];
  hiddenTypes: string[];
  onToggleType: (typeId: string) => void;
  onShowAll: () => void;
  /// When true, the canvas hides every non-bookmarked entity. Doubles as
  /// a "show me what I starred" quick filter.
  bookmarkedOnly: boolean;
  onToggleBookmarkedOnly: () => void;
}

/// Compact strip across the top of the canvas with one chip per canonical
/// type that actually has entities on the graph. Click toggles visibility;
/// hidden chips dim out + show a slash. "Show all" reappears when
/// anything is hidden so the user can recover without hunting through
/// individual toggles.
///
/// Counts reflect the *current* graph (not the full DB), so it doubles as
/// a quick "what's on this graph and how much" readout.
export function FilterStrip({
  entities,
  canonicalTypes,
  hiddenTypes,
  onToggleType,
  onShowAll,
  bookmarkedOnly,
  onToggleBookmarkedOnly,
}: Props) {
  const bookmarkedCount = useMemo(
    () => entities.filter((e) => e.bookmarked).length,
    [entities],
  );
  const counts = useMemo(() => {
    const m = new Map<string, number>();
    for (const e of entities) m.set(e.entity_type, (m.get(e.entity_type) ?? 0) + 1);
    // Order: most populous first so the chip the user is most likely to
    // care about lands at the start. Ties broken alphabetically for
    // stability across renders.
    return [...m.entries()].sort((a, b) =>
      b[1] !== a[1] ? b[1] - a[1] : a[0].localeCompare(b[0]),
    );
  }, [entities]);

  const typeById = useMemo(() => {
    const m = new Map<string, CanonicalType>();
    for (const t of canonicalTypes) m.set(t.id, t);
    return m;
  }, [canonicalTypes]);

  if (counts.length === 0) return null;

  const hiddenSet = new Set(hiddenTypes);
  const anyHidden = counts.some(([id]) => hiddenSet.has(id));

  return (
    <div className="filter-strip" role="toolbar" aria-label="Filter by entity type">
      {bookmarkedCount > 0 && (
        <button
          className={`filter-chip filter-chip-star${bookmarkedOnly ? " active" : ""}`}
          onClick={onToggleBookmarkedOnly}
          title={
            bookmarkedOnly
              ? "Show all (currently bookmarked only)"
              : "Show only bookmarked entities"
          }
          aria-pressed={bookmarkedOnly}
        >
          <span aria-hidden="true" style={{ color: "#fbbf24" }}>
            ★
          </span>
          <span className="filter-chip-label">Bookmarked</span>
          <span className="filter-chip-count">{bookmarkedCount}</span>
        </button>
      )}
      {counts.map(([typeId, n]) => {
        const t = typeById.get(typeId);
        const hidden = hiddenSet.has(typeId);
        return (
          <button
            key={typeId}
            className={`filter-chip${hidden ? " hidden" : ""}`}
            onClick={() => onToggleType(typeId)}
            title={hidden ? `Show ${t?.label ?? typeId}` : `Hide ${t?.label ?? typeId}`}
            aria-pressed={!hidden}
            style={
              t ? ({ "--chip-color": t.color } as React.CSSProperties) : undefined
            }
          >
            <span className="filter-chip-dot" aria-hidden="true" />
            <span className="filter-chip-label">{t?.label ?? typeId}</span>
            <span className="filter-chip-count">{n}</span>
          </button>
        );
      })}
      {anyHidden && (
        <button
          className="filter-show-all"
          onClick={onShowAll}
          title="Show all hidden types"
        >
          Show all
        </button>
      )}
    </div>
  );
}
