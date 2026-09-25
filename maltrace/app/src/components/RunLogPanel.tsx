import { useCallback, useEffect, useState } from "react";
import { api } from "../api";
import type { TransformRun } from "../types";

interface Props {
  caseId: number | null;
  /// Bumped by the parent after each `runTransform` / `runMachine` /
  /// `expandCollection` call so the panel refreshes without polling.
  refreshKey: number;
  onClose: () => void;
}

/// Bottom drawer listing recent transform runs for the active case. Rows
/// show timing + status; clicking expands to show error message + sidecar
/// stderr. This is where you go when a transform "didn't work" and you
/// want to know why.
export function RunLogPanel({ caseId, refreshKey, onClose }: Props) {
  const [runs, setRuns] = useState<TransformRun[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [expandedId, setExpandedId] = useState<number | null>(null);

  const refresh = useCallback(() => {
    if (caseId == null) return;
    api
      .listTransformRuns(caseId, 100)
      .then(setRuns)
      .catch((e) => setError(String(e)));
  }, [caseId]);

  useEffect(() => {
    refresh();
  }, [refresh, refreshKey]);

  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") {
        // Don't steal Esc when another overlay is open above us; the App
        // handles overlay precedence.
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const okCount = runs?.filter((r) => r.status === "ok").length ?? 0;
  const errCount = runs?.filter((r) => r.status !== "ok").length ?? 0;

  return (
    <div className="runlog">
      <header className="runlog-head">
        <div className="runlog-title">
          <span className="badge">Run log</span>
          <span className="muted small">
            {runs == null
              ? "Loading…"
              : `${runs.length} run${runs.length === 1 ? "" : "s"} · ${okCount} ok · ${errCount} error${errCount === 1 ? "" : "s"}`}
          </span>
        </div>
        <div className="runlog-actions">
          <button
            className="icon-btn"
            onClick={refresh}
            title="Refresh"
            aria-label="Refresh"
          >
            ↻
          </button>
          <button
            className="icon-btn"
            onClick={onClose}
            title="Close run log"
            aria-label="Close run log"
          >
            ×
          </button>
        </div>
      </header>
      {error && (
        <p className="error-banner" role="alert">
          {error}
        </p>
      )}
      {runs && runs.length === 0 && (
        <p className="empty">No runs yet — run a transform to see it here.</p>
      )}
      <ul className="runlog-list">
        {runs?.map((r) => {
          const expanded = expandedId === r.id;
          const failed = r.status !== "ok";
          return (
            <li key={r.id} className={`runlog-row${failed ? " failed" : ""}`}>
              <button
                className="runlog-row-head"
                onClick={() => setExpandedId(expanded ? null : r.id)}
                aria-expanded={expanded}
              >
                <span className={`runlog-status ${r.status}`}>
                  {failed ? "✗" : "✓"}
                </span>
                <code className="runlog-transform">{r.transform_id}</code>
                {r.source_label && (
                  <span className="runlog-source" title={r.source_label}>
                    on <em>{r.source_label}</em>
                  </span>
                )}
                <span className="muted small runlog-meta">
                  +{r.new_entity_count} ·{" "}
                  {formatDuration(r.duration_ms)} · {formatTime(r.started_at)}
                </span>
              </button>
              {expanded && (
                <div className="runlog-detail">
                  {r.error_message && (
                    <div className="runlog-detail-block">
                      <div className="runlog-detail-label">Error</div>
                      <pre className="runlog-pre runlog-pre-err">
                        {r.error_message}
                      </pre>
                    </div>
                  )}
                  {r.stderr_excerpt && (
                    <div className="runlog-detail-block">
                      <div className="runlog-detail-label">
                        Sidecar stderr
                      </div>
                      <pre className="runlog-pre">{r.stderr_excerpt}</pre>
                    </div>
                  )}
                  {!r.error_message && !r.stderr_excerpt && (
                    <p className="muted small">No diagnostics captured.</p>
                  )}
                </div>
              )}
            </li>
          );
        })}
      </ul>
    </div>
  );
}

function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  if (ms < 60000) return `${(ms / 1000).toFixed(1)}s`;
  const m = Math.floor(ms / 60000);
  const s = Math.floor((ms % 60000) / 1000);
  return `${m}m${s}s`;
}

function formatTime(iso: string): string {
  try {
    const d = new Date(iso);
    const now = new Date();
    const sameDay =
      d.getFullYear() === now.getFullYear() &&
      d.getMonth() === now.getMonth() &&
      d.getDate() === now.getDate();
    if (sameDay) {
      return d.toLocaleTimeString(undefined, {
        hour: "2-digit",
        minute: "2-digit",
        second: "2-digit",
      });
    }
    return d.toLocaleString();
  } catch {
    return iso;
  }
}
