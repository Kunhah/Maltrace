import { useMemo } from "react";
import type { CanonicalType, Entity } from "../types";

interface Props {
  entities: Entity[];
  canonicalTypes: CanonicalType[];
  /// Jump to (select + centre on) the chosen entity, then close the panel.
  onPick: (id: string) => void;
  onClose: () => void;
}

/// Modal listing every bookmarked entity in the current case. Click any
/// row to jump to it on the canvas. The FilterStrip "Bookmarked" chip
/// covers the same ground but as a *filter*; this surface answers "what
/// are all the things I've starred?" without hiding the rest of the graph.
export function BookmarksPanel({
  entities,
  canonicalTypes,
  onPick,
  onClose,
}: Props) {
  const bookmarked = useMemo(
    () => entities.filter((e) => e.bookmarked),
    [entities],
  );
  const typeById = useMemo(() => {
    const m = new Map<string, CanonicalType>();
    for (const t of canonicalTypes) m.set(t.id, t);
    return m;
  }, [canonicalTypes]);

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div
        className="modal bookmarks-modal"
        onClick={(e) => e.stopPropagation()}
      >
        <header>
          <h2>Bookmarks ({bookmarked.length})</h2>
          <button className="icon-btn" onClick={onClose} aria-label="Close">
            ×
          </button>
        </header>
        {bookmarked.length === 0 ? (
          <p className="empty">
            No bookmarks yet. Click the star icon on an entity in the
            inspector to add one.
          </p>
        ) : (
          <ul className="bookmark-list">
            {bookmarked.map((e) => {
              const t = typeById.get(e.entity_type);
              const badgeStyle = t
                ? { color: t.color, borderColor: t.color }
                : undefined;
              return (
                <li key={e.id}>
                  <button
                    className="bookmark-row"
                    onClick={() => {
                      onPick(e.id);
                      onClose();
                    }}
                    title={`Jump to ${e.label}`}
                  >
                    <span className="badge" style={badgeStyle}>
                      {t?.label ?? e.entity_type}
                    </span>
                    <span className="bookmark-row-label">{e.label}</span>
                    {e.notes && <span className="muted small">📝</span>}
                  </button>
                </li>
              );
            })}
          </ul>
        )}
      </div>
    </div>
  );
}
