import { useCallback, useEffect, useMemo, useState } from "react";
import { api } from "../api";
import type {
  CanonicalKind,
  CanonicalType,
  Limits,
  ProxyConfig,
  RateLimitConfig,
  SecretInfo,
} from "../types";

interface Props {
  onClose: () => void;
  /// Canonical-types snapshot from App, so the Entity-types tab can show
  /// existing custom types and flag collisions before the backend rejects.
  canonicalTypes: CanonicalType[];
  /// Fired after a successful create/delete so App can reload the types
  /// list and refresh the AddEntityForm + graph stylesheet.
  onCanonicalTypesChanged: () => void;
}

type SettingsTab = "secrets" | "types" | "proxies" | "ratelimits" | "webhooks";

/// Curated list of common SpiderFoot module API-key option names.
const PRESET_KEY_NAMES: Array<{ name: string; hint: string }> = [
  { name: "api_key_shodan", hint: "Shodan — sfp_shodan" },
  { name: "api_key_securitytrails", hint: "SecurityTrails" },
  { name: "api_key_hibp", hint: "Have I Been Pwned — sfp_haveibeenpwned" },
  { name: "api_key_virustotal", hint: "VirusTotal — sfp_virustotal" },
  { name: "api_key_censys", hint: "Censys" },
  { name: "api_key_urlscan", hint: "urlscan.io — sfp_urlscan" },
  { name: "api_key_intelx", hint: "Intelligence X — sfp_intelx" },
  { name: "api_key_hunter", hint: "Hunter.io — sfp_hunter" },
  { name: "api_key_emailrep", hint: "EmailRep — sfp_emailrep" },
  { name: "api_key_abuseipdb", hint: "AbuseIPDB — sfp_abuseipdb" },
  { name: "api_key_greynoise", hint: "GreyNoise — sfp_greynoise" },
  { name: "api_key_ipinfo", hint: "ipinfo.io — sfp_ipinfo" },
];

interface DraftEntry {
  name: string;
  value: string;
}

/// Multi-tab settings modal. "API keys" stores per-sidecar credentials;
/// "Entity types" lets the user add user-defined canonical types that
/// show up in the AddEntityForm and graph stylesheet.
export function SettingsPanel({
  onClose,
  canonicalTypes,
  onCanonicalTypesChanged,
}: Props) {
  const [tab, setTab] = useState<SettingsTab>("secrets");

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
        className="modal settings-modal"
        onClick={(e) => e.stopPropagation()}
      >
        <header>
          <h2>Settings</h2>
          <button className="icon-btn" onClick={onClose} aria-label="Close">
            ×
          </button>
        </header>

        <div className="settings-tabs">
          <button
            className={`settings-tab${tab === "secrets" ? " active" : ""}`}
            onClick={() => setTab("secrets")}
          >
            API keys
          </button>
          <button
            className={`settings-tab${tab === "types" ? " active" : ""}`}
            onClick={() => setTab("types")}
          >
            Entity types
          </button>
          <button
            className={`settings-tab${tab === "proxies" ? " active" : ""}`}
            onClick={() => setTab("proxies")}
          >
            Proxies
          </button>
          <button
            className={`settings-tab${tab === "ratelimits" ? " active" : ""}`}
            onClick={() => setTab("ratelimits")}
          >
            Rate limits
          </button>
          <button
            className={`settings-tab${tab === "webhooks" ? " active" : ""}`}
            onClick={() => setTab("webhooks")}
            title="Outbound webhook URLs (Slack/Discord/generic JSON)"
          >
            Webhooks
          </button>
        </div>

        {tab === "secrets" ? (
          <SecretsTab />
        ) : tab === "proxies" ? (
          <ProxiesTab />
        ) : tab === "ratelimits" ? (
          <RateLimitsTab />
        ) : tab === "webhooks" ? (
          <WebhooksTab />
        ) : (
          <EntityTypesTab
            canonicalTypes={canonicalTypes}
            onChanged={onCanonicalTypesChanged}
          />
        )}
      </div>
    </div>
  );
}

// ─── Secrets tab ──────────────────────────────────────────────────

function SecretsTab() {
  const [secrets, setSecrets] = useState<SecretInfo[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [draft, setDraft] = useState<DraftEntry>({ name: "", value: "" });
  const [showValues, setShowValues] = useState(false);
  const [editing, setEditing] = useState<Record<string, string>>({});

  const refresh = useCallback(() => {
    api
      .listSecretNames()
      .then(setSecrets)
      .catch((e) => setError(String(e)));
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  const onAdd = useCallback(async () => {
    const name = draft.name.trim();
    const value = draft.value;
    if (!name || !value) return;
    try {
      await api.setSecret(name, value);
      setDraft({ name: "", value: "" });
      refresh();
    } catch (e) {
      setError(String(e));
    }
  }, [draft, refresh]);

  const onSaveEdit = useCallback(
    async (name: string) => {
      const value = editing[name];
      if (value === undefined) return;
      try {
        await api.setSecret(name, value);
        setEditing((cur) => {
          const next = { ...cur };
          delete next[name];
          return next;
        });
        refresh();
      } catch (e) {
        setError(String(e));
      }
    },
    [editing, refresh],
  );

  const onDelete = useCallback(
    async (name: string) => {
      if (!window.confirm(`Delete secret "${name}"?`)) return;
      try {
        await api.deleteSecret(name);
        refresh();
      } catch (e) {
        setError(String(e));
      }
    },
    [refresh],
  );

  const rows = useMemo(() => {
    const storedNames = new Set((secrets ?? []).map((s) => s.name));
    const preset = PRESET_KEY_NAMES.filter((p) => !storedNames.has(p.name));
    return { stored: secrets ?? [], preset };
  }, [secrets]);

  return (
    <>
      <p className="muted small">
        Stored in <code>secrets.json</code> in your app data directory
        (mode 0600 on Unix). Passed as <code>--opts</code> JSON to every
        sidecar run; each module picks the keys it knows about.
      </p>

      {error && (
        <p className="error-banner" role="alert">
          {error}
        </p>
      )}

      <section className="settings-section">
        <header className="settings-section-head">
          <h3>Add a secret</h3>
          <label className="show-values-toggle">
            <input
              type="checkbox"
              checked={showValues}
              onChange={(e) => setShowValues(e.currentTarget.checked)}
            />
            <span>Show values</span>
          </label>
        </header>
        <div className="secret-add">
          <input
            className="secret-input"
            placeholder="name (e.g. api_key_shodan)"
            value={draft.name}
            list="preset-secret-names"
            onChange={(e) =>
              setDraft((d) => ({ ...d, name: e.currentTarget.value }))
            }
          />
          <datalist id="preset-secret-names">
            {PRESET_KEY_NAMES.map((p) => (
              <option key={p.name} value={p.name}>
                {p.hint}
              </option>
            ))}
          </datalist>
          <input
            className="secret-input"
            placeholder="value"
            type={showValues ? "text" : "password"}
            value={draft.value}
            onChange={(e) =>
              setDraft((d) => ({ ...d, value: e.currentTarget.value }))
            }
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                onAdd();
              }
            }}
          />
          <button
            className="secret-add-btn"
            disabled={!draft.name.trim() || !draft.value}
            onClick={onAdd}
          >
            Add
          </button>
        </div>
      </section>

      <section className="settings-section">
        <h3>Stored ({rows.stored.length})</h3>
        {rows.stored.length === 0 ? (
          <p className="empty">No secrets yet.</p>
        ) : (
          <ul className="secret-list">
            {rows.stored.map((s) => {
              const isEditing = editing[s.name] !== undefined;
              return (
                <li key={s.name} className="secret-row">
                  <code className="secret-name">{s.name}</code>
                  {isEditing ? (
                    <>
                      <input
                        className="secret-input"
                        type={showValues ? "text" : "password"}
                        autoFocus
                        value={editing[s.name]}
                        onChange={(e) =>
                          setEditing((cur) => ({
                            ...cur,
                            [s.name]: e.currentTarget.value,
                          }))
                        }
                        onKeyDown={(e) => {
                          if (e.key === "Enter") {
                            e.preventDefault();
                            onSaveEdit(s.name);
                          } else if (e.key === "Escape") {
                            e.preventDefault();
                            setEditing((cur) => {
                              const next = { ...cur };
                              delete next[s.name];
                              return next;
                            });
                          }
                        }}
                      />
                      <button
                        className="secret-row-btn"
                        onClick={() => onSaveEdit(s.name)}
                      >
                        Save
                      </button>
                    </>
                  ) : (
                    <>
                      <span className="muted small set-tag">set</span>
                      <button
                        className="secret-row-btn"
                        onClick={() =>
                          setEditing((cur) => ({ ...cur, [s.name]: "" }))
                        }
                      >
                        Edit
                      </button>
                    </>
                  )}
                  <button
                    className="secret-row-btn danger"
                    onClick={() => onDelete(s.name)}
                  >
                    Delete
                  </button>
                </li>
              );
            })}
          </ul>
        )}
      </section>

      {rows.preset.length > 0 && (
        <section className="settings-section">
          <h3>Suggested</h3>
          <ul className="secret-list">
            {rows.preset.slice(0, 8).map((p) => (
              <li key={p.name} className="secret-row preset">
                <code className="secret-name">{p.name}</code>
                <span className="muted small">{p.hint}</span>
                <button
                  className="secret-row-btn"
                  onClick={() => setDraft({ name: p.name, value: "" })}
                >
                  Use
                </button>
              </li>
            ))}
          </ul>
        </section>
      )}
    </>
  );
}

// ─── Entity types tab ────────────────────────────────────────────

const KIND_OPTIONS: Array<{ id: CanonicalKind; label: string }> = [
  { id: "identity", label: "Identity" },
  { id: "asset", label: "Asset" },
  { id: "evidence", label: "Evidence" },
  { id: "threat", label: "Threat" },
  { id: "generic", label: "Other" },
];

const COLOR_SUGGESTIONS = [
  "#f472b6", "#a855f7", "#22c55e", "#f59e0b", "#0ea5e9", "#fb7185",
  "#34d399", "#fde047", "#94a3b8", "#fb923c", "#ef4444",
];

function EntityTypesTab({
  canonicalTypes,
  onChanged,
}: {
  canonicalTypes: CanonicalType[];
  onChanged: () => void;
}) {
  const [error, setError] = useState<string | null>(null);
  const [label, setLabel] = useState("");
  const [kind, setKind] = useState<CanonicalKind>("identity");
  const [color, setColor] = useState("#f472b6");

  const builtIns = useMemo(
    () => canonicalTypes.filter((t) => !t.custom),
    [canonicalTypes],
  );
  const customs = useMemo(
    () => canonicalTypes.filter((t) => t.custom),
    [canonicalTypes],
  );

  const onCreate = useCallback(async () => {
    setError(null);
    try {
      await api.createCustomEntityType(label.trim(), kind, color);
      setLabel("");
      onChanged();
    } catch (e) {
      setError(String(e));
    }
  }, [label, kind, color, onChanged]);

  const onDelete = useCallback(
    async (id: string) => {
      if (!window.confirm(`Delete entity type "${id}"?\n\nExisting entities of this type stay on the graph.`)) return;
      try {
        await api.deleteCustomEntityType(id);
        onChanged();
      } catch (e) {
        setError(String(e));
      }
    },
    [onChanged],
  );

  return (
    <>
      <p className="muted small">
        Custom types appear in the entity dropdown and get their own
        node colour. They have no SpiderFoot mapping — transforms targeting
        them won't return anything until a plugin wires one up.
      </p>

      {error && (
        <p className="error-banner" role="alert">
          {error}
        </p>
      )}

      <section className="settings-section">
        <h3>Add a type</h3>
        <div className="type-add">
          <input
            className="secret-input"
            placeholder="label (e.g. Wallet, Vehicle, Aircraft)"
            value={label}
            onChange={(e) => setLabel(e.currentTarget.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && label.trim()) onCreate();
            }}
          />
          <select
            className="secret-input"
            value={kind}
            onChange={(e) => setKind(e.currentTarget.value as CanonicalKind)}
            aria-label="Kind"
          >
            {KIND_OPTIONS.map((k) => (
              <option key={k.id} value={k.id}>
                {k.label}
              </option>
            ))}
          </select>
          <div className="type-color-row">
            <input
              type="color"
              value={color}
              onChange={(e) => setColor(e.currentTarget.value)}
              aria-label="Color"
            />
            <div className="type-color-swatches">
              {COLOR_SUGGESTIONS.map((c) => (
                <button
                  key={c}
                  className={`type-color-swatch${c === color ? " active" : ""}`}
                  style={{ background: c }}
                  onClick={() => setColor(c)}
                  title={c}
                  aria-label={`Use color ${c}`}
                />
              ))}
            </div>
          </div>
          <button
            className="secret-add-btn"
            disabled={!label.trim()}
            onClick={onCreate}
          >
            Add
          </button>
        </div>
      </section>

      <section className="settings-section">
        <h3>Custom ({customs.length})</h3>
        {customs.length === 0 ? (
          <p className="empty">No custom types yet.</p>
        ) : (
          <ul className="type-list">
            {customs.map((t) => (
              <li key={t.id} className="type-row">
                <span
                  className="type-dot"
                  style={{ background: t.color }}
                  aria-hidden="true"
                />
                <span className="type-row-label">{t.label}</span>
                <code className="muted small">{t.id}</code>
                <span className="muted small">{t.kind}</span>
                <button
                  className="secret-row-btn danger"
                  onClick={() => onDelete(t.id)}
                >
                  Delete
                </button>
              </li>
            ))}
          </ul>
        )}
      </section>

      <section className="settings-section">
        <h3>Built-in ({builtIns.length})</h3>
        <div className="type-built-in-mix">
          {builtIns.map((t) => (
            <span
              key={t.id}
              className="type-built-in-chip"
              style={
                {
                  "--chip-color": t.color,
                } as React.CSSProperties
              }
              title={`${t.kind} · ${t.id}`}
            >
              {t.label}
            </span>
          ))}
        </div>
      </section>
    </>
  );
}

// ─── Proxies tab ─────────────────────────────────────────────────
function ProxiesTab() {
  const [cfg, setCfg] = useState<ProxyConfig | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  useEffect(() => {
    api
      .getProxyConfig()
      .then(setCfg)
      .catch((e) => setError(String(e)));
  }, []);

  const save = useCallback(async () => {
    if (!cfg) return;
    try {
      await api.setProxyConfig(cfg);
      setSaved(true);
      setError(null);
      window.setTimeout(() => setSaved(false), 1500);
    } catch (e) {
      setError(String(e));
    }
  }, [cfg]);

  if (!cfg) {
    return (
      <section className="settings-body">
        {error ? <p className="error-banner">{error}</p> : <p className="muted">Loading…</p>}
      </section>
    );
  }

  const entries = Object.entries(cfg.per_sidecar);

  return (
    <section className="settings-body">
      <p className="muted small">
        Routes every transform's outbound HTTP through a proxy. Tor:
        <code> socks5://127.0.0.1:9050</code>. Per-sidecar overrides
        win over the default. Set a sidecar's override to an empty
        string to disable the default for it.
      </p>
      <div className="settings-row">
        <label>Default proxy</label>
        <input
          type="text"
          placeholder="socks5://127.0.0.1:9050 or http://corp-proxy:8080"
          value={cfg.default_proxy}
          onChange={(e) =>
            setCfg({ ...cfg, default_proxy: e.currentTarget.value })
          }
        />
      </div>
      <h4 className="settings-subhead">Per-sidecar overrides</h4>
      {entries.length === 0 && (
        <p className="muted small">No overrides yet.</p>
      )}
      {entries.map(([sid, url]) => (
        <div key={sid} className="settings-row settings-row-pair">
          <input
            type="text"
            value={sid}
            onChange={(e) => {
              const next = { ...cfg.per_sidecar };
              const newKey = e.currentTarget.value;
              delete next[sid];
              next[newKey] = url;
              setCfg({ ...cfg, per_sidecar: next });
            }}
            placeholder="sidecar id (spiderfoot, face-search, …)"
          />
          <input
            type="text"
            value={url}
            placeholder="proxy URL (empty disables default)"
            onChange={(e) => {
              const next = { ...cfg.per_sidecar };
              next[sid] = e.currentTarget.value;
              setCfg({ ...cfg, per_sidecar: next });
            }}
          />
          <button
            className="icon-btn danger"
            onClick={() => {
              const next = { ...cfg.per_sidecar };
              delete next[sid];
              setCfg({ ...cfg, per_sidecar: next });
            }}
            title="Remove override"
          >
            ×
          </button>
        </div>
      ))}
      <button
        className="settings-add"
        onClick={() => {
          const id = `sidecar-${entries.length + 1}`;
          setCfg({ ...cfg, per_sidecar: { ...cfg.per_sidecar, [id]: "" } });
        }}
      >
        + Add override
      </button>
      {error && <p className="error-banner">{error}</p>}
      <footer className="settings-foot">
        <button className="primary" onClick={save}>
          {saved ? "Saved ✓" : "Save"}
        </button>
      </footer>
    </section>
  );
}

// ─── Rate limits tab ─────────────────────────────────────────────
function RateLimitsTab() {
  const [cfg, setCfg] = useState<RateLimitConfig | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  useEffect(() => {
    api
      .getRateLimitConfig()
      .then(setCfg)
      .catch((e) => setError(String(e)));
  }, []);

  const save = useCallback(async () => {
    if (!cfg) return;
    try {
      await api.setRateLimitConfig(cfg);
      setSaved(true);
      setError(null);
      window.setTimeout(() => setSaved(false), 1500);
    } catch (e) {
      setError(String(e));
    }
  }, [cfg]);

  if (!cfg) {
    return (
      <section className="settings-body">
        {error ? <p className="error-banner">{error}</p> : <p className="muted">Loading…</p>}
      </section>
    );
  }

  const def: Limits = cfg.default ?? { max_concurrent: 4, min_interval_ms: 200 };
  const entries = Object.entries(cfg.per_sidecar);

  return (
    <section className="settings-body">
      <p className="muted small">
        Caps how many transforms one sidecar can run at once
        (<code>max_concurrent</code>) and the minimum wall-clock between
        consecutive calls (<code>min_interval_ms</code>). Keyed sidecars
        (Shodan, HIBP, VT) want tighter limits than free ones.
      </p>
      <h4 className="settings-subhead">Default</h4>
      <LimitsRow
        value={def}
        onChange={(next) => setCfg({ ...cfg, default: next })}
      />
      <h4 className="settings-subhead">Per-sidecar overrides</h4>
      {entries.length === 0 && (
        <p className="muted small">No overrides yet.</p>
      )}
      {entries.map(([sid, limits]) => (
        <div key={sid} className="settings-row settings-row-limit">
          <input
            type="text"
            value={sid}
            onChange={(e) => {
              const next = { ...cfg.per_sidecar };
              const newKey = e.currentTarget.value;
              delete next[sid];
              next[newKey] = limits;
              setCfg({ ...cfg, per_sidecar: next });
            }}
            placeholder="sidecar id"
          />
          <LimitsRow
            value={limits}
            onChange={(next) => {
              setCfg({
                ...cfg,
                per_sidecar: { ...cfg.per_sidecar, [sid]: next },
              });
            }}
          />
          <button
            className="icon-btn danger"
            onClick={() => {
              const next = { ...cfg.per_sidecar };
              delete next[sid];
              setCfg({ ...cfg, per_sidecar: next });
            }}
          >
            ×
          </button>
        </div>
      ))}
      <button
        className="settings-add"
        onClick={() => {
          const id = `sidecar-${entries.length + 1}`;
          setCfg({
            ...cfg,
            per_sidecar: {
              ...cfg.per_sidecar,
              [id]: { max_concurrent: 2, min_interval_ms: 500 },
            },
          });
        }}
      >
        + Add override
      </button>
      {error && <p className="error-banner">{error}</p>}
      <footer className="settings-foot">
        <button className="primary" onClick={save}>
          {saved ? "Saved ✓" : "Save"}
        </button>
      </footer>
    </section>
  );
}

function LimitsRow({
  value,
  onChange,
}: {
  value: Limits;
  onChange: (next: Limits) => void;
}) {
  return (
    <div className="limits-row">
      <label>
        <span className="muted small">max concurrent</span>
        <input
          type="number"
          min={1}
          max={64}
          value={value.max_concurrent}
          onChange={(e) =>
            onChange({
              ...value,
              max_concurrent: Math.max(1, Number(e.currentTarget.value)),
            })
          }
        />
      </label>
      <label>
        <span className="muted small">min interval (ms)</span>
        <input
          type="number"
          min={0}
          max={60000}
          value={value.min_interval_ms}
          onChange={(e) =>
            onChange({
              ...value,
              min_interval_ms: Math.max(0, Number(e.currentTarget.value)),
            })
          }
        />
      </label>
    </div>
  );
}

// ─── Webhooks tab ────────────────────────────────────────────────
function WebhooksTab() {
  const [cfg, setCfg] = useState<import("../types").WebhookConfig | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);
  const [testMsg, setTestMsg] = useState<string | null>(null);

  useEffect(() => {
    api
      .getWebhookConfig()
      .then(setCfg)
      .catch((e) => setError(String(e)));
  }, []);

  const save = useCallback(async () => {
    if (!cfg) return;
    try {
      const sanitized: import("../types").WebhookConfig = {
        urls: cfg.urls.map((u) => u.trim()).filter((u) => u.length > 0),
      };
      await api.setWebhookConfig(sanitized);
      setCfg(sanitized);
      setSaved(true);
      setError(null);
      window.setTimeout(() => setSaved(false), 1500);
    } catch (e) {
      setError(String(e));
    }
  }, [cfg]);

  const testFire = useCallback(async () => {
    setTestMsg(null);
    try {
      const n = await api.testWebhooks();
      setTestMsg(`Fired ${n} ${n === 1 ? "webhook" : "webhooks"} — check your channel.`);
    } catch (e) {
      setTestMsg(`Test failed: ${e}`);
    }
  }, []);

  if (!cfg) {
    return (
      <section className="settings-body">
        {error ? <p className="error-banner">{error}</p> : <p className="muted">Loading…</p>}
      </section>
    );
  }

  return (
    <section className="settings-body">
      <p className="muted small">
        Outbound webhook URLs. Every entry receives a JSON{" "}
        <code>{`{"text": "..."}`}</code> POST on:
      </p>
      <ul className="muted small">
        <li>Scheduled-machine run that finds new entities</li>
        <li>Scheduled-machine run that errors</li>
      </ul>
      <p className="muted small">
        Slack and Discord incoming webhooks both accept this shape unchanged.
      </p>
      {cfg.urls.map((u, i) => (
        <div key={i} className="settings-row settings-row-pair">
          <input
            type="text"
            value={u}
            placeholder="https://hooks.slack.com/services/…"
            onChange={(e) => {
              const next = [...cfg.urls];
              next[i] = e.currentTarget.value;
              setCfg({ urls: next });
            }}
          />
          <button
            className="icon-btn danger"
            onClick={() => {
              const next = cfg.urls.filter((_, j) => j !== i);
              setCfg({ urls: next });
            }}
            title="Remove webhook"
          >
            ×
          </button>
        </div>
      ))}
      <button
        className="settings-add"
        onClick={() => setCfg({ urls: [...cfg.urls, ""] })}
      >
        + Add webhook URL
      </button>
      {error && <p className="error-banner">{error}</p>}
      {testMsg && <p className="muted small">{testMsg}</p>}
      <footer className="settings-foot" style={{ display: "flex", gap: 8, justifyContent: "flex-end" }}>
        <button onClick={testFire} disabled={cfg.urls.length === 0}>
          Fire test
        </button>
        <button className="primary" onClick={save}>
          {saved ? "Saved ✓" : "Save"}
        </button>
      </footer>
    </section>
  );
}
