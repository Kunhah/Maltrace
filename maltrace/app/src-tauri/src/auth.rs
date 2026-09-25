//! Token-based authentication for the standalone HTTP server.
//!
//! The Tauri desktop build is single-user and skips auth entirely (the
//! middleware is only mounted on the axum router). The server build
//! reads a `users.json` file from the app data directory:
//!
//! ```json
//! { "users": [
//!     { "name": "alice",   "token": "...", "role": "analyst" },
//!     { "name": "readonly","token": "...", "role": "reader" }
//! ] }
//! ```
//!
//! `Authorization: Bearer <token>` requests are looked up against this
//! file. Unknown tokens return 401; missing tokens on protected routes
//! return 401 too. The reader role is allowed only GET/HEAD; everything
//! else requires an analyst.
//!
//! Tokens are compared with a constant-time check (`subtle::ConstantTimeEq`-
//! equivalent via byte-by-byte loop) so a slow comparator can't be used
//! to enumerate token bytes.
//!
//! `MALTRACE_NO_AUTH=1` disables auth entirely — useful for local
//! development against `maltrace-server` without juggling tokens.

use anyhow::{Context, Result};
use axum::{
    body::Body,
    extract::Request,
    http::{header::AUTHORIZATION, Method, StatusCode},
    middleware::Next,
    response::Response,
    Json,
};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// Full read+write access. Default for new users.
    Analyst,
    /// GET/HEAD only — for handoff recipients who shouldn't mutate.
    Reader,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserRecord {
    pub name: String,
    pub token: String,
    #[serde(default = "default_analyst")]
    pub role: Role,
}

fn default_analyst() -> Role {
    Role::Analyst
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OnDisk {
    #[serde(default)]
    pub users: Vec<UserRecord>,
}

/// Read-only user info attached to authenticated requests via axum's
/// extensions map. Handlers that care about the caller's identity (e.g.
/// future per-user audit) can pull this out of `Request::extensions()`.
#[derive(Debug, Clone)]
pub struct AuthedUser {
    pub name: String,
    pub role: Role,
}

#[derive(Debug, Clone)]
pub struct UsersStore {
    path: PathBuf,
    /// In-memory map of token → user record, kept in sync with the file
    /// on every write. The RwLock lets the auth middleware do
    /// lock-free-ish lookups.
    by_token: Arc<RwLock<HashMap<String, UserRecord>>>,
    /// True when auth is bypassed (no `users.json`, no users, or
    /// `MALTRACE_NO_AUTH=1`). Lets the middleware short-circuit.
    bypass: bool,
}

impl UsersStore {
    pub fn open(data_dir: &std::path::Path) -> Self {
        let path = data_dir.join("users.json");
        let on_disk = match Self::read_from(&path) {
            Ok(o) => o,
            Err(e) => {
                eprintln!(
                    "maltrace auth: starting open (no auth) — {e}. To enable, create {} with a `users` array.",
                    path.display(),
                );
                OnDisk::default()
            }
        };
        let env_bypass = std::env::var("MALTRACE_NO_AUTH")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        let bypass = env_bypass || on_disk.users.is_empty();
        let mut map: HashMap<String, UserRecord> = HashMap::new();
        for u in on_disk.users {
            if !u.token.is_empty() {
                map.insert(u.token.clone(), u);
            }
        }
        if bypass {
            eprintln!(
                "maltrace auth: AUTH BYPASS in effect ({}).",
                if env_bypass {
                    "MALTRACE_NO_AUTH=1"
                } else {
                    "no users configured"
                }
            );
        }
        Self {
            path,
            by_token: Arc::new(RwLock::new(map)),
            bypass,
        }
    }

    fn read_from(path: &std::path::Path) -> Result<OnDisk> {
        if !path.exists() {
            return Ok(OnDisk::default());
        }
        let body = fs::read_to_string(path)
            .with_context(|| format!("reading {}", path.display()))?;
        let parsed: OnDisk = serde_json::from_str(&body)
            .with_context(|| format!("parsing {}", path.display()))?;
        Ok(parsed)
    }

    /// Returns the user record matching `token`, or `None` if none does.
    /// Constant-time compare against every stored token to avoid leaking
    /// existence-vs-mismatch timing differences. (The hashmap lookup
    /// alone would already be a timing channel.)
    pub fn lookup(&self, token: &str) -> Option<UserRecord> {
        let guard = self.by_token.read();
        let mut matched: Option<UserRecord> = None;
        for (k, v) in guard.iter() {
            if constant_time_eq(k.as_bytes(), token.as_bytes()) && matched.is_none() {
                matched = Some(v.clone());
            }
        }
        matched
    }

    pub fn bypass(&self) -> bool {
        self.bypass
    }

    /// Read the user list (without tokens) for the GET /api/auth/users
    /// endpoint. Tokens are intentionally redacted — the UI never needs
    /// them, and exposing them would defeat the auth model.
    pub fn redacted_list(&self) -> Vec<RedactedUser> {
        self.by_token
            .read()
            .values()
            .map(|u| RedactedUser {
                name: u.name.clone(),
                role: u.role.clone(),
            })
            .collect()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RedactedUser {
    pub name: String,
    pub role: Role,
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        // Comparing different-length inputs is a timing oracle for
        // length only; the caller's input length is already public over
        // the wire, so this is fine.
        return false;
    }
    let mut r: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        r |= x ^ y;
    }
    r == 0
}

/// Axum middleware: enforce `Authorization: Bearer <token>` on every
/// route under `/api/*`, except the dedicated whoami endpoint (which
/// the UI uses to detect "auth required" without retrying every call).
pub async fn require_auth(req: Request<Body>, next: Next) -> Result<Response, Response> {
    let store: Option<&UsersStore> = req.extensions().get();
    if let Some(store) = store {
        if store.bypass {
            // Fall straight through. Attach a synthetic AuthedUser so
            // handlers that read it still work.
            let mut req = req;
            req.extensions_mut().insert(AuthedUser {
                name: "anonymous".to_string(),
                role: Role::Analyst,
            });
            return Ok(next.run(req).await);
        }
    }

    // Skip the auth header check for /auth/* routes themselves; the
    // login endpoint is the only way to *obtain* knowledge of valid
    // tokens.
    let path = req.uri().path();
    if path.starts_with("/api/auth/") {
        return Ok(next.run(req).await);
    }

    let token = req
        .headers()
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|s| s.trim().to_string());

    let Some(token) = token else {
        return Err(unauth("missing Bearer token"));
    };
    let Some(store) = store else {
        return Err(unauth("auth not initialised"));
    };
    let Some(user) = store.lookup(&token) else {
        return Err(unauth("invalid token"));
    };

    // Readers can only call safe-verb routes.
    let method = req.method();
    if user.role == Role::Reader && !is_safe(method) {
        return Err(forbidden("reader role cannot mutate"));
    }

    let mut req = req;
    req.extensions_mut().insert(AuthedUser {
        name: user.name.clone(),
        role: user.role.clone(),
    });
    Ok(next.run(req).await)
}

fn is_safe(m: &Method) -> bool {
    matches!(m.as_str(), "GET" | "HEAD" | "OPTIONS")
}

fn unauth(msg: &str) -> Response {
    let body = Json(serde_json::json!({ "error": msg }));
    let mut resp = body.into_response();
    *resp.status_mut() = StatusCode::UNAUTHORIZED;
    resp.headers_mut()
        .insert("WWW-Authenticate", "Bearer".parse().unwrap());
    resp
}

fn forbidden(msg: &str) -> Response {
    let body = Json(serde_json::json!({ "error": msg }));
    let mut resp = body.into_response();
    *resp.status_mut() = StatusCode::FORBIDDEN;
    resp
}

// IntoResponse import — kept inside this module to avoid leaking
// implementation detail to the rest of the codebase.
use axum::response::IntoResponse;
