import { useCallback, useState } from "react";

interface Props {
  tags: string[];
  readOnly?: boolean;
  onAdd: (tag: string) => Promise<void> | void;
  onRemove: (tag: string) => Promise<void> | void;
}

/// Free-form tag chips for the entity inspector. Investigators use tags
/// to cluster the graph beyond the canonical type (e.g. "victim",
/// "infra", "follow-up"). Lower-cased server-side so casing collisions
/// fold together.
export function TagsRow({ tags, readOnly, onAdd, onRemove }: Props) {
  const [draft, setDraft] = useState("");
  const [busy, setBusy] = useState(false);

  const submit = useCallback(async () => {
    const t = draft.trim();
    if (!t) return;
    setBusy(true);
    try {
      await onAdd(t);
      setDraft("");
    } finally {
      setBusy(false);
    }
  }, [draft, onAdd]);

  return (
    <div className="tags-row">
      <ul className="tags-list">
        {tags.map((t) => (
          <li key={t} className="tag-chip">
            <span>{t}</span>
            {!readOnly && (
              <button
                className="tag-remove"
                onClick={() => onRemove(t)}
                aria-label={`Remove tag ${t}`}
                title="Remove tag"
              >
                ×
              </button>
            )}
          </li>
        ))}
        {!readOnly && (
          <li className="tag-add-li">
            <input
              type="text"
              className="tag-add-input"
              placeholder="+ tag"
              value={draft}
              onChange={(e) => setDraft(e.currentTarget.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  e.preventDefault();
                  void submit();
                }
              }}
              disabled={busy}
            />
          </li>
        )}
      </ul>
    </div>
  );
}
