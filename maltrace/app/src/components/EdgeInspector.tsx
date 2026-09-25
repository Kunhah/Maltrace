import { useEffect, useState } from "react";
import { api } from "../api";
import type { EdgeDetail } from "../types";

interface Props {
  edgeId: string;
  /// Jump the canvas selection to one of the edge's endpoints.
  onFocusEntity: (entityId: string) => void;
  onClose: () => void;
}

/// Floating side-pane shown in place of the entity inspector when the
/// user selects an edge. Surfaces the transform_id + run id that
/// produced the edge — the "where does this link come from" question.
export function EdgeInspector({ edgeId, onFocusEntity, onClose }: Props) {
  const [detail, setDetail] = useState<EdgeDetail | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setDetail(null);
    setError(null);
    api
      .getEdgeDetail(edgeId)
      .then((d) => {
        if (!cancelled) setDetail(d);
      })
      .catch((e) => {
        if (!cancelled) setError(String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [edgeId]);

  return (
    <aside className="inspector">
      <header className="inspector-header">
        <div className="inspector-header-top">
          <span className="badge">Edge</span>
          <button
            className="icon-btn"
            onClick={onClose}
            title="Close edge inspector"
            aria-label="Close edge inspector"
          >
            ×
          </button>
        </div>
        <h2>
          {detail?.edge.label ? (
            <code className="edge-transform-name">{detail.edge.label}</code>
          ) : (
            <span className="muted">Untyped edge</span>
          )}
        </h2>
      </header>
      {error && (
        <p className="error-banner" role="alert">
          {error}
        </p>
      )}
      {!detail && !error && (
        <p className="muted small">Loading edge…</p>
      )}
      {detail && (
        <section className="provenance">
          <h3>Endpoints</h3>
          <ul className="prov-edges-list">
            {detail.source_label && (
              <li>
                <button
                  className="prov-edge-row"
                  onClick={() => onFocusEntity(detail.edge.source_id)}
                  title="Focus source on graph"
                >
                  <span className="prov-edge-type">
                    {detail.source_type ?? "Generic"}
                  </span>
                  <span className="prov-edge-label">
                    {detail.source_label}
                  </span>
                  <span className="prov-edge-meta muted small">source →</span>
                </button>
              </li>
            )}
            {detail.target_label && (
              <li>
                <button
                  className="prov-edge-row"
                  onClick={() => onFocusEntity(detail.edge.target_id)}
                  title="Focus target on graph"
                >
                  <span className="prov-edge-type">
                    {detail.target_type ?? "Generic"}
                  </span>
                  <span className="prov-edge-label">
                    {detail.target_label}
                  </span>
                  <span className="prov-edge-meta muted small">→ target</span>
                </button>
              </li>
            )}
          </ul>

          <RelationshipEditor
            edgeId={detail.edge.id}
            current={detail.edge.relationship_type ?? null}
            onChanged={(next) => {
              setDetail((d) =>
                d
                  ? { ...d, edge: { ...d.edge, relationship_type: next } }
                  : d,
              );
            }}
          />

          <h3 style={{ marginTop: 18 }}>Provenance</h3>
          <dl className="provenance-grid">
            <dt>Created</dt>
            <dd title={detail.edge.created_at}>
              {formatTimestamp(detail.edge.created_at)}
            </dd>

            <dt>Transform</dt>
            <dd>
              {detail.edge.label ? (
                <code>{detail.edge.label}</code>
              ) : (
                <span className="muted">Manual / sample edge</span>
              )}
            </dd>

            {detail.run ? (
              <>
                <dt>Run</dt>
                <dd>
                  <code className="prov-run-id">#{detail.run.id}</code>{" "}
                  <span className="muted small">({detail.run.status})</span>
                </dd>
                {detail.run.source_label && (
                  <>
                    <dt>Seed</dt>
                    <dd>{detail.run.source_label}</dd>
                  </>
                )}
                <dt>Duration</dt>
                <dd>{formatDurationMs(detail.run.duration_ms)}</dd>
              </>
            ) : (
              <>
                <dt>Run</dt>
                <dd>
                  <span className="muted">
                    {detail.edge.run_id != null
                      ? `#${detail.edge.run_id} (record missing)`
                      : "no run record (manual or pre-v7 edge)"}
                  </span>
                </dd>
              </>
            )}
          </dl>

          {detail.run?.error_message && (
            <div className="prov-error">
              <div className="muted small">Run error</div>
              <pre className="runlog-pre runlog-pre-err">
                {detail.run.error_message}
              </pre>
            </div>
          )}
        </section>
      )}
    </aside>
  );
}

function formatTimestamp(iso: string): string {
  try {
    return new Date(iso).toLocaleString();
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

/// Inline editor for the edge's typed `relationship_type`. The value
/// is freeform but offered as a datalist of common relationships so
/// users tend to converge on the same vocabulary across cases.
const RELATIONSHIP_PRESETS = [
  "owns",
  "resolves_to",
  "hosts",
  "uses",
  "abuses_of",
  "controls",
  "registered_by",
  "communicates_with",
  "located_in",
];

function RelationshipEditor({
  edgeId,
  current,
  onChanged,
}: {
  edgeId: string;
  current: string | null;
  onChanged: (next: string | null) => void;
}) {
  const [draft, setDraft] = useState(current ?? "");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setDraft(current ?? "");
  }, [current]);

  const save = async () => {
    const next = draft.trim().toLowerCase();
    if (next === (current ?? "")) return;
    setBusy(true);
    setError(null);
    try {
      await api.updateEdgeRelationship(edgeId, next || null);
      onChanged(next || null);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <section style={{ marginTop: 18 }}>
      <h3>Relationship</h3>
      <div className="bulk-row">
        <input
          type="text"
          list="rel-presets"
          value={draft}
          placeholder="e.g. owns / resolves_to / abuses_of"
          onChange={(e) => setDraft(e.currentTarget.value)}
          onBlur={save}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              void save();
            }
          }}
          disabled={busy}
        />
        <datalist id="rel-presets">
          {RELATIONSHIP_PRESETS.map((p) => (
            <option key={p} value={p} />
          ))}
        </datalist>
      </div>
      {error && (
        <p className="error-banner" role="alert">
          {error}
        </p>
      )}
      <p className="muted small">
        Free-form label distinct from the producer transform. Sets the
        semantic meaning of the link (the transform id is preserved
        below).
      </p>
    </section>
  );
}
