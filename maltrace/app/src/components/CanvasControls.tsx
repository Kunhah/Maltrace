import { useState } from "react";
import type { LayoutKind } from "./GraphCanvas";

interface Props {
  onFit: () => void;
  onCenter: () => void;
  onCenterAverage: () => void;
  onZoomIn: () => void;
  onZoomOut: () => void;
  onResetZoom: () => void;
  onLayout: (kind: LayoutKind) => void;
}

const LAYOUTS: Array<{ id: LayoutKind; label: string; hint: string }> = [
  { id: "fcose", label: "Organic", hint: "Force-directed, untangles dense graphs" },
  { id: "dagre", label: "Hierarchical", hint: "Layered top-down — pivot trees" },
  { id: "concentric", label: "Concentric", hint: "High-degree nodes at the center" },
  { id: "circle", label: "Circle", hint: "All nodes around one ring" },
  { id: "grid", label: "Grid", hint: "Uniform grid — useful for screenshots" },
];

/// Floating canvas-controls cluster — Figma/Excalidraw style. Sits bottom-right
/// of the graph and is purely a wrapper around imperative ops on the underlying
/// Cytoscape instance, exposed via the [`GraphHandle`] ref.
export function CanvasControls({
  onFit,
  onCenter,
  onCenterAverage,
  onZoomIn,
  onZoomOut,
  onResetZoom,
  onLayout,
}: Props) {
  const [layoutOpen, setLayoutOpen] = useState(false);

  return (
    <div className="canvas-controls">
      <div className="cc-layout-wrap">
        <button
          className="cc-layout-btn"
          title="Re-layout the graph"
          aria-label="Layout"
          aria-haspopup="menu"
          aria-expanded={layoutOpen}
          onClick={() => setLayoutOpen((o) => !o)}
          onBlur={() => window.setTimeout(() => setLayoutOpen(false), 100)}
        >
          <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
            <circle cx="4" cy="4" r="1.6" fill="currentColor" />
            <circle cx="12" cy="4" r="1.6" fill="currentColor" />
            <circle cx="4" cy="12" r="1.6" fill="currentColor" />
            <circle cx="12" cy="12" r="1.6" fill="currentColor" />
            <path
              d="M4 4l8 8M12 4l-8 8"
              stroke="currentColor"
              strokeWidth="1.2"
              strokeLinecap="round"
              opacity="0.55"
            />
          </svg>
        </button>
        {layoutOpen && (
          <div className="cc-layout-menu" role="menu">
            {LAYOUTS.map((l) => (
              <button
                key={l.id}
                role="menuitem"
                title={l.hint}
                onMouseDown={(e) => {
                  // Use mousedown so onBlur doesn't swallow the click. Then
                  // explicitly close so re-opening doesn't toggle the same
                  // layout twice.
                  e.preventDefault();
                  onLayout(l.id);
                  setLayoutOpen(false);
                }}
              >
                <span className="cc-layout-name">{l.label}</span>
                <span className="cc-layout-hint">{l.hint}</span>
              </button>
            ))}
          </div>
        )}
      </div>
      <span className="canvas-controls-sep" />
      <button title="Zoom in (+)" aria-label="Zoom in" onClick={onZoomIn}>
        <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
          <path d="M8 3v10M3 8h10" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" />
        </svg>
      </button>
      <button title="Zoom out (−)" aria-label="Zoom out" onClick={onZoomOut}>
        <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
          <path d="M3 8h10" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" />
        </svg>
      </button>
      <span className="canvas-controls-sep" />
      <button title="Reset zoom (100%)" aria-label="Reset zoom" onClick={onResetZoom}>
        <span className="cc-text">1:1</span>
      </button>
      <button title="Fit to view (F)" aria-label="Fit to view" onClick={onFit}>
        <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
          <path
            d="M3 6V3h3M13 6V3h-3M3 10v3h3M13 10v3h-3"
            stroke="currentColor"
            strokeWidth="1.6"
            strokeLinecap="round"
            fill="none"
          />
        </svg>
      </button>
      <button title="Center selection" aria-label="Center selection" onClick={onCenter}>
        <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
          <circle cx="8" cy="8" r="2" fill="currentColor" />
          <circle cx="8" cy="8" r="5.4" fill="none" stroke="currentColor" strokeWidth="1.4" />
        </svg>
      </button>
      <button
        title="Center on average of nodes"
        aria-label="Center on average of nodes"
        onClick={onCenterAverage}
      >
        <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
          {/* scattered nodes with a crosshair on their centroid */}
          <circle cx="3.5" cy="4" r="1.3" fill="currentColor" opacity="0.55" />
          <circle cx="12.5" cy="5" r="1.3" fill="currentColor" opacity="0.55" />
          <circle cx="4.5" cy="12" r="1.3" fill="currentColor" opacity="0.55" />
          <circle cx="11.5" cy="12" r="1.3" fill="currentColor" opacity="0.55" />
          <circle cx="8" cy="8" r="1.6" fill="currentColor" />
          <path
            d="M8 3.4v2M8 10.6v2M3.4 8h2M10.6 8h2"
            stroke="currentColor"
            strokeWidth="1.3"
            strokeLinecap="round"
          />
        </svg>
      </button>
    </div>
  );
}
