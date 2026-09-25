import { useEffect, useRef, useState } from "react";
import type { Case } from "../types";

interface Props {
  cases: Case[];
  currentCaseId: number | null;
  busy: boolean;
  onSelect: (id: number) => void;
  onCreate: () => void;
  onRename: (id: number, name: string) => void;
  onDelete: (id: number) => void;
  onExport: (id: number) => void;
  onExportCsv: (id: number) => void;
  onExportPng: (id: number) => void;
  onExportPdf: (id: number) => void;
  onExportStix: (id: number) => void;
  onExportReport: (id: number) => void;
  /// GeoJSON / KML — the case's geolocated entities. Surfaces only
  /// on the cases menu; the geo view itself shows the same data.
  onExportGeoJson: (id: number) => void;
  onExportKml: (id: number) => void;
  /// Export the whole case as a `.tar.gz` for handoff / archival.
  onExportBackup: (id: number) => void;
  /// Restore a `.tar.gz` backup into a brand-new case.
  onRestoreBackup: () => void;
  /// Toggle the case's evidence-lock. The parent reflects the result
  /// into the `cases` array so the UI updates immediately.
  onToggleLock: (id: number, next: boolean) => void;
}

/// Topbar tab strip — one tab per case, plus a "+ new" button. Doubles as
/// the only place to rename / delete / export. Renaming is inline (double-
/// click the active tab); the others live behind a small overflow menu so
/// the tab stays compact.
export function CaseTabs({
  cases,
  currentCaseId,
  busy,
  onSelect,
  onCreate,
  onRename,
  onDelete,
  onExport,
  onExportCsv,
  onExportPng,
  onExportPdf,
  onExportStix,
  onExportReport,
  onExportGeoJson,
  onExportKml,
  onExportBackup,
  onRestoreBackup,
  onToggleLock,
}: Props) {
  const [renamingId, setRenamingId] = useState<number | null>(null);
  const [draft, setDraft] = useState("");
  const [menuOpenFor, setMenuOpenFor] = useState<number | null>(null);
  const renameRef = useRef<HTMLInputElement | null>(null);

  useEffect(() => {
    if (renamingId !== null) {
      renameRef.current?.focus();
      renameRef.current?.select();
    }
  }, [renamingId]);

  function commitRename(id: number) {
    const v = draft.trim();
    if (v) onRename(id, v);
    setRenamingId(null);
    setDraft("");
  }

  return (
    <div className="case-tabs" role="tablist">
      {cases.map((c) => {
        const active = c.id === currentCaseId;
        const renaming = renamingId === c.id;
        return (
          <div
            key={c.id}
            className={`case-tab${active ? " active" : ""}`}
            role="tab"
            aria-selected={active}
            onClick={() => {
              if (!active && !renaming) onSelect(c.id);
            }}
            onDoubleClick={(e) => {
              e.stopPropagation();
              setRenamingId(c.id);
              setDraft(c.name);
            }}
          >
            {renaming ? (
              <input
                ref={renameRef}
                className="case-tab-rename"
                value={draft}
                onChange={(e) => setDraft(e.currentTarget.value)}
                onBlur={() => commitRename(c.id)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    e.preventDefault();
                    commitRename(c.id);
                  } else if (e.key === "Escape") {
                    e.preventDefault();
                    setRenamingId(null);
                    setDraft("");
                  }
                }}
              />
            ) : (
              <span
                className="case-tab-name"
                title={
                  c.locked
                    ? `${c.name} (locked${
                        c.locked_at ? ` ${new Date(c.locked_at).toLocaleString()}` : ""
                      })`
                    : c.name
                }
              >
                {c.locked && (
                  <span
                    className="case-tab-lock"
                    aria-hidden="true"
                    title="Case locked (read-only)"
                  >
                    🔒
                  </span>
                )}
                {c.name}
              </span>
            )}
            {active && !renaming && (
              <span className="case-tab-menu-wrap">
                <button
                  className="case-tab-menu-btn"
                  title="Case actions"
                  aria-label="Case actions"
                  aria-haspopup="menu"
                  aria-expanded={menuOpenFor === c.id}
                  onClick={(e) => {
                    e.stopPropagation();
                    setMenuOpenFor((cur) => (cur === c.id ? null : c.id));
                  }}
                  onBlur={() =>
                    window.setTimeout(() => setMenuOpenFor(null), 100)
                  }
                >
                  ⋯
                </button>
                {menuOpenFor === c.id && (
                  <div className="case-tab-menu" role="menu">
                    <button
                      role="menuitem"
                      onMouseDown={(e) => {
                        e.preventDefault();
                        setRenamingId(c.id);
                        setDraft(c.name);
                        setMenuOpenFor(null);
                      }}
                    >
                      Rename
                    </button>
                    <button
                      role="menuitem"
                      onMouseDown={(e) => {
                        e.preventDefault();
                        onExport(c.id);
                        setMenuOpenFor(null);
                      }}
                    >
                      Export GraphML
                    </button>
                    <button
                      role="menuitem"
                      onMouseDown={(e) => {
                        e.preventDefault();
                        onExportCsv(c.id);
                        setMenuOpenFor(null);
                      }}
                    >
                      Export entities CSV
                    </button>
                    <button
                      role="menuitem"
                      onMouseDown={(e) => {
                        e.preventDefault();
                        onExportPng(c.id);
                        setMenuOpenFor(null);
                      }}
                    >
                      Export graph PNG
                    </button>
                    <button
                      role="menuitem"
                      onMouseDown={(e) => {
                        e.preventDefault();
                        onExportPdf(c.id);
                        setMenuOpenFor(null);
                      }}
                    >
                      Export graph PDF
                    </button>
                    <button
                      role="menuitem"
                      onMouseDown={(e) => {
                        e.preventDefault();
                        onExportStix(c.id);
                        setMenuOpenFor(null);
                      }}
                    >
                      Export STIX 2.1 (.json)
                    </button>
                    <button
                      role="menuitem"
                      onMouseDown={(e) => {
                        e.preventDefault();
                        onExportReport(c.id);
                        setMenuOpenFor(null);
                      }}
                    >
                      Export full HTML report
                    </button>
                    <button
                      role="menuitem"
                      onMouseDown={(e) => {
                        e.preventDefault();
                        onExportGeoJson(c.id);
                        setMenuOpenFor(null);
                      }}
                    >
                      Export geo (GeoJSON)
                    </button>
                    <button
                      role="menuitem"
                      onMouseDown={(e) => {
                        e.preventDefault();
                        onExportKml(c.id);
                        setMenuOpenFor(null);
                      }}
                    >
                      Export geo (KML)
                    </button>
                    <button
                      role="menuitem"
                      onMouseDown={(e) => {
                        e.preventDefault();
                        onExportBackup(c.id);
                        setMenuOpenFor(null);
                      }}
                    >
                      Export case bundle (.tar.gz)
                    </button>
                    <button
                      role="menuitem"
                      onMouseDown={(e) => {
                        e.preventDefault();
                        onRestoreBackup();
                        setMenuOpenFor(null);
                      }}
                    >
                      Restore case from bundle…
                    </button>
                    <button
                      role="menuitem"
                      onMouseDown={(e) => {
                        e.preventDefault();
                        const next = !c.locked;
                        if (
                          next &&
                          !window.confirm(
                            `Lock case "${c.name}"? Once locked, the case becomes read-only — no entity edits, transform runs, or notes until you unlock it. Reads still work.`,
                          )
                        ) {
                          setMenuOpenFor(null);
                          return;
                        }
                        onToggleLock(c.id, next);
                        setMenuOpenFor(null);
                      }}
                    >
                      {c.locked ? "🔓 Unlock (allow edits)" : "🔒 Lock (freeze evidence)"}
                    </button>
                    <button
                      role="menuitem"
                      className="danger"
                      disabled={cases.length <= 1 || !!c.locked}
                      onMouseDown={(e) => {
                        e.preventDefault();
                        if (cases.length <= 1 || c.locked) return;
                        if (window.confirm(`Delete case "${c.name}"?`)) {
                          onDelete(c.id);
                        }
                        setMenuOpenFor(null);
                      }}
                    >
                      Delete
                    </button>
                  </div>
                )}
              </span>
            )}
          </div>
        );
      })}
      <button
        className="case-tab-add"
        title="New case"
        aria-label="New case"
        disabled={busy}
        onClick={onCreate}
      >
        +
      </button>
    </div>
  );
}
