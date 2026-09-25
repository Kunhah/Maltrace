import { useCallback, useEffect, useState } from "react";
import { api } from "../api";
import type { CaseDiff, CaseSnapshotMeta } from "../types";

interface Props {
  caseId: number;
  readOnly?: boolean;
  onClose: () => void;
}

/// Investigator-facing snapshot manager. Snapshots are the "what did
/// the graph look like before that machine ran" mechanism — distinct
/// from per-entity undo (EntitySnapshot) and from transform_runs
/// (action history). Each row offers "Diff vs current" (compare to
/// live state) and "Diff vs…" (pick another snapshot from the list).
///
/// The diff result renders inline below the row that produced it —
/// keeps the user oriented around which snapshot they're comparing.
export function SnapshotsPanel({ caseId, readOnly, onClose }: Props) {
  const [snapshots, setSnapshots] = useState<CaseSnapshotMeta[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [refreshKey, setRefreshKey] = useState(0);
  const [name, setName] = useState("");
  const [diff, setDiff] = useState<{
    snapshotId: number;
    against: number | null;
    result: CaseDiff;
  } | null>(null);

  useEffect(() => {
    let cancelled = false;
    api
      .listCaseSnapshots(caseId)
      .then((rows) => {
        if (!cancelled) setSnapshots(rows);
      })
      .catch((e) => {
        if (!cancelled) setError(String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [caseId, refreshKey]);

  const onCreate = useCallback(async () => {
    const trimmed = name.trim();
    if (!trimmed) {
      setError("Name required.");
      return;
    }
    try {
      await api.createCaseSnapshot(caseId, trimmed);
      setName("");
      setError(null);
      setRefreshKey((k) => k + 1);
    } catch (e) {
      setError(String(e));
    }
  }, [name, caseId]);

  const onDelete = useCallback(async (id: number) => {
    if (!window.confirm("Delete this snapshot?")) return;
    try {
      await api.deleteCaseSnapshot(id);
      setRefreshKey((k) => k + 1);
      setDiff((d) => (d?.snapshotId === id || d?.against === id ? null : d));
    } catch (e) {
      setError(String(e));
    }
  }, []);

  const runDiff = useCallback(
    async (snapshotId: number, against: number | null) => {
      try {
        const result = await api.diffCaseSnapshots(snapshotId, against);
        setDiff({ snapshotId, against, result });
        setError(null);
      } catch (e) {
        setError(String(e));
      }
    },
    [],
  );

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div
        className="modal saved-searches-modal"
        onClick={(e) => e.stopPropagation()}
      >
        <header>
          <h2>Snapshots</h2>
          <button className="icon-btn" onClick={onClose} aria-label="Close">
            ×
          </button>
        </header>

        <section className="saved-searches-list">
          {!snapshots && <p className="muted small">Loading…</p>}
          {snapshots && snapshots.length === 0 && (
            <p className="muted small empty-state">
              No snapshots yet. Snapshot the case before running a noisy
              machine, then "Diff vs current" to see exactly what it added.
            </p>
          )}
          {snapshots?.map((s) => (
            <div key={s.id}>
              <div className="saved-search-row">
                <div className="saved-search-row-main">
                  <div className="saved-search-name">{s.name}</div>
                  <div className="muted small">
                    {new Date(s.created_at).toLocaleString()} ·{" "}
                    {s.entity_count} entities · {s.edge_count} edges
                  </div>
                </div>
                <div className="saved-search-actions">
                  <button onClick={() => runDiff(s.id, null)}>
                    Diff vs current
                  </button>
                  <DiffAgainstPicker
                    snapshot={s}
                    others={snapshots.filter((o) => o.id !== s.id)}
                    onPick={(otherId) => runDiff(s.id, otherId)}
                  />
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
              {diff && diff.snapshotId === s.id && (
                <DiffSummary diff={diff.result} against={diff.against} />
              )}
            </div>
          ))}
        </section>

        {!readOnly && (
          <section className="saved-search-new">
            <h3>New snapshot</h3>
            <div className="bulk-row">
              <input
                type="text"
                placeholder="Name (e.g. before infra-scan)"
                value={name}
                onChange={(e) => setName(e.currentTarget.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") onCreate();
                }}
              />
              <button
                className="primary"
                onClick={onCreate}
                disabled={!name.trim()}
              >
                Snapshot now
              </button>
            </div>
            {error && (
              <p className="error-banner" role="alert">
                {error}
              </p>
            )}
          </section>
        )}
      </div>
    </div>
  );
}

function DiffAgainstPicker({
  others,
  onPick,
}: {
  snapshot: CaseSnapshotMeta;
  others: CaseSnapshotMeta[];
  onPick: (otherId: number) => void;
}) {
  if (others.length === 0) return null;
  return (
    <select
      defaultValue=""
      onChange={(e) => {
        const v = e.currentTarget.value;
        if (v) {
          onPick(Number(v));
          e.currentTarget.value = "";
        }
      }}
      title="Diff against another snapshot"
    >
      <option value="" disabled>
        Diff vs…
      </option>
      {others.map((o) => (
        <option key={o.id} value={o.id}>
          {o.name}
        </option>
      ))}
    </select>
  );
}

function DiffSummary({
  diff,
  against,
}: {
  diff: CaseDiff;
  against: number | null;
}) {
  const totalChanges =
    diff.added_entities.length +
    diff.removed_entities.length +
    diff.renamed_entities.length +
    diff.added_edges.length +
    diff.removed_edges.length;
  return (
    <div className="diff-summary">
      <div className="muted small">
        Diff {against == null ? "vs current state" : "vs other snapshot"} —{" "}
        {totalChanges} change{totalChanges === 1 ? "" : "s"}
      </div>
      {diff.added_entities.length > 0 && (
        <DiffBucket
          label={`+ ${diff.added_entities.length} entit${diff.added_entities.length === 1 ? "y" : "ies"} added`}
          items={diff.added_entities.map(
            (e) => `${e.entity_type}: ${e.label}`,
          )}
        />
      )}
      {diff.removed_entities.length > 0 && (
        <DiffBucket
          label={`− ${diff.removed_entities.length} entit${diff.removed_entities.length === 1 ? "y" : "ies"} removed`}
          items={diff.removed_entities.map(
            (e) => `${e.entity_type}: ${e.label}`,
          )}
        />
      )}
      {diff.renamed_entities.length > 0 && (
        <DiffBucket
          label={`~ ${diff.renamed_entities.length} renamed`}
          items={diff.renamed_entities.map(
            (r) => `${r.entity_type}: ${r.before} → ${r.after}`,
          )}
        />
      )}
      {diff.added_edges.length > 0 && (
        <DiffBucket
          label={`+ ${diff.added_edges.length} edge${diff.added_edges.length === 1 ? "" : "s"} added`}
          items={diff.added_edges.map(
            (e) => `${e.source_id} → ${e.target_id}${e.label ? ` (${e.label})` : ""}`,
          )}
        />
      )}
      {diff.removed_edges.length > 0 && (
        <DiffBucket
          label={`− ${diff.removed_edges.length} edge${diff.removed_edges.length === 1 ? "" : "s"} removed`}
          items={diff.removed_edges.map(
            (e) => `${e.source_id} → ${e.target_id}${e.label ? ` (${e.label})` : ""}`,
          )}
        />
      )}
      {totalChanges === 0 && (
        <div className="muted small">No differences.</div>
      )}
    </div>
  );
}

function DiffBucket({ label, items }: { label: string; items: string[] }) {
  const [open, setOpen] = useState(false);
  return (
    <div className="diff-bucket">
      <button className="diff-bucket-head" onClick={() => setOpen((v) => !v)}>
        {open ? "▾" : "▸"} {label}
      </button>
      {open && (
        <ul className="diff-bucket-body">
          {items.slice(0, 50).map((it, i) => (
            <li key={i} className="small">
              {it}
            </li>
          ))}
          {items.length > 50 && (
            <li className="muted small">
              …and {items.length - 50} more
            </li>
          )}
        </ul>
      )}
    </div>
  );
}
