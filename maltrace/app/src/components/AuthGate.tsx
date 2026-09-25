import { useCallback, useEffect, useState } from "react";
import { api, runtime } from "../api";
import type { AuthStatus } from "../types";

interface Props {
  children: React.ReactNode;
}

/// Top-level gate: probes `/api/auth/whoami` on first render. If the
/// server says auth is required and we don't have a working token,
/// renders a small login card instead of the app. Once login succeeds,
/// the gate flips and the real `<App>` is mounted.
///
/// Tauri builds short-circuit through (no HTTP server). The auth model
/// is intentionally simple: a bearer token; the operator hands the
/// analyst a token they paste into the field. No password rotation
/// flow — the operator just edits `users.json` and tells the analyst
/// the new value.
export function AuthGate({ children }: Props) {
  const [status, setStatus] = useState<AuthStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [token, setToken] = useState("");
  const [busy, setBusy] = useState(false);

  // Probe once on mount. Tauri returns synthetic bypass so this is a
  // single-frame round-trip there.
  useEffect(() => {
    let cancelled = false;
    api
      .authWhoami()
      .then((s) => {
        if (!cancelled) setStatus(s);
      })
      .catch((e) => {
        if (cancelled) return;
        // 401 means "auth required, no current token" — render the
        // login form rather than the app.
        const message = String(e);
        if (message.includes("401") || message.includes("missing Bearer")) {
          setStatus({
            bypass: false,
            authenticated: false,
            name: null,
            role: null,
          });
        } else {
          setError(message);
        }
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const onSubmit = useCallback(
    async (e: React.FormEvent) => {
      e.preventDefault();
      const t = token.trim();
      if (!t) return;
      setBusy(true);
      setError(null);
      api.setAuthToken(t);
      try {
        const next = await api.authLogin(t);
        setStatus(next);
      } catch (err) {
        api.setAuthToken(null);
        setError(String(err));
      } finally {
        setBusy(false);
      }
    },
    [token],
  );

  if (!status) {
    if (error) {
      return (
        <div className="auth-gate">
          <div className="auth-card">
            <h1>maltrace</h1>
            <p className="error-banner">{error}</p>
          </div>
        </div>
      );
    }
    return null; // Single-frame; render nothing rather than a flash.
  }

  // Bypass or authenticated → render the actual app.
  if (status.bypass || status.authenticated) {
    return <>{children}</>;
  }

  return (
    <div className="auth-gate">
      <form className="auth-card" onSubmit={onSubmit}>
        <h1>maltrace</h1>
        <p className="muted small">
          This server requires authentication. Paste your access token
          to continue. {runtime.isTauri ? "" : "(Web build)"}
        </p>
        <input
          type="password"
          autoFocus
          placeholder="Bearer token"
          value={token}
          onChange={(e) => setToken(e.currentTarget.value)}
        />
        {error && (
          <p className="error-banner" role="alert">
            {error}
          </p>
        )}
        <button type="submit" className="primary" disabled={busy || !token.trim()}>
          {busy ? "Verifying…" : "Sign in"}
        </button>
        <p className="muted small auth-hint">
          Tokens are issued by the server admin in <code>users.json</code>.
        </p>
      </form>
    </div>
  );
}
