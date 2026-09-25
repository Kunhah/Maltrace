import { useCallback, useMemo, useRef, useState } from "react";
import type { CanonicalType, Entity } from "../types";
import { api } from "../api";

interface Props {
  caseId: number;
  canonicalTypes: CanonicalType[];
  onClose: () => void;
  /// Called after a successful import with the freshly-added entities so the
  /// canvas can splat them in. The parent decides selection / layout.
  onImported: (added: Entity[]) => void;
}

type Mode = "bulk" | "csv" | "graphml";

/// Import modal with two modes:
///   * Bulk: pick a type, paste a list of values (one per line). Each
///     becomes an entity. The 80% case — starting an investigation from
///     a list of 50 emails.
///   * CSV: drop or paste a CSV. Header row must include `entity_type` and
///     `label` columns. Extra columns are ignored. Validation happens
///     client-side and shows a preview before commit.
export function ImportPanel({ caseId, canonicalTypes, onClose, onImported }: Props) {
  const [mode, setMode] = useState<Mode>("bulk");
  const [bulkType, setBulkType] = useState(
    canonicalTypes.find((t) => t.id === "Domain")?.id ?? canonicalTypes[0]?.id ?? "",
  );
  const [bulkText, setBulkText] = useState("");
  const [csvText, setCsvText] = useState("");
  const [graphmlText, setGraphmlText] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [progress, setProgress] = useState<{ done: number; total: number } | null>(null);
  const fileInputRef = useRef<HTMLInputElement | null>(null);

  // ─── Bulk parse: one value per line, blank lines + leading/trailing
  // whitespace dropped, hash comments allowed for paste-from-spreadsheet
  // ergonomics.
  const bulkLines = useMemo(() => {
    return bulkText
      .split(/\r?\n/)
      .map((l) => l.trim())
      .filter((l) => l && !l.startsWith("#"));
  }, [bulkText]);

  // ─── CSV parse: minimal RFC 4180 — supports quoted fields with escaped
  // double-quotes. Returns parsed rows, with the first row treated as the
  // header. Errors surface as the `error` field of the return.
  const csvParsed = useMemo(() => {
    if (!csvText.trim()) return null;
    try {
      const rows = parseCsv(csvText);
      if (rows.length === 0) return { rows: [], header: [], data: [], error: "Empty CSV." };
      const header = rows[0].map((s) => s.trim().toLowerCase());
      const typeIdx = header.indexOf("entity_type");
      const labelIdx = header.indexOf("label");
      if (typeIdx === -1 || labelIdx === -1) {
        return {
          rows,
          header,
          data: [],
          error:
            "CSV must include `entity_type` and `label` columns (case-insensitive).",
        };
      }
      const data: { entity_type: string; label: string }[] = [];
      for (let i = 1; i < rows.length; i++) {
        const row = rows[i];
        const t = (row[typeIdx] ?? "").trim();
        const l = (row[labelIdx] ?? "").trim();
        if (!t || !l) continue;
        data.push({ entity_type: t, label: l });
      }
      return { rows, header, data, error: null };
    } catch (e) {
      return { rows: [], header: [], data: [], error: String(e) };
    }
  }, [csvText]);

  const onFile = useCallback(async (file: File) => {
    const text = await file.text();
    setCsvText(text);
    setMode("csv");
  }, []);

  const onDrop = useCallback(
    (e: React.DragEvent) => {
      e.preventDefault();
      const file = e.dataTransfer.files?.[0];
      if (file) onFile(file);
    },
    [onFile],
  );

  const submit = useCallback(async () => {
    setError(null);
    // GraphML mode runs through a dedicated endpoint that parses on
    // the backend; the UI doesn't have to convert to rows first.
    if (mode === "graphml") {
      const xml = graphmlText.trim();
      if (!xml) {
        setError("Paste or load a GraphML document first.");
        return;
      }
      setBusy(true);
      try {
        const res = await api.importCaseGraphml(caseId, xml);
        // The endpoint doesn't return the new entities — refetch the
        // whole entity list rather than reconstruct optimistically.
        const ents = await api.listEntities(caseId);
        onImported(ents);
        if (res.skipped.length > 0) {
          // eslint-disable-next-line no-console
          console.warn("GraphML import skipped:", res.skipped);
        }
        onClose();
      } catch (e) {
        setError(String(e));
      } finally {
        setBusy(false);
      }
      return;
    }
    const items: { entity_type: string; label: string }[] =
      mode === "bulk"
        ? bulkLines.map((l) => ({ entity_type: bulkType, label: l }))
        : csvParsed?.data ?? [];
    if (items.length === 0) {
      setError("Nothing to import.");
      return;
    }
    setBusy(true);
    setProgress({ done: 0, total: items.length });
    const added: Entity[] = [];
    try {
      // Sequential so we don't hammer the DB lock with parallel inserts.
      // Walking-skeleton scale (≤1000 entities) finishes in well under a
      // second even sequentially.
      for (let i = 0; i < items.length; i++) {
        const it = items[i];
        // Lay imported entities out on a loose grid centred on the canvas
        // origin so they don't all collapse to one point.
        const col = i % 8;
        const row = Math.floor(i / 8);
        const e = await api.addEntity(caseId, it.entity_type, it.label, {
          x: col * 120 - 420,
          y: row * 90 - 320,
        });
        added.push(e);
        setProgress({ done: i + 1, total: items.length });
      }
      onImported(added);
      onClose();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
      setProgress(null);
    }
  }, [mode, bulkLines, bulkType, csvParsed, graphmlText, caseId, onImported, onClose]);

  return (
    <div
      className="modal-backdrop"
      onClick={onClose}
      onDragOver={(e) => e.preventDefault()}
      onDrop={onDrop}
    >
      <div
        className="modal import-modal"
        onClick={(e) => e.stopPropagation()}
      >
        <header>
          <h2>Import entities</h2>
          <button className="icon-btn" onClick={onClose} aria-label="Close">
            ×
          </button>
        </header>

        <div className="settings-tabs">
          <button
            className={`settings-tab${mode === "bulk" ? " active" : ""}`}
            onClick={() => setMode("bulk")}
          >
            Bulk
          </button>
          <button
            className={`settings-tab${mode === "csv" ? " active" : ""}`}
            onClick={() => setMode("csv")}
          >
            CSV
          </button>
          <button
            className={`settings-tab${mode === "graphml" ? " active" : ""}`}
            onClick={() => setMode("graphml")}
            title="Import a Maltego / Gephi / yEd graph"
          >
            GraphML
          </button>
        </div>

        {mode === "bulk" && (
          <div className="import-bulk">
            <div className="import-row">
              <label>
                Type{" "}
                <select
                  value={bulkType}
                  onChange={(e) => setBulkType(e.currentTarget.value)}
                >
                  {canonicalTypes
                    .filter((t) => t.id !== "Collection")
                    .map((t) => (
                      <option key={t.id} value={t.id}>
                        {t.label}
                      </option>
                    ))}
                </select>
              </label>
              <span className="muted small">
                {bulkLines.length}{" "}
                {bulkLines.length === 1 ? "value" : "values"}
              </span>
            </div>
            <textarea
              className="import-textarea"
              placeholder={`One value per line, e.g.\nexample.com\nbob@example.com\n…`}
              value={bulkText}
              onChange={(e) => setBulkText(e.currentTarget.value)}
              rows={10}
              spellCheck={false}
            />
            <p className="muted small">
              Tip: paste a column from a spreadsheet. Lines starting with{" "}
              <code>#</code> are skipped.
            </p>
          </div>
        )}

        {mode === "graphml" && (
          <div className="import-csv">
            <div className="import-row">
              <button
                className="secret-row-btn"
                onClick={() => {
                  // Reuse the file input for graphml — wider accept list.
                  const input = document.createElement("input");
                  input.type = "file";
                  input.accept = ".graphml,.xml,application/xml,text/xml";
                  input.onchange = async () => {
                    const f = input.files?.[0];
                    if (f) setGraphmlText(await f.text());
                  };
                  input.click();
                }}
              >
                Choose file
              </button>
              <span className="muted small">
                Accepts maltrace's own export, Maltego CSV/GraphML, yEd, Gephi, NetworkX.
              </span>
            </div>
            <textarea
              className="import-textarea"
              placeholder={'<?xml version="1.0" encoding="UTF-8"?>\n<graphml ...>\n  <node ...>'}
              value={graphmlText}
              onChange={(e) => setGraphmlText(e.currentTarget.value)}
              rows={10}
              spellCheck={false}
            />
            <p className="muted small">
              Node <code>type</code> + <code>label</code> data keys are
              honoured. Unknown types fall back to <code>Generic</code>.
              Re-importing is idempotent: existing
              (type, label) pairs aren't duplicated.
            </p>
          </div>
        )}

        {mode === "csv" && (
          <div className="import-csv">
            <div className="import-row">
              <button
                className="secret-row-btn"
                onClick={() => fileInputRef.current?.click()}
              >
                Choose file
              </button>
              <span className="muted small">
                …or drag a CSV onto this modal, or paste below.
              </span>
              <input
                ref={fileInputRef}
                type="file"
                accept=".csv,text/csv"
                style={{ display: "none" }}
                onChange={(e) => {
                  const f = e.currentTarget.files?.[0];
                  if (f) onFile(f);
                }}
              />
            </div>
            <textarea
              className="import-textarea"
              placeholder="entity_type,label&#10;Domain,example.com&#10;EmailAddress,bob@example.com"
              value={csvText}
              onChange={(e) => setCsvText(e.currentTarget.value)}
              rows={10}
              spellCheck={false}
            />
            {csvParsed && (
              <div className="import-preview">
                {csvParsed.error ? (
                  <p className="error-banner">{csvParsed.error}</p>
                ) : (
                  <>
                    <p className="muted small">
                      {csvParsed.data.length} valid row
                      {csvParsed.data.length === 1 ? "" : "s"}.
                    </p>
                    <div className="import-preview-table">
                      <div className="import-preview-row import-preview-header">
                        <span>type</span>
                        <span>label</span>
                      </div>
                      {csvParsed.data.slice(0, 6).map((d, i) => (
                        <div key={i} className="import-preview-row">
                          <code>{d.entity_type}</code>
                          <span>{d.label}</span>
                        </div>
                      ))}
                      {csvParsed.data.length > 6 && (
                        <div className="muted small import-preview-row">
                          …and {csvParsed.data.length - 6} more
                        </div>
                      )}
                    </div>
                  </>
                )}
              </div>
            )}
          </div>
        )}

        {error && <p className="error-banner">{error}</p>}

        <footer className="import-footer">
          {progress && (
            <span className="muted small">
              Importing {progress.done}/{progress.total}…
            </span>
          )}
          <button className="secret-row-btn" onClick={onClose} disabled={busy}>
            Cancel
          </button>
          <button
            className="machine-run-btn"
            onClick={submit}
            disabled={
              busy ||
              (mode === "bulk"
                ? bulkLines.length === 0
                : mode === "graphml"
                  ? graphmlText.trim().length === 0
                  : !csvParsed ||
                    !!csvParsed.error ||
                    csvParsed.data.length === 0)
            }
          >
            Import{" "}
            {mode === "bulk"
              ? bulkLines.length
              : mode === "graphml"
                ? "GraphML"
                : csvParsed?.data.length ?? 0}
          </button>
        </footer>
      </div>
    </div>
  );
}

/// Minimal RFC 4180 CSV parser. Handles quoted fields, escaped double
/// quotes (`""`), CRLF/LF newlines, and ignores trailing blank lines.
/// Throws on unterminated quotes.
function parseCsv(text: string): string[][] {
  const rows: string[][] = [];
  let field = "";
  let row: string[] = [];
  let inQuotes = false;
  let i = 0;
  while (i < text.length) {
    const ch = text[i];
    if (inQuotes) {
      if (ch === '"') {
        if (text[i + 1] === '"') {
          field += '"';
          i += 2;
          continue;
        }
        inQuotes = false;
        i++;
        continue;
      }
      field += ch;
      i++;
      continue;
    }
    if (ch === '"') {
      inQuotes = true;
      i++;
      continue;
    }
    if (ch === ",") {
      row.push(field);
      field = "";
      i++;
      continue;
    }
    if (ch === "\n" || ch === "\r") {
      row.push(field);
      rows.push(row);
      row = [];
      field = "";
      // Eat the second half of \r\n.
      if (ch === "\r" && text[i + 1] === "\n") i += 2;
      else i++;
      continue;
    }
    field += ch;
    i++;
  }
  if (inQuotes) throw new Error("Unterminated quoted field in CSV.");
  // Flush trailing field if any meaningful content.
  if (field.length > 0 || row.length > 0) {
    row.push(field);
    rows.push(row);
  }
  // Drop fully empty trailing rows (artefact of trailing newline).
  while (rows.length > 0 && rows[rows.length - 1].every((c) => c === "")) {
    rows.pop();
  }
  return rows;
}
