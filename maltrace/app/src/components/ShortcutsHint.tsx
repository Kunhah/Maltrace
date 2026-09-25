import { useEffect } from "react";

interface Props {
  onClose: () => void;
}

/// Keyboard-shortcut help overlay. Toggled by `?` in the topbar or directly,
/// dismissed by `Esc`, by clicking outside the panel, or via the close button.
export function ShortcutsHint({ onClose }: Props) {
  // Trap nothing fancy — just a backdrop click and Esc handled at App level.
  useEffect(() => {
    document.body.style.overflow = "hidden";
    return () => {
      document.body.style.overflow = "";
    };
  }, []);

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <header>
          <h2>Keyboard shortcuts</h2>
          <button
            className="icon-btn"
            onClick={onClose}
            aria-label="Close shortcuts"
          >
            ×
          </button>
        </header>
        <dl className="shortcut-list">
          <Shortcut keys={["Ctrl/Cmd", "K"]} desc="Command palette" />
          <Shortcut keys={["Ctrl/Cmd", "F"]} desc="Find entity on graph" />
          <Shortcut keys={["Ctrl/Cmd", "Z"]} desc="Undo" />
          <Shortcut keys={["Ctrl/Cmd+Shift", "Z"]} desc="Redo" />
          <Shortcut keys={["/"]} desc="Focus the transform filter" />
          <Shortcut keys={["?"]} desc="Toggle this help panel" />
          <Shortcut keys={["Esc"]} desc="Deselect / close overlay" />
          <Shortcut keys={["Del"]} desc="Delete the selected entity" />
          <Shortcut keys={["Ctrl", "Click"]} desc="Toggle multi-select" />
          <Shortcut keys={["Drag", "empty"]} desc="Box-select multiple nodes" />
          <Shortcut keys={["Scroll"]} desc="Zoom the canvas" />
          <Shortcut keys={["Click", "empty"]} desc="Deselect" />
        </dl>
        <footer>
          <span className="muted">
            Tip: every entity carries the SpiderFoot event type it came from —
            the inspector shows it as a dashed badge.
          </span>
        </footer>
      </div>
    </div>
  );
}

function Shortcut({ keys, desc }: { keys: string[]; desc: string }) {
  return (
    <>
      <dt>
        {keys.map((k, i) => (
          <span key={i}>
            <span className="kbd">{k}</span>
            {i < keys.length - 1 && <span className="kbd-plus">+</span>}
          </span>
        ))}
      </dt>
      <dd>{desc}</dd>
    </>
  );
}
