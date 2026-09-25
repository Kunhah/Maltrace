import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api } from "../api";
import type {
  CanonicalType,
  Entity,
  SavedSearch,
  SearchPredicate,
} from "../types";
import { useModalA11y } from "../hooks/useModalA11y";

interface Props {
  caseId: number;
  entities: Entity[];
  canonicalTypes: CanonicalType[];
  readOnly?: boolean;
  /// Apply the predicate: caller filters its own entity list and
  /// selects matches on the graph canvas.
  onApply: (predicate: SearchPredicate) => void;
  onClose: () => void;
}

/// Modal listing the case's saved searches. Each row "applies" (selects
/// matching entities on the canvas) or deletes. The form at the bottom
/// creates a new saved search by picking a name + composing the
/// predicate.
///
/// Predicates are serialised as JSON in the `predicate` column. The
/// backend treats them opaquely; the frontend interprets them
/// client-side via `evaluatePredicate` so changing the grammar doesn't
/// need a migration.
export function SavedSearchesPopover({
  caseId,
  entities,
  canonicalTypes,
  readOnly,
  onApply,
  onClose,
}: Props) {
  const [searches, setSearches] = useState<SavedSearch[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [refreshKey, setRefreshKey] = useState(0);

  useEffect(() => {
    let cancelled = false;
    api
      .listSavedSearches(caseId)
      .then((rows) => {
        if (!cancelled) setSearches(rows);
      })
      .catch((e) => {
        if (!cancelled) setError(String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [caseId, refreshKey]);

  // New-search form state.
  const [name, setName] = useState("");
  const [selectedTypes, setSelectedTypes] = useState<Set<string>>(new Set());
  const [labelContains, setLabelContains] = useState("");
  const [allTags, setAllTags] = useState("");
  const [anyTag, setAnyTag] = useState("");
  const [bookmarkedOnly, setBookmarkedOnly] = useState(false);
  const [hasSourceUrl, setHasSourceUrl] = useState(false);

  const buildPredicate = useCallback((): SearchPredicate => {
    const out: SearchPredicate = {};
    if (selectedTypes.size > 0) out.entity_types = [...selectedTypes];
    if (labelContains.trim()) out.label_contains = labelContains.trim();
    const allList = allTags
      .split(/[,;\s]+/)
      .map((t) => t.trim().toLowerCase())
      .filter(Boolean);
    if (allList.length > 0) out.all_tags = allList;
    const anyList = anyTag
      .split(/[,;\s]+/)
      .map((t) => t.trim().toLowerCase())
      .filter(Boolean);
    if (anyList.length > 0) out.any_tag = anyList;
    if (bookmarkedOnly) out.bookmarked_only = true;
    if (hasSourceUrl) out.has_source_url = true;
    return out;
  }, [selectedTypes, labelContains, allTags, anyTag, bookmarkedOnly, hasSourceUrl]);

  const currentMatchCount = useMemo(() => {
    const pred = buildPredicate();
    return entities.filter((e) => evaluatePredicate(e, pred)).length;
  }, [entities, buildPredicate]);

  const onSave = useCallback(async () => {
    const trimmed = name.trim();
    if (!trimmed) {
      setError("Name required.");
      return;
    }
    const pred = buildPredicate();
    if (Object.keys(pred).length === 0) {
      setError("Predicate is empty — set at least one criterion.");
      return;
    }
    try {
      await api.createSavedSearch(caseId, trimmed, JSON.stringify(pred));
      setName("");
      setSelectedTypes(new Set());
      setLabelContains("");
      setAllTags("");
      setAnyTag("");
      setBookmarkedOnly(false);
      setHasSourceUrl(false);
      setError(null);
      setRefreshKey((k) => k + 1);
    } catch (e) {
      setError(String(e));
    }
  }, [name, buildPredicate, caseId]);

  const apply = useCallback(
    (s: SavedSearch) => {
      try {
        const pred = JSON.parse(s.predicate) as SearchPredicate;
        onApply(pred);
      } catch (e) {
        setError(`bad predicate JSON: ${e}`);
      }
    },
    [onApply],
  );

  const onDelete = useCallback(async (id: number) => {
    if (!window.confirm("Delete this saved search?")) return;
    try {
      await api.deleteSavedSearch(id);
      setRefreshKey((k) => k + 1);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  const toggleType = useCallback((id: string) => {
    setSelectedTypes((cur) => {
      const next = new Set(cur);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }, []);

  const modalRef = useRef<HTMLDivElement | null>(null);
  useModalA11y(modalRef, onClose);

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div
        ref={modalRef}
        className="modal saved-searches-modal"
        onClick={(e) => e.stopPropagation()}
        role="dialog"
        aria-modal="true"
        aria-labelledby="saved-searches-title"
        tabIndex={-1}
      >
        <header>
          <h2 id="saved-searches-title">Saved searches</h2>
          <button className="icon-btn" onClick={onClose} aria-label="Close saved searches">
            ×
          </button>
        </header>

        <section className="saved-searches-list">
          {!searches && <p className="muted small">Loading…</p>}
          {searches && searches.length === 0 && (
            <p className="muted small empty-state">
              No saved searches yet. Build one below — common picks: "All
              unresolved domains", "Bookmarked + sourced", "Tagged
              follow-up".
            </p>
          )}
          {searches?.map((s) => (
            <div key={s.id} className="saved-search-row">
              <div className="saved-search-row-main">
                <div className="saved-search-name">{s.name}</div>
                <div className="muted small">
                  {previewPredicate(s.predicate)}
                </div>
              </div>
              <div className="saved-search-actions">
                <button onClick={() => apply(s)}>Apply</button>
                {!readOnly && (
                  <button
                    className="icon-btn danger"
                    onClick={() => onDelete(s.id)}
                    title="Delete"
                  >
                    ×
                  </button>
                )}
              </div>
            </div>
          ))}
        </section>

        {!readOnly && (
          <section className="saved-search-new">
            <h3>New saved search</h3>
            <div className="bulk-row">
              <input
                type="text"
                placeholder="Name (e.g. follow-up bookmarked)"
                value={name}
                onChange={(e) => setName(e.currentTarget.value)}
              />
            </div>
            <div className="saved-search-types">
              <div className="muted small">Types (any of):</div>
              <div className="saved-search-types-chips">
                {canonicalTypes
                  .filter((t) => t.id !== "Collection")
                  .map((t) => (
                    <button
                      key={t.id}
                      className={`filter-chip${selectedTypes.has(t.id) ? " active" : " hidden"}`}
                      onClick={() => toggleType(t.id)}
                      style={
                        { "--chip-color": t.color } as React.CSSProperties
                      }
                    >
                      <span className="filter-chip-dot" aria-hidden="true" />
                      <span className="filter-chip-label">{t.label}</span>
                    </button>
                  ))}
              </div>
            </div>
            <div className="bulk-row">
              <input
                type="text"
                placeholder="Label contains…"
                value={labelContains}
                onChange={(e) => setLabelContains(e.currentTarget.value)}
              />
            </div>
            <div className="bulk-row">
              <input
                type="text"
                placeholder="All of these tags (comma/space-separated)"
                value={allTags}
                onChange={(e) => setAllTags(e.currentTarget.value)}
              />
            </div>
            <div className="bulk-row">
              <input
                type="text"
                placeholder="Any of these tags"
                value={anyTag}
                onChange={(e) => setAnyTag(e.currentTarget.value)}
              />
            </div>
            <div className="bulk-row">
              <label className="schedule-toggle">
                <input
                  type="checkbox"
                  checked={bookmarkedOnly}
                  onChange={(e) => setBookmarkedOnly(e.currentTarget.checked)}
                />
                <span>Bookmarked only</span>
              </label>
              <label className="schedule-toggle">
                <input
                  type="checkbox"
                  checked={hasSourceUrl}
                  onChange={(e) => setHasSourceUrl(e.currentTarget.checked)}
                />
                <span>Has source URL</span>
              </label>
              <span className="muted small" style={{ marginLeft: "auto" }}>
                Would match {currentMatchCount} entities now
              </span>
            </div>
            {error && (
              <p className="error-banner" role="alert">
                {error}
              </p>
            )}
            <footer className="settings-foot">
              <button
                className="primary"
                onClick={onSave}
                disabled={!name.trim()}
              >
                Save search
              </button>
            </footer>
          </section>
        )}
      </div>
    </div>
  );
}

/// Apply a predicate to one entity. Used both for the live "would
/// match N" counter and for the actual on-apply selection. Tags
/// require the caller to inject a `tags` field — we don't fetch tags
/// per entity here; the caller bulk-fetches `listTagsForCase(caseId)`
/// once and patches every entity before applying. To keep this fn
/// pure, we read tags off `(entity as any)._tags`.
export function evaluatePredicate(
  entity: Entity & { _tags?: string[] },
  pred: SearchPredicate,
): boolean {
  if (pred.entity_types && pred.entity_types.length > 0) {
    if (!pred.entity_types.includes(entity.entity_type)) return false;
  }
  if (pred.label_contains) {
    if (!entity.label.toLowerCase().includes(pred.label_contains.toLowerCase())) {
      return false;
    }
  }
  if (pred.bookmarked_only && !entity.bookmarked) return false;
  if (pred.has_source_url && !entity.source_url) return false;
  const tags = entity._tags ?? [];
  if (pred.all_tags && pred.all_tags.length > 0) {
    for (const t of pred.all_tags) {
      if (!tags.includes(t)) return false;
    }
  }
  if (pred.any_tag && pred.any_tag.length > 0) {
    if (!pred.any_tag.some((t) => tags.includes(t))) return false;
  }
  return true;
}

function previewPredicate(raw: string): string {
  try {
    const p = JSON.parse(raw) as SearchPredicate;
    const parts: string[] = [];
    if (p.entity_types?.length) parts.push(`type ∈ [${p.entity_types.join(", ")}]`);
    if (p.label_contains) parts.push(`label ~ "${p.label_contains}"`);
    if (p.all_tags?.length) parts.push(`tags ⊇ ${p.all_tags.join("+")}`);
    if (p.any_tag?.length) parts.push(`tag ∈ {${p.any_tag.join(", ")}}`);
    if (p.bookmarked_only) parts.push("★");
    if (p.has_source_url) parts.push("has source");
    return parts.length > 0 ? parts.join(" · ") : "(empty)";
  } catch {
    return "(bad json)";
  }
}
