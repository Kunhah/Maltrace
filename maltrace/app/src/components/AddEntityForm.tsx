import { useEffect, useMemo, useRef, useState } from "react";
import type { CanonicalType } from "../types";
import { detectCanonicalType } from "../recommendations";

interface Props {
  canonicalTypes: CanonicalType[];
  onAdd: (entityType: string, label: string) => void;
  disabled?: boolean;
}

/// Default to Domain since it's the most common starting entity.
const DEFAULT_TYPE = "Domain";

export function AddEntityForm({ canonicalTypes, onAdd, disabled }: Props) {
  const [entityType, setEntityType] = useState(DEFAULT_TYPE);
  const [label, setLabel] = useState("");
  /// True once the user has touched the type dropdown explicitly — once
  /// they have, we stop overwriting their choice with auto-detection.
  const userPickedType = useRef(false);

  // If the ontology loads after mount and our default isn't in it, snap
  // to whatever first type the backend offers so the dropdown is valid.
  useEffect(() => {
    if (
      canonicalTypes.length > 0 &&
      !canonicalTypes.some((t) => t.id === entityType)
    ) {
      setEntityType(canonicalTypes[0].id);
    }
  }, [canonicalTypes, entityType]);

  /// Detection runs on the current label; we re-run on every keystroke so
  /// the hint stays in sync with what the user is typing.
  const detected = useMemo(() => detectCanonicalType(label), [label]);

  /// Auto-snap the dropdown to the detected type, but only when the user
  /// hasn't manually picked one for this label. Clearing the input resets
  /// `userPickedType` so the next investigation starts fresh.
  useEffect(() => {
    if (userPickedType.current) return;
    if (!detected) return;
    if (!canonicalTypes.some((t) => t.id === detected)) return;
    if (detected !== entityType) setEntityType(detected);
  }, [detected, entityType, canonicalTypes]);

  function submit(e: React.FormEvent) {
    e.preventDefault();
    if (!label.trim()) return;
    onAdd(entityType, label.trim());
    setLabel("");
    userPickedType.current = false;
  }

  // Group dropdown options by kind so the list reads naturally
  // (Identity → Asset → Evidence → Threat → Generic).
  const grouped = canonicalTypes.reduce<Record<string, CanonicalType[]>>(
    (acc, t) => {
      (acc[t.kind] ??= []).push(t);
      return acc;
    },
    {},
  );
  const kindOrder = ["identity", "asset", "evidence", "threat", "generic"];
  const kindLabel: Record<string, string> = {
    identity: "Identity",
    asset: "Asset",
    evidence: "Evidence",
    threat: "Threat",
    generic: "Other",
  };

  const detectedLabel =
    detected && detected !== entityType
      ? canonicalTypes.find((t) => t.id === detected)?.label ?? null
      : null;
  const autoMatchedLabel =
    detected && detected === entityType && !userPickedType.current
      ? canonicalTypes.find((t) => t.id === detected)?.label ?? null
      : null;

  return (
    <form className="add-entity" onSubmit={submit}>
      <select
        value={entityType}
        onChange={(e) => {
          userPickedType.current = true;
          setEntityType(e.currentTarget.value);
        }}
        aria-label="Entity type"
        disabled={canonicalTypes.length === 0}
        title="Entity type (auto-detected from value — change to override)"
      >
        {kindOrder.map((kind) =>
          grouped[kind] ? (
            <optgroup key={kind} label={kindLabel[kind]}>
              {grouped[kind].map((t) => (
                <option key={t.id} value={t.id}>
                  {t.label}
                </option>
              ))}
            </optgroup>
          ) : null,
        )}
      </select>
      <div className="add-entity-value">
        <input
          value={label}
          onChange={(e) => {
            setLabel(e.currentTarget.value);
            // Re-enable auto-detection when the field is cleared so the
            // next investigation starts from a clean slate.
            if (e.currentTarget.value === "") userPickedType.current = false;
          }}
          placeholder="domain, email, IP, URL, hash, name…"
          aria-label="Entity value"
        />
        {autoMatchedLabel && (
          <span
            className="add-entity-detected"
            title={`Detected as ${autoMatchedLabel}. Pick a different type to override.`}
          >
            <span className="add-entity-detected-dot" aria-hidden="true" />
            {autoMatchedLabel}
          </span>
        )}
        {detectedLabel && (
          <button
            type="button"
            className="add-entity-suggest"
            onClick={() => {
              userPickedType.current = false;
              setEntityType(detected!);
            }}
            title={`Use detected type: ${detectedLabel}`}
          >
            Use {detectedLabel}?
          </button>
        )}
      </div>
      <button type="submit" disabled={disabled || !label.trim()}>
        Add
      </button>
    </form>
  );
}
