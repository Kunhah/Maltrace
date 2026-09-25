import { useCallback, useEffect, useState } from "react";
import { api, runtime } from "../api";
import type { PluginSummary } from "../types";

interface Props {
  onClose: () => void;
}

/// Modal listing every loaded plugin. Surfaces the plugin SDK so users can
/// see what's installed, where it lives, and whether the runner is healthy.
export function PluginsPopover({ onClose }: Props) {
  const [plugins, setPlugins] = useState<PluginSummary[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [refreshKey, setRefreshKey] = useState(0);
  const [installBusy, setInstallBusy] = useState(false);
  const [installMessage, setInstallMessage] = useState<string | null>(null);
  const [dirInput, setDirInput] = useState("");

  useEffect(() => {
    let cancelled = false;
    api
      .listPlugins()
      .then((p) => {
        if (!cancelled) setPlugins(p);
      })
      .catch((e) => {
        if (!cancelled) setError(String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [refreshKey]);

  const reload = useCallback(() => setRefreshKey((k) => k + 1), []);

  const installFromDir = useCallback(async () => {
    const dir = dirInput.trim();
    if (!dir) return;
    setInstallBusy(true);
    setInstallMessage(null);
    try {
      const res = await api.installPluginFromDirectory(dir);
      setDirInput("");
      setInstallMessage(
        `Installed ${res.plugin_id} at ${res.plugin_dir}${
          res.needs_venv ? " — click Bootstrap venv to set up Python deps." : "."
        }`,
      );
      reload();
    } catch (e) {
      setInstallMessage(`Install failed: ${e}`);
    } finally {
      setInstallBusy(false);
    }
  }, [dirInput, reload]);

  const installFromFile = useCallback(
    async (file: File) => {
      setInstallBusy(true);
      setInstallMessage(null);
      try {
        const dataUrl = await new Promise<string>((resolve, reject) => {
          const r = new FileReader();
          r.onload = () => resolve(String(r.result ?? ""));
          r.onerror = () => reject(r.error ?? new Error("read failed"));
          r.readAsDataURL(file);
        });
        const res = await api.installPluginFromTarball(dataUrl);
        setInstallMessage(
          `Installed ${res.plugin_id} from ${file.name}${
            res.needs_venv ? " — click Bootstrap venv to set up Python deps." : "."
          }`,
        );
        reload();
      } catch (e) {
        setInstallMessage(`Install failed: ${e}`);
      } finally {
        setInstallBusy(false);
      }
    },
    [reload],
  );

  const uninstall = useCallback(
    async (id: string) => {
      if (!window.confirm(`Uninstall plugin "${id}"?`)) return;
      try {
        await api.uninstallPlugin(id);
        reload();
      } catch (e) {
        setInstallMessage(`Uninstall failed: ${e}`);
      }
    },
    [reload],
  );

  const bootstrap = useCallback(
    async (id: string) => {
      setInstallBusy(true);
      setInstallMessage(`Bootstrapping ${id}…`);
      try {
        const res = await api.bootstrapPluginVenv(id);
        setInstallMessage(
          res.status === "ok"
            ? `Bootstrapped ${id}.`
            : `Bootstrap of ${id} failed:\n${res.stderr_excerpt || res.stdout_excerpt}`,
        );
        reload();
      } catch (e) {
        setInstallMessage(`Bootstrap failed: ${e}`);
      } finally {
        setInstallBusy(false);
      }
    },
    [reload],
  );

  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") onClose();
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  return (
    <div
      className="modal-backdrop"
      onClick={onClose}
      role="dialog"
      aria-label="Loaded plugins"
    >
      <div
        className="modal plugins-modal"
        onClick={(e) => e.stopPropagation()}
      >
        <header>
          <h2>Plugins</h2>
          <button className="icon-btn" onClick={onClose} aria-label="Close">
            ×
          </button>
        </header>

        <section className="plugin-install">
          <h3>Install a plugin</h3>
          {runtime.isTauri ? (
            <div className="bulk-row">
              <input
                type="text"
                placeholder="/absolute/path/to/plugin/directory"
                value={dirInput}
                onChange={(e) => setDirInput(e.currentTarget.value)}
                disabled={installBusy}
              />
              <button
                disabled={installBusy || !dirInput.trim()}
                onClick={installFromDir}
              >
                Install from directory
              </button>
            </div>
          ) : (
            <p className="muted small">
              Pick a <code>.tar.gz</code> bundle containing
              <code> plugin.toml</code> at its root (or wrapped in one
              top-level directory).
            </p>
          )}
          <div className="bulk-row">
            <input
              type="file"
              accept=".tar.gz,.tgz,application/gzip"
              disabled={installBusy}
              onChange={(e) => {
                const f = e.currentTarget.files?.[0];
                if (f) void installFromFile(f);
              }}
            />
          </div>
          {installMessage && (
            <pre className="plugin-install-message">{installMessage}</pre>
          )}
        </section>

        {error && <div className="ai-msg-error">Error loading plugins: {error}</div>}

        {!plugins && !error && (
          <div className="muted">Loading…</div>
        )}

        {plugins && plugins.length === 0 && (
          <div className="muted">
            No plugins loaded. See <code>docs/PLUGIN_SDK.md</code> for how to
            install one.
          </div>
        )}

        {plugins && plugins.length > 0 && (
          <ul className="plugin-list">
            {plugins.map((p) => (
              <li key={p.id} className={p.runnable ? "" : "is-unrunnable"}>
                <div className="plugin-list-header">
                  <span className="plugin-name">{p.name}</span>
                  <span className="muted small">v{p.version}</span>
                  {p.license && (
                    <span className="small-tag" title={`License: ${p.license}`}>
                      {p.license}
                    </span>
                  )}
                  {p.transform_count != null && (
                    <span className="small-tag plugin-tcount">
                      {p.transform_count} transforms
                    </span>
                  )}
                  {!p.runnable && (
                    <span
                      className="small-tag plugin-warn"
                      title="The plugin's runner command was not found on disk. Usually means the venv hasn't been set up."
                    >
                      not runnable
                    </span>
                  )}
                </div>
                {p.description && (
                  <div className="plugin-desc">{p.description}</div>
                )}
                <div className="plugin-meta">
                  <code className="plugin-id">{p.id}</code>
                  <span className="muted plugin-path" title={p.plugin_dir}>
                    {p.plugin_dir}
                  </span>
                  {p.homepage && (
                    <a
                      href={p.homepage}
                      target="_blank"
                      rel="noopener noreferrer"
                      className="plugin-homepage"
                      onClick={(e) => {
                        e.preventDefault();
                        api.openUrl(p.homepage!).catch(() => {});
                      }}
                    >
                      homepage ↗
                    </a>
                  )}
                </div>
                <div className="plugin-actions">
                  {!p.runnable && (
                    <button
                      className="plugin-action"
                      disabled={installBusy}
                      onClick={() => bootstrap(p.id)}
                      title="Create a Python venv and pip install requirements.txt"
                    >
                      Bootstrap venv
                    </button>
                  )}
                  {/* Only allow uninstall on user-installed plugins.
                      In-tree plugins live under the repo and would just
                      reappear on the next launch. */}
                  {isUserInstalled(p.plugin_dir) && (
                    <button
                      className="plugin-action danger"
                      disabled={installBusy}
                      onClick={() => uninstall(p.id)}
                    >
                      Uninstall
                    </button>
                  )}
                </div>
              </li>
            ))}
          </ul>
        )}

        <footer>
          See{" "}
          <code>docs/PLUGIN_SDK.md</code> for the plugin SDK reference.
        </footer>
      </div>
    </div>
  );
}

/// Heuristic — user-installed plugins live under the OS data dir
/// (`.../dev.maltrace.app/plugins/...`); in-tree plugins live in the
/// maltrace checkout. We can't safely uninstall the latter (they
/// reappear on next launch).
function isUserInstalled(pluginDir: string): boolean {
  return pluginDir.includes("dev.maltrace.app") && pluginDir.includes("plugins");
}
