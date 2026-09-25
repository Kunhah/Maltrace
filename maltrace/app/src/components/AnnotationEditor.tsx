import { useEffect, useState } from "react";
import type { Annotation } from "../types";

interface Props {
  annotations: Annotation[];
  /// The currently-selected annotation id. The component renders nothing
  /// when this is null (the parent should fall back to EntityInspector).
  selectedId: string | null;
  onTextChange: (id: string, text: string) => void;
  onColorChange: (id: string, color: string) => void;
  onDelete: (id: string) => void;
}

/// Curated palette for sticky notes. Picked so the dark text on top stays
/// readable on every swatch — pastel saturation, ~80% lightness.
const SWATCHES = [
  "#fbbf24", // amber (default)
  "#fde68a", // pale yellow
  "#fca5a5", // pale red
  "#fdba74", // peach
  "#a7f3d0", // mint
  "#bae6fd", // sky
  "#ddd6fe", // lavender
  "#fbcfe8", // pink
];

/// Inspector sidebar shown when the user has selected a free-floating
/// annotation (not an entity). Lets them edit the text inline, recolour
/// the sticky note, or delete it.
export function AnnotationEditor({
  annotations,
  selectedId,
  onTextChange,
  onColorChange,
  onDelete,
}: Props) {
  const ann = annotations.find((a) => a.id === selectedId) ?? null;
  // Local draft so typing isn't gated on every keystroke round-tripping
  // through the optimistic patch. Synced on selection change.
  const [text, setText] = useState("");
  useEffect(() => {
    setText(ann?.text ?? "");
  }, [ann?.id, ann?.text]);

  if (!ann) return null;

  return (
    <aside className="inspector">
      <header className="inspector-header">
        <div className="inspector-header-top">
          <span className="badge multi-badge">Note</span>
          <button
            className="icon-btn danger"
            title="Delete this note"
            aria-label="Delete note"
            onClick={() => {
              if (window.confirm("Delete this note?")) onDelete(ann.id);
            }}
          >
            <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
              <path
                d="M5 4V3a1 1 0 011-1h4a1 1 0 011 1v1m-7 0h10m-9 0v9a2 2 0 002 2h4a2 2 0 002-2V4M7 7v6m2-6v6"
                stroke="currentColor"
                strokeWidth="1.4"
                fill="none"
                strokeLinecap="round"
              />
            </svg>
          </button>
        </div>
        <h2>Canvas note</h2>
      </header>

      <section>
        <h3>Text</h3>
        <textarea
          className="notes-textarea"
          placeholder="Free-form annotation. Lives on the canvas, not attached to any entity."
          value={text}
          onChange={(e) => setText(e.currentTarget.value)}
          onBlur={() => {
            if (text !== ann.text) onTextChange(ann.id, text);
          }}
          rows={6}
          autoFocus
        />
      </section>

      <section>
        <h3>Colour</h3>
        <div className="type-color-row">
          <input
            type="color"
            value={ann.color}
            onChange={(e) => onColorChange(ann.id, e.currentTarget.value)}
            aria-label="Note background colour"
          />
          <div className="type-color-swatches">
            {SWATCHES.map((c) => (
              <button
                key={c}
                className={`type-color-swatch${c === ann.color ? " active" : ""}`}
                style={{ background: c }}
                onClick={() => onColorChange(ann.id, c)}
                title={c}
                aria-label={`Use colour ${c}`}
              />
            ))}
          </div>
        </div>
      </section>
    </aside>
  );
}
