import { useCallback, useEffect, useState } from "react";
import type {
  Entity,
  MachineMeta,
  MachineSchedule,
  MachineScheduleRun,
  MachineStepResult,
} from "../types";
import { api } from "../api";

interface Props {
  machines: MachineMeta[];
  /// Primary selected entity (the machine's input). null = no selection.
  selectedEntity: Entity | null;
  busy: boolean;
  onRun: (machineId: string) => void;
  onClose: () => void;
  /// Last run's per-step result, surfaced underneath the machine that ran
  /// so the user sees what happened. Cleared when a new machine is run.
  lastRun: { machineId: string; steps: MachineStepResult[] } | null;
  /// Active case id, threaded down so the schedules tab can list /
  /// create rows. Disabled when null.
  caseId: number | null;
  readOnly?: boolean;
}

type Tab = "machines" | "schedules";

export function MachinesPanel({
  machines,
  selectedEntity,
  busy,
  onRun,
  onClose,
  lastRun,
  caseId,
  readOnly,
}: Props) {
  const [tab, setTab] = useState<Tab>("machines");
  const [activeId, setActiveId] = useState<string | null>(
    machines[0]?.id ?? null,
  );

  // Snap to a machine that matches the current selection's type — saves the
  // user a click when they open the panel with the "right" entity selected.
  useEffect(() => {
    if (!selectedEntity) return;
    const match = machines.find((m) => m.input_type === selectedEntity.entity_type);
    if (match) setActiveId(match.id);
  }, [selectedEntity, machines]);

  // Dismiss on Esc.
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

  const active = machines.find((m) => m.id === activeId) ?? null;
  const canRun =
    !!active && !!selectedEntity && selectedEntity.entity_type === active.input_type;
  const runBlockReason = !selectedEntity
    ? "Select an entity to run a machine on it."
    : active && selectedEntity.entity_type !== active.input_type
      ? `This machine expects a ${active.input_type}, but a ${selectedEntity.entity_type} is selected.`
      : null;

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div
        className="modal machines-modal"
        onClick={(e) => e.stopPropagation()}
      >
        <header>
          <h2>Machines</h2>
          <button
            className="icon-btn"
            onClick={async () => {
              try {
                const n = await api.reloadMachines();
                window.alert(`Reloaded ${n} machine${n === 1 ? "" : "s"} from disk.`);
                // Force the panel to re-fetch from upstream — the
                // App reloads via `listMachines` on next interaction.
                // For an immediate refresh, the user can close+reopen.
              } catch (e) {
                window.alert(`Reload failed: ${e}`);
              }
            }}
            title="Re-scan machines/ on disk so JSON edits land without an app restart"
            aria-label="Reload machines from disk"
          >
            ↻
          </button>
          <button className="icon-btn" onClick={onClose} aria-label="Close">
            ×
          </button>
        </header>
        <div className="settings-tabs">
          <button
            className={`settings-tab${tab === "machines" ? " active" : ""}`}
            onClick={() => setTab("machines")}
          >
            Machines
          </button>
          <button
            className={`settings-tab${tab === "schedules" ? " active" : ""}`}
            onClick={() => setTab("schedules")}
            title="Recurring scheduled machine runs"
          >
            Schedules
          </button>
        </div>
        {tab === "schedules" ? (
          <SchedulesTab
            caseId={caseId}
            machines={machines}
            selectedEntity={selectedEntity}
            readOnly={!!readOnly}
          />
        ) : machines.length === 0 ? (
          <p className="empty">
            No machines defined. Drop JSON files into{" "}
            <code>maltrace/machines/</code> and restart.
          </p>
        ) : (
          <div className="machines-grid">
            <ul className="machines-list">
              {machines.map((m) => (
                <li
                  key={m.id}
                  className={`machines-list-item${m.id === activeId ? " active" : ""}`}
                  onClick={() => setActiveId(m.id)}
                >
                  <div className="machines-list-item-name">{m.name}</div>
                  <div className="machines-list-item-meta">
                    <span className="sub-badge">{m.input_type}</span>
                    <span className="muted small">
                      {m.step_count} {m.step_count === 1 ? "step" : "steps"}
                    </span>
                  </div>
                </li>
              ))}
            </ul>
            <div className="machine-detail">
              {active ? (
                <>
                  <div className="machine-detail-head">
                    <h3>{active.name}</h3>
                    <code className="muted">{active.id}</code>
                  </div>
                  {active.description && (
                    <p className="machine-description">{active.description}</p>
                  )}
                  <div className="machine-input">
                    <span className="muted small">Input:</span>{" "}
                    <span className="sub-badge">{active.input_type}</span>
                  </div>
                  <h4>Steps</h4>
                  <ol className="machine-steps">
                    {active.steps.map((s, i) => (
                      <li key={i}>
                        <code className="t-id">{s.transform}</code>
                        <span className="muted small">
                          on {s.on}
                          {s.filter_type ? ` · filter ${s.filter_type}` : ""}
                        </span>
                      </li>
                    ))}
                  </ol>
                  {lastRun?.machineId === active.id && (
                    <>
                      <h4>Last run</h4>
                      <ol className="machine-steps run-results">
                        {lastRun.steps.map((s, i) => (
                          <li key={i}>
                            <code className="t-id">{s.transform}</code>
                            <span className="muted small">
                              {s.targets_run} target
                              {s.targets_run === 1 ? "" : "s"} → +
                              {s.new_entity_count}
                            </span>
                            {s.failures.length > 0 && (
                              <details>
                                <summary className="muted small failure-summary">
                                  {s.failures.length} failure
                                  {s.failures.length === 1 ? "" : "s"}
                                </summary>
                                <ul className="failure-list">
                                  {s.failures.map((f, j) => (
                                    <li key={j}>{f}</li>
                                  ))}
                                </ul>
                              </details>
                            )}
                          </li>
                        ))}
                      </ol>
                    </>
                  )}
                  <div className="machine-run-row">
                    {runBlockReason && (
                      <span className="muted small">{runBlockReason}</span>
                    )}
                    <button
                      className="machine-run-btn"
                      disabled={!canRun || busy}
                      onClick={() => onRun(active.id)}
                    >
                      Run on{" "}
                      <span className="t-id" style={{ marginLeft: 4 }}>
                        {selectedEntity?.label ?? "—"}
                      </span>
                    </button>
                  </div>
                </>
              ) : (
                <p className="empty">Pick a machine on the left.</p>
              )}
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

// ─── Schedules tab ─────────────────────────────────────────────
// Lists existing schedules and lets the user add a new one against
// the currently-selected entity. The backend scheduler ticks every
// 30s; intervals < 60s are clamped server-side.
function SchedulesTab({
  caseId,
  machines,
  selectedEntity,
  readOnly,
}: {
  caseId: number | null;
  machines: MachineMeta[];
  selectedEntity: Entity | null;
  readOnly: boolean;
}) {
  const [schedules, setSchedules] = useState<MachineSchedule[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [refreshKey, setRefreshKey] = useState(0);

  // Form state for the new-schedule row.
  const [draftMachineId, setDraftMachineId] = useState<string>(
    machines[0]?.id ?? "",
  );
  const [intervalValue, setIntervalValue] = useState<number>(6);
  const [intervalUnit, setIntervalUnit] = useState<"minutes" | "hours" | "days">(
    "hours",
  );

  useEffect(() => {
    if (caseId == null) return;
    let cancelled = false;
    api
      .listMachineSchedules(caseId)
      .then((rows) => {
        if (!cancelled) setSchedules(rows);
      })
      .catch((e) => {
        if (!cancelled) setError(String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [caseId, refreshKey]);

  const intervalSeconds = useCallback(() => {
    const mult =
      intervalUnit === "minutes"
        ? 60
        : intervalUnit === "hours"
          ? 3600
          : 86_400;
    return Math.max(60, Math.round(intervalValue * mult));
  }, [intervalValue, intervalUnit]);

  const compatibleMachines = machines.filter(
    (m) => !selectedEntity || m.input_type === selectedEntity.entity_type,
  );

  const onCreate = useCallback(async () => {
    if (!caseId || !selectedEntity || !draftMachineId) return;
    try {
      await api.createMachineSchedule(
        caseId,
        draftMachineId,
        selectedEntity.id,
        intervalSeconds(),
      );
      setRefreshKey((k) => k + 1);
    } catch (e) {
      setError(String(e));
    }
  }, [caseId, selectedEntity, draftMachineId, intervalSeconds]);

  const onToggle = useCallback(async (id: number, enabled: boolean) => {
    try {
      await api.setMachineScheduleEnabled(id, enabled);
      setRefreshKey((k) => k + 1);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  const onDelete = useCallback(async (id: number) => {
    if (!window.confirm("Delete this schedule?")) return;
    try {
      await api.deleteMachineSchedule(id);
      setRefreshKey((k) => k + 1);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  if (caseId == null) {
    return (
      <p className="empty">Open a case to manage scheduled machines.</p>
    );
  }

  return (
    <div className="schedules-tab">
      <p className="muted small">
        Re-run a machine on a seed entity at a fixed interval. New
        entities produced by each run land on the graph live; the run
        log shows every firing under the machine's id.
      </p>

      {/* New-schedule row */}
      <div className="schedule-create">
        <select
          value={draftMachineId}
          onChange={(e) => setDraftMachineId(e.currentTarget.value)}
          disabled={readOnly || compatibleMachines.length === 0}
        >
          {compatibleMachines.length === 0 ? (
            <option value="">
              {selectedEntity
                ? `No machine accepts ${selectedEntity.entity_type}`
                : "Select an entity first"}
            </option>
          ) : (
            compatibleMachines.map((m) => (
              <option key={m.id} value={m.id}>
                {m.name} ({m.input_type})
              </option>
            ))
          )}
        </select>
        <span className="muted small">every</span>
        <input
          type="number"
          min={1}
          max={1000}
          className="schedule-interval-input"
          value={intervalValue}
          onChange={(e) =>
            setIntervalValue(Math.max(1, Number(e.currentTarget.value)))
          }
          disabled={readOnly}
        />
        <select
          value={intervalUnit}
          onChange={(e) =>
            setIntervalUnit(
              e.currentTarget.value as "minutes" | "hours" | "days",
            )
          }
          disabled={readOnly}
        >
          <option value="minutes">minutes</option>
          <option value="hours">hours</option>
          <option value="days">days</option>
        </select>
        <span className="muted small">on</span>
        <code className="t-id">{selectedEntity?.label ?? "(no selection)"}</code>
        <button
          className="primary"
          disabled={
            readOnly ||
            !selectedEntity ||
            compatibleMachines.length === 0 ||
            !draftMachineId
          }
          onClick={onCreate}
        >
          Add schedule
        </button>
      </div>

      {error && <p className="error-banner">{error}</p>}

      {/* Existing schedules */}
      {!schedules ? (
        <p className="muted">Loading…</p>
      ) : schedules.length === 0 ? (
        <p className="empty">No schedules yet.</p>
      ) : (
        <ul className="schedules-list">
          {schedules.map((s) => (
            <ScheduleRow
              key={s.id}
              schedule={s}
              onToggle={onToggle}
              onDelete={onDelete}
              readOnly={readOnly}
            />
          ))}
        </ul>
      )}
    </div>
  );
}

function ScheduleRow({
  schedule,
  onToggle,
  onDelete,
  readOnly,
}: {
  schedule: MachineSchedule;
  onToggle: (id: number, enabled: boolean) => void;
  onDelete: (id: number) => void;
  readOnly: boolean;
}) {
  const [expanded, setExpanded] = useState(false);
  const [runs, setRuns] = useState<MachineScheduleRun[] | null>(null);

  useEffect(() => {
    if (!expanded) return;
    let cancelled = false;
    api
      .listScheduleRuns(schedule.id, 20)
      .then((r) => {
        if (!cancelled) setRuns(r);
      })
      .catch(() => {
        if (!cancelled) setRuns([]);
      });
    return () => {
      cancelled = true;
    };
  }, [expanded, schedule.id]);

  return (
    <li className={`schedule-row${schedule.enabled ? "" : " disabled"}`}>
      <button
        className="schedule-row-head"
        onClick={() => setExpanded((v) => !v)}
        aria-expanded={expanded}
      >
        <div className="schedule-row-title">
          <code>{schedule.machine_id}</code>{" "}
          <span className="muted small">on {schedule.entity_label}</span>
        </div>
        <div className="schedule-row-meta muted small">
          every {formatInterval(schedule.interval_seconds)} · next{" "}
          {formatRelative(schedule.next_run_at)}
          {schedule.last_run_at && (
            <> · last {formatRelative(schedule.last_run_at)}</>
          )}
        </div>
      </button>
      <div className="schedule-row-actions">
        <label className="schedule-toggle">
          <input
            type="checkbox"
            checked={schedule.enabled}
            disabled={readOnly}
            onChange={(e) => onToggle(schedule.id, e.currentTarget.checked)}
          />
          <span>Enabled</span>
        </label>
        <button
          className="icon-btn danger"
          onClick={() => onDelete(schedule.id)}
          disabled={readOnly}
          title="Delete schedule"
        >
          ×
        </button>
      </div>
      {expanded && runs && (
        <ul className="schedule-runs">
          {runs.length === 0 && (
            <li className="muted small">No firings yet.</li>
          )}
          {runs.map((r) => (
            <li
              key={r.id}
              className={r.status === "ok" ? "schedule-run-ok" : "schedule-run-err"}
            >
              <span className="muted small">
                {new Date(r.started_at).toLocaleString()}
              </span>{" "}
              {r.status === "ok" ? (
                <>
                  ✓ +{r.new_entity_count}{" "}
                  {r.new_entity_count === 1 ? "entity" : "entities"}
                </>
              ) : (
                <>✗ {r.error_message ?? "error"}</>
              )}
            </li>
          ))}
        </ul>
      )}
    </li>
  );
}

function formatInterval(seconds: number): string {
  if (seconds < 60) return `${seconds}s`;
  if (seconds < 3600) return `${Math.round(seconds / 60)}m`;
  if (seconds < 86_400) {
    const h = seconds / 3600;
    return Number.isInteger(h) ? `${h}h` : `${h.toFixed(1)}h`;
  }
  const d = seconds / 86_400;
  return Number.isInteger(d) ? `${d}d` : `${d.toFixed(1)}d`;
}

function formatRelative(iso: string): string {
  const t = Date.parse(iso);
  if (!Number.isFinite(t)) return iso;
  const diff = t - Date.now();
  const abs = Math.abs(diff);
  const past = diff < 0;
  const unit = abs < 60_000 ? "now" : abs < 3_600_000 ? "minute" : abs < 86_400_000 ? "hour" : "day";
  if (unit === "now") return past ? "just now" : "now";
  const n =
    unit === "minute"
      ? Math.round(abs / 60_000)
      : unit === "hour"
        ? Math.round(abs / 3_600_000)
        : Math.round(abs / 86_400_000);
  return past ? `${n} ${unit}${n === 1 ? "" : "s"} ago` : `in ${n} ${unit}${n === 1 ? "" : "s"}`;
}
