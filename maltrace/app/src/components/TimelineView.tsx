import { useEffect, useMemo, useRef, useState } from "react";
import type { CanonicalType, Entity, TransformRun } from "../types";
import { api } from "../api";

interface Props {
  caseId: number | null;
  entities: Entity[];
  canonicalTypes: CanonicalType[];
  selectedIds: string[];
  onSelectEntity: (id: string) => void;
  /// Bumped by the parent after each transform run so the timeline
  /// re-fetches runs without polling.
  refreshKey: number;
}

interface TimelineEvent {
  ts: number; // ms since epoch
  kind: "entity" | "run-ok" | "run-error";
  /// What gets drawn on the timeline track.
  color: string;
  label: string;
  sublabel: string;
  /// `entity_id` so click can focus the canvas. None for run-only
  /// events whose source entity has been deleted.
  entityId?: string;
}

/// Horizontal timeline of entity creations + transform runs. Lays
/// everything out on a single time axis so the investigator can read
/// "what happened when" at a glance. Click an event to select the
/// corresponding entity on the graph.
///
/// Implementation notes:
///   - Pure SVG so we don't pull a charting library. Wheel zooms the
///     axis; drag pans.
///   - Run events are only fetched once per `refreshKey` bump — entity
///     events come straight from the `entities` prop the App already
///     has loaded.
export function TimelineView({
  caseId,
  entities,
  canonicalTypes,
  selectedIds,
  onSelectEntity,
  refreshKey,
}: Props) {
  const colorByType = useMemo(() => {
    const m = new Map<string, string>();
    for (const t of canonicalTypes) m.set(t.id, t.color);
    return m;
  }, [canonicalTypes]);

  const [runs, setRuns] = useState<TransformRun[]>([]);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    if (caseId == null) return;
    let cancelled = false;
    api
      .listTransformRuns(caseId, 500)
      .then((r) => {
        if (!cancelled) setRuns(r);
      })
      .catch((e) => {
        if (!cancelled) setError(String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [caseId, refreshKey]);

  const events = useMemo<TimelineEvent[]>(() => {
    const out: TimelineEvent[] = [];
    for (const e of entities) {
      const ts = Date.parse(e.created_at);
      if (!Number.isFinite(ts)) continue;
      out.push({
        ts,
        kind: "entity",
        color: colorByType.get(e.entity_type) ?? "#94a3b8",
        label: e.label,
        sublabel: e.entity_type,
        entityId: e.id,
      });
    }
    for (const r of runs) {
      const ts = Date.parse(r.started_at);
      if (!Number.isFinite(ts)) continue;
      out.push({
        ts,
        kind: r.status === "ok" ? "run-ok" : "run-error",
        color: r.status === "ok" ? "#5eead4" : "#ef4444",
        label: r.transform_id,
        sublabel: r.source_label ?? `run #${r.id}`,
        entityId: r.source_entity_id ?? undefined,
      });
    }
    out.sort((a, b) => a.ts - b.ts);
    return out;
  }, [entities, runs, colorByType]);

  // ─── Time axis with pan + zoom ─────────────────────────────────
  const containerRef = useRef<HTMLDivElement | null>(null);
  const [size, setSize] = useState({ w: 800, h: 360 });
  useEffect(() => {
    if (!containerRef.current) return;
    const obs = new ResizeObserver(() => {
      const r = containerRef.current?.getBoundingClientRect();
      if (r) setSize({ w: Math.max(400, r.width), h: Math.max(220, r.height) });
    });
    obs.observe(containerRef.current);
    return () => obs.disconnect();
  }, []);

  // Establish the initial axis once we have events; thereafter the
  // user-driven pan/zoom takes over and we leave the range alone.
  const [range, setRange] = useState<{ start: number; end: number } | null>(
    null,
  );
  useEffect(() => {
    if (range || events.length === 0) return;
    const min = events[0].ts;
    const max = events[events.length - 1].ts;
    const pad = Math.max(60_000, (max - min) * 0.1);
    setRange({ start: min - pad, end: max + pad });
  }, [events, range]);

  const dragRef = useRef<{ x0: number; start: number; end: number } | null>(
    null,
  );

  const margin = { top: 20, right: 24, bottom: 38, left: 24 };
  const innerW = Math.max(50, size.w - margin.left - margin.right);
  const tracks = 6;
  const trackH =
    (size.h - margin.top - margin.bottom) / Math.max(1, tracks);

  // Place every event on a free track using a tiny first-fit scheduler.
  // O(n * tracks) but the practical n is bounded by the LIMIT on
  // list_transform_runs (500) + visible entities. Fine.
  const positions = useMemo(() => {
    if (!range) return [] as Array<{ x: number; y: number; ev: TimelineEvent }>;
    const spanMs = Math.max(1, range.end - range.start);
    const trackEnds: number[] = Array(tracks).fill(-Infinity);
    const minSpacingPx = 18;
    const out: Array<{ x: number; y: number; ev: TimelineEvent }> = [];
    for (const ev of events) {
      const x = margin.left + ((ev.ts - range.start) / spanMs) * innerW;
      let placed = false;
      for (let t = 0; t < tracks; t++) {
        if (x - trackEnds[t] >= minSpacingPx) {
          trackEnds[t] = x;
          out.push({
            x,
            y: margin.top + t * trackH + trackH / 2,
            ev,
          });
          placed = true;
          break;
        }
      }
      // Overflow: stack on the last track. Rare for normal cases.
      if (!placed) {
        out.push({
          x,
          y: margin.top + (tracks - 1) * trackH + trackH / 2,
          ev,
        });
      }
    }
    return out;
  }, [events, range, innerW, trackH, margin.top, margin.left]);

  const ticks = useMemo(() => {
    if (!range) return [] as Array<{ x: number; label: string }>;
    return computeAxisTicks(range.start, range.end, innerW).map((t) => ({
      x: margin.left + ((t.ts - range.start) / Math.max(1, range.end - range.start)) * innerW,
      label: t.label,
    }));
  }, [range, innerW, margin.left]);

  if (error) {
    return (
      <div className="view-empty">
        <div className="view-empty-title">Timeline unavailable</div>
        <div className="view-empty-sub muted">{error}</div>
      </div>
    );
  }
  if (events.length === 0) {
    return (
      <div className="view-empty">
        <div className="view-empty-title">No events yet</div>
        <div className="view-empty-sub muted">
          Entities and transform runs will appear here as you investigate.
        </div>
      </div>
    );
  }

  const selected = new Set(selectedIds);

  const onWheel = (e: React.WheelEvent<SVGSVGElement>) => {
    if (!range) return;
    e.preventDefault();
    const factor = Math.exp(e.deltaY * 0.001);
    // Zoom around the cursor's time position so it stays under the
    // pointer — classic scrub behaviour.
    const bound = e.currentTarget.getBoundingClientRect();
    const x = e.clientX - bound.left - margin.left;
    const ratio = x / innerW;
    const span = range.end - range.start;
    const newSpan = Math.max(1000, Math.min(span * factor, 1000 * 60 * 60 * 24 * 365 * 50));
    const pivot = range.start + ratio * span;
    setRange({
      start: pivot - ratio * newSpan,
      end: pivot + (1 - ratio) * newSpan,
    });
  };

  const onMouseDown = (e: React.MouseEvent<SVGSVGElement>) => {
    if (!range) return;
    dragRef.current = { x0: e.clientX, start: range.start, end: range.end };
  };
  const onMouseMove = (e: React.MouseEvent<SVGSVGElement>) => {
    if (!dragRef.current || !range) return;
    const dx = e.clientX - dragRef.current.x0;
    const span = dragRef.current.end - dragRef.current.start;
    const shift = (-dx / innerW) * span;
    setRange({
      start: dragRef.current.start + shift,
      end: dragRef.current.end + shift,
    });
  };
  const onMouseUp = () => {
    dragRef.current = null;
  };

  return (
    <div className="timeline" ref={containerRef}>
      <div className="timeline-hint muted small">
        Scroll to zoom · drag to pan · click an event to focus the entity
      </div>
      <svg
        width={size.w}
        height={size.h}
        viewBox={`0 0 ${size.w} ${size.h}`}
        onWheel={onWheel}
        onMouseDown={onMouseDown}
        onMouseMove={onMouseMove}
        onMouseUp={onMouseUp}
        onMouseLeave={onMouseUp}
        style={{ cursor: dragRef.current ? "grabbing" : "grab" }}
      >
        {/* Axis baseline */}
        <line
          x1={margin.left}
          x2={margin.left + innerW}
          y1={size.h - margin.bottom}
          y2={size.h - margin.bottom}
          stroke="#334155"
        />
        {ticks.map((t, i) => (
          <g key={i}>
            <line
              x1={t.x}
              x2={t.x}
              y1={margin.top - 4}
              y2={size.h - margin.bottom}
              stroke="#1f2937"
              strokeDasharray="2,3"
            />
            <text
              x={t.x}
              y={size.h - margin.bottom + 14}
              textAnchor="middle"
              fill="#94a3b8"
              fontSize="10"
              fontFamily="ui-monospace, 'JetBrains Mono', monospace"
            >
              {t.label}
            </text>
          </g>
        ))}
        {positions.map((p, i) => {
          const isSelected = p.ev.entityId
            ? selected.has(p.ev.entityId)
            : false;
          return (
            <g
              key={i}
              transform={`translate(${p.x}, ${p.y})`}
              style={{ cursor: p.ev.entityId ? "pointer" : "default" }}
              onClick={() => p.ev.entityId && onSelectEntity(p.ev.entityId)}
            >
              {p.ev.kind === "entity" && (
                <circle
                  r={isSelected ? 7 : 5}
                  fill={p.ev.color}
                  stroke={isSelected ? "#5eead4" : "#0b1120"}
                  strokeWidth={isSelected ? 2.5 : 1.5}
                />
              )}
              {p.ev.kind === "run-ok" && (
                <rect
                  x={-5}
                  y={-5}
                  width={10}
                  height={10}
                  fill="none"
                  stroke={p.ev.color}
                  strokeWidth={isSelected ? 2.5 : 1.5}
                  transform="rotate(45)"
                />
              )}
              {p.ev.kind === "run-error" && (
                <path
                  d="M -5 -5 L 5 5 M -5 5 L 5 -5"
                  stroke={p.ev.color}
                  strokeWidth="2"
                  strokeLinecap="round"
                />
              )}
              <title>{`${formatTime(p.ev.ts)}\n${p.ev.label}\n${p.ev.sublabel}`}</title>
              <text
                x={8}
                y={3}
                fill="#cbd5e1"
                fontSize="10"
                fontFamily="ui-monospace, 'JetBrains Mono', monospace"
              >
                {truncate(p.ev.label, 26)}
              </text>
            </g>
          );
        })}
      </svg>
      <div className="timeline-legend muted small">
        <span>
          <span
            className="timeline-key"
            style={{ background: "#94a3b8" }}
          />{" "}
          entity created
        </span>
        <span>
          <span
            className="timeline-key timeline-key-diamond"
            style={{ borderColor: "#5eead4" }}
          />{" "}
          transform run (ok)
        </span>
        <span>
          <span
            className="timeline-key timeline-key-cross"
            style={{ color: "#ef4444" }}
          >
            ×
          </span>
          transform run (error)
        </span>
        <span className="muted">
          {events.length} events · {runs.length} runs
        </span>
      </div>
    </div>
  );
}

function truncate(s: string, n: number): string {
  return s.length <= n ? s : `${s.slice(0, n - 1)}…`;
}

function formatTime(ts: number): string {
  try {
    return new Date(ts).toLocaleString();
  } catch {
    return String(ts);
  }
}

/// Pick 4–8 axis ticks across the visible range, snapped to a sensible
/// unit (year / month / day / hour / minute) for the current zoom.
function computeAxisTicks(
  start: number,
  end: number,
  widthPx: number,
): Array<{ ts: number; label: string }> {
  const span = end - start;
  if (span <= 0 || !Number.isFinite(span)) return [];
  const targetTicks = Math.max(4, Math.min(8, Math.round(widthPx / 110)));
  const step = pickStep(span, targetTicks);
  const out: Array<{ ts: number; label: string }> = [];
  const first = Math.ceil(start / step) * step;
  for (let t = first; t <= end; t += step) {
    out.push({ ts: t, label: formatTick(t, step) });
  }
  return out;
}

function pickStep(spanMs: number, targetTicks: number): number {
  const candidates = [
    1000,
    5_000,
    30_000,
    60_000,
    5 * 60_000,
    15 * 60_000,
    60 * 60_000,
    3 * 3600_000,
    6 * 3600_000,
    12 * 3600_000,
    24 * 3600_000,
    7 * 24 * 3600_000,
    30 * 24 * 3600_000,
    365 * 24 * 3600_000,
  ];
  const desired = spanMs / targetTicks;
  let pick = candidates[0];
  for (const c of candidates) {
    if (c >= desired) {
      pick = c;
      break;
    }
    pick = c;
  }
  return pick;
}

function formatTick(ts: number, stepMs: number): string {
  const d = new Date(ts);
  if (stepMs >= 365 * 24 * 3600_000) return `${d.getFullYear()}`;
  if (stepMs >= 24 * 3600_000) {
    return d.toLocaleDateString(undefined, { month: "short", day: "numeric" });
  }
  if (stepMs >= 3600_000) {
    return d.toLocaleTimeString(undefined, {
      month: "short",
      day: "numeric",
      hour: "2-digit",
    } as Intl.DateTimeFormatOptions);
  }
  return d.toLocaleTimeString(undefined, {
    hour: "2-digit",
    minute: "2-digit",
  });
}
