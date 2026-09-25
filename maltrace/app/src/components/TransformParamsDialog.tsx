import { useCallback, useState } from "react";
import type { ParameterSpec, TransformMeta } from "../types";

interface Props {
  transform: TransformMeta;
  entityLabel: string;
  onCancel: () => void;
  onRun: (params: Record<string, unknown>) => void;
}

/// Modal form rendered before invoking a transform that declares any
/// `parameters` in its metadata. Each parameter becomes one row in the
/// form — string/number/boolean/choice. Submits the collected values
/// back to the parent which calls `runTransformWithParams`.
///
/// Bypassed for transforms with no parameters: the parent calls the
/// regular `runTransform` directly. We never show this dialog to the
/// user when there's nothing to ask.
export function TransformParamsDialog({
  transform,
  entityLabel,
  onCancel,
  onRun,
}: Props) {
  const params = transform.parameters ?? [];

  // Seed with defaults so the form is filled in on first paint.
  const [values, setValues] = useState<Record<string, unknown>>(() => {
    const out: Record<string, unknown> = {};
    for (const p of params) {
      if (p.default !== undefined && p.default !== null) {
        out[p.name] = p.default;
      } else if (p.type === "boolean") {
        out[p.name] = false;
      } else if (p.type === "choice" && (p.choices?.length ?? 0) > 0) {
        out[p.name] = p.choices![0];
      } else if (p.type === "number") {
        out[p.name] = 0;
      } else {
        out[p.name] = "";
      }
    }
    return out;
  });

  const [error, setError] = useState<string | null>(null);

  const setValue = useCallback((name: string, v: unknown) => {
    setValues((cur) => ({ ...cur, [name]: v }));
  }, []);

  const submit = useCallback(() => {
    // Validate required fields. We accept "" for strings only when
    // `required` is explicitly false.
    for (const p of params) {
      const v = values[p.name];
      const required = p.required !== false;
      if (!required) continue;
      if (v == null) {
        setError(`${p.name}: required`);
        return;
      }
      if (typeof v === "string" && v.trim().length === 0) {
        setError(`${p.name}: required`);
        return;
      }
    }
    setError(null);
    onRun(values);
  }, [params, values, onRun]);

  return (
    <div className="modal-overlay" onClick={onCancel}>
      <div
        className="modal-card transform-params-modal"
        onClick={(e) => e.stopPropagation()}
        role="dialog"
        aria-modal="true"
      >
        <header className="modal-head">
          <h2>
            <code>{transform.id}</code>{" "}
            <span className="muted small">on {entityLabel}</span>
          </h2>
          <button className="icon-btn" onClick={onCancel} aria-label="Cancel">
            ×
          </button>
        </header>
        {transform.description && (
          <p className="muted small" style={{ marginTop: -6 }}>
            {transform.description}
          </p>
        )}
        <div className="transform-params-form">
          {params.map((p) => (
            <ParamRow
              key={p.name}
              spec={p}
              value={values[p.name]}
              onChange={(v) => setValue(p.name, v)}
            />
          ))}
        </div>
        {error && (
          <p className="error-banner" role="alert">
            {error}
          </p>
        )}
        <footer className="modal-foot">
          <button onClick={onCancel}>Cancel</button>
          <button className="primary" onClick={submit}>
            Run transform
          </button>
        </footer>
      </div>
    </div>
  );
}

function ParamRow({
  spec,
  value,
  onChange,
}: {
  spec: ParameterSpec;
  value: unknown;
  onChange: (v: unknown) => void;
}) {
  const required = spec.required !== false;
  const label = (
    <label htmlFor={`tp-${spec.name}`} className="transform-params-label">
      <span>
        {spec.name}
        {required && <span className="required-mark">*</span>}
      </span>
      {spec.description && (
        <span className="muted small">{spec.description}</span>
      )}
    </label>
  );
  let input: React.ReactNode;
  switch (spec.type) {
    case "boolean":
      input = (
        <input
          id={`tp-${spec.name}`}
          type="checkbox"
          checked={!!value}
          onChange={(e) => onChange(e.currentTarget.checked)}
        />
      );
      break;
    case "number":
      input = (
        <input
          id={`tp-${spec.name}`}
          type="number"
          value={typeof value === "number" ? value : ""}
          onChange={(e) => {
            const n = e.currentTarget.value === "" ? null : Number(e.currentTarget.value);
            onChange(n);
          }}
        />
      );
      break;
    case "choice":
      input = (
        <select
          id={`tp-${spec.name}`}
          value={typeof value === "string" ? value : ""}
          onChange={(e) => onChange(e.currentTarget.value)}
        >
          {(spec.choices ?? []).map((c) => (
            <option key={c} value={c}>
              {c}
            </option>
          ))}
        </select>
      );
      break;
    case "string":
    default:
      input = (
        <input
          id={`tp-${spec.name}`}
          type="text"
          value={typeof value === "string" ? value : ""}
          onChange={(e) => onChange(e.currentTarget.value)}
        />
      );
  }
  return (
    <div className="transform-params-row">
      {label}
      {input}
    </div>
  );
}
