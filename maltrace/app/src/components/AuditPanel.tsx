import { useCallback, useEffect, useState } from "react";
import { api } from "../api";
import type { AuditEvent } from "../types";

interface Props {
  caseId: number | null;
  onClose: () => void;
}

/// Audit-log modal. The case's mutation history beyond just transform
/// runs: every rename, delete, merge, and lock toggle that touched
/// evidence. Append-only on the backend; this surface is read-only.
///
/// Renders as a modal (not a drawer) because audits are reviewed
/// occasionally, not watched live — the existing RunLogPanel drawer is
/// the right shape for live transform output, but the audit log doesn't
/// need that always-visible affordance.
export function AuditPanel({ caseId, onClose }: Props) {
  const [events, setEvents] = useState<AuditEvent[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(() => {
    if (caseId == null) return;
    api
      .listAuditEvents(caseId, 200)
      .then(setEvents)
      .catch((e) => setError(String(e)));
  }, [caseId]);

  useEffect(() => {
    load();
  }, [load]);

  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div
        className="modal bookmarks-modal"
        onClick={(e) => e.stopPropagation()}
      >
        <header>
          <h2>Audit log ({events?.length ?? "…"})</h2>
          <div style={{ display: "inline-flex", gap: 4 }}>
            <button className="icon-btn" onClick={load} title="Refresh">
              ↻
            </button>
            <button className="icon-btn" onClick={onClose} aria-label="Close">
              ×
            </button>
          </div>
        </header>
        <p className="muted small">
          Append-only record of investigator mutations to this case. Every
          rename, delete, merge, and lock toggle lands here.
        </p>
        {error && <p className="error-banner">{error}</p>}
        {events && events.length === 0 && (
          <p className="empty">No audit events yet.</p>
        )}
        {events && events.length > 0 && (
          <ul className="bookmark-list">
            {events.map((e) => (
              <AuditRow key={e.id} event={e} />
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}

/// Pretty-prints a single audit event. The `kind` verb drives the label
/// and the payload (if any) is parsed and shown inline.
function AuditRow({ event }: { event: AuditEvent }) {
  const { kind, payload, created_at } = event;
  // Action verbs we recognise; everything else falls through to the raw
  // kind. Mapping kept short on purpose — easy to extend.
  const summary = describe(kind, payload);
  return (
    <li>
      <div className="bookmark-row" style={{ cursor: "default" }}>
        <span className={`runlog-status ${actionTone(kind)}`}>
          {actionGlyph(kind)}
        </span>
        <span className="bookmark-row-label">{summary}</span>
        <span className="muted small">
          {new Date(created_at).toLocaleString()}
        </span>
      </div>
    </li>
  );
}

function describe(kind: string, payload: string | null): string {
  let p: Record<string, unknown> = {};
  try {
    if (payload) p = JSON.parse(payload);
  } catch {
    /* keep p empty; raw kind below */
  }
  switch (kind) {
    case "entity_renamed":
      return `Renamed "${p.from ?? "?"}" → "${p.to ?? "?"}"`;
    case "entity_deleted":
      return `Deleted ${p.entity_type ?? "entity"} "${p.label ?? "?"}"`;
    case "entity_merged":
      return `Merged "${p.drop_label ?? "?"}" → "${p.keep_label ?? "?"}"`;
    case "case_locked":
      return "Case locked (read-only)";
    case "case_unlocked":
      return "Case unlocked";
    default:
      return payload ? `${kind} — ${payload}` : kind;
  }
}

function actionTone(kind: string): "ok" | "error" {
  // Deletes / merges get the "error" tone (red dot) because they're
  // destructive; everything else is neutral / green.
  if (kind === "entity_deleted" || kind === "entity_merged") return "error";
  return "ok";
}

function actionGlyph(kind: string): string {
  switch (kind) {
    case "entity_renamed":
      return "✎";
    case "entity_deleted":
      return "✗";
    case "entity_merged":
      return "⇄";
    case "case_locked":
      return "🔒";
    case "case_unlocked":
      return "🔓";
    default:
      return "·";
  }
}
