mod ai;
mod auth;
mod backup;
mod event_bus;
mod geo_export;
mod plugin_trust;
mod commands;
mod core;
mod db;
mod files;
mod http;
mod machine;
mod manifest;
mod ontology;
mod passivity;
mod plugins;
mod proxies;
mod ratelimit;
mod report;
mod scheduler;
mod secrets;
mod sidecar;
mod stix;
mod webhooks;

use crate::ai::AiState;
pub use crate::core::{AppState, SharedState};
use crate::machine::MachineRegistry;
use crate::proxies::ProxyStore;
use crate::ratelimit::RateLimiter;
use crate::secrets::SecretStore;
use crate::sidecar::SidecarRegistry;
use parking_lot::Mutex;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::Manager;

/// Bundle identifier — must match `identifier` in `tauri.conf.json`. Used so
/// the desktop binary and the standalone server resolve to the same SQLite
/// file under the platform's data dir.
pub const BUNDLE_IDENTIFIER: &str = "dev.maltrace.app";

/// Resolve the per-user data directory used by the standalone server. Matches
/// the path Tauri's `app_data_dir()` produces from `BUNDLE_IDENTIFIER`.
pub fn server_data_dir() -> PathBuf {
    let base = dirs::data_dir().unwrap_or_else(|| {
        dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")).join(".local").join("share")
    });
    base.join(BUNDLE_IDENTIFIER)
}

/// ─── Public API for the native (egui) frontend ──────────────────────────
/// The native GUI binary (`maltrace-native`) links this crate directly and
/// drives the same core the Tauri commands and HTTP handlers use — no WebView,
/// no IPC, no HTTP. These re-exports are that surface.
pub use crate::core::{
    add_entity, add_tag, ai_chat, ai_status, analyze_image_entity, create_annotation, create_case,
    create_case_snapshot, create_custom_entity_type, create_saved_search, current_case,
    delete_annotation, delete_case, delete_case_snapshot, delete_custom_entity_type,
    delete_entity, delete_saved_search, delete_secret, entity_provenance, import_case_graphml,
    list_plugins,
    expand_collection, export_case_entities_csv, export_case_graphml, export_case_report_html,
    export_case_stix, get_proxy_config, get_rate_limit_config, get_webhook_config,
    list_audit_events, list_bookmarked, list_canonical_types, list_cases,
    list_annotations, list_case_snapshots, list_edges, list_entities, list_machines,
    list_notifications, list_saved_searches, list_secret_names, list_tags_for_entity,
    list_transform_runs, list_transform_runs_for_entity, list_transforms, mark_notifications_read,
    read_entity_file_as_data_url, remove_tag, rename_case, run_all_transforms, run_machine,
    run_transform_batch, run_transform_with_params, set_case_locked, set_proxy_config,
    set_rate_limit_config, set_secret, set_webhook_config, sidecar_status, test_webhooks,
    unread_notification_count, upload_file_to_case,
    update_annotation, update_edge_relationship, update_entity_bookmark, update_entity_label,
    update_entity_locked, update_entity_notes, update_entity_position, AddEntityInput, AiChatInput,
    AiStatus, CanonicalTypeDto, EntityProvenance, TransformRunResult,
};
pub use crate::ai::{ChatReply, ChatTurn};
pub use crate::db::{
    Annotation, AuditEvent, Case, CaseSnapshotMeta, Edge, Entity, Notification, SavedSearch,
    TransformRun,
};
pub use crate::core::ImportResult;
pub use crate::machine::{MachineMeta, MachineRunResult};
pub use crate::manifest::PluginSummary;
pub use crate::proxies::ProxyConfig;
pub use crate::ratelimit::{Limits, RateLimitConfig};
pub use crate::secrets::SecretInfo;
pub use crate::sidecar::{ParameterSpec, ParameterType, SidecarStatus, TransformMeta};
pub use crate::webhooks::WebhookConfig;

/// Open application state for a directly-linked frontend. Same init path as the
/// desktop/server builds (SQLite under [`server_data_dir`], optional SQLCipher
/// passphrase from `MALTRACE_DB_PASSPHRASE`).
pub fn open_state_public(data_dir: &PathBuf) -> Result<SharedState, anyhow::Error> {
    open_state(data_dir)
}

fn open_state(data_dir: &PathBuf) -> Result<SharedState, anyhow::Error> {
    std::fs::create_dir_all(data_dir).ok();
    let db_path = data_dir.join("maltrace.sqlite");
    // SQLCipher passphrase comes from an env var so the user can
    // wire it up via shell config, a launchd plist, or systemd. Empty /
    // unset → plaintext DB (status quo).
    let passphrase = std::env::var("MALTRACE_DB_PASSPHRASE").ok();
    let db = db::Db::open(&db_path, passphrase.as_deref())?;
    db.default_case()?;
    Ok(Arc::new(AppState {
        db: Mutex::new(db),
        sidecars: SidecarRegistry::from_repo_layout(Arc::new(
            plugin_trust::TrustStore::open(data_dir),
        )),
        machines: MachineRegistry::from_repo_layout(),
        ai: AiState::load(),
        secrets: SecretStore::open(data_dir),
        proxies: ProxyStore::open(data_dir),
        rate_limiter: RateLimiter::open(data_dir),
        users: auth::UsersStore::open(data_dir),
        webhooks: webhooks::WebhookStore::open(data_dir),
        event_bus: event_bus::EventBus::new(),
        data_dir: data_dir.clone(),
    }))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let data_dir: PathBuf = app
                .path()
                .app_data_dir()
                .map_err(|e| format!("app_data_dir: {e}"))?;
            let state = open_state(&data_dir).map_err(|e| format!("opening db: {e}"))?;
            // Start the background machine scheduler. The tokio runtime
            // is already up at this point (tauri 2 multi-thread); the
            // task lives for the app's lifetime.
            let sched_state = state.clone();
            tauri::async_runtime::spawn(async move {
                scheduler::run_loop(sched_state).await;
            });
            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::current_case,
            commands::list_cases,
            commands::create_case,
            commands::rename_case,
            commands::delete_case,
            commands::export_case_graphml,
            commands::list_entities,
            commands::list_edges,
            commands::list_canonical_types,
            commands::add_entity,
            commands::update_entity_position,
            commands::delete_entity,
            commands::list_transforms,
            commands::run_transform,
            commands::run_transform_batch,
            commands::run_all_transforms,
            commands::expand_collection,
            commands::list_machines,
            commands::run_machine,
            commands::ai_status,
            commands::ai_chat,
            commands::list_plugins,
            commands::list_sidecar_status,
            commands::list_secret_names,
            commands::set_secret,
            commands::delete_secret,
            commands::list_transform_runs,
            commands::export_case_entities_csv,
            commands::upload_file_to_case,
            commands::read_entity_file_as_data_url,
            commands::create_custom_entity_type,
            commands::delete_custom_entity_type,
            commands::update_entity_notes,
            commands::update_entity_bookmark,
            commands::update_entity_label,
            commands::seed_sample_graph,
            commands::update_entity_locked,
            commands::list_bookmarked,
            commands::list_transform_runs_for_entity,
            commands::search_entities_across_cases,
            commands::list_audit_events,
            commands::list_annotations,
            commands::create_annotation,
            commands::update_annotation,
            commands::delete_annotation,
            commands::merge_entities,
            commands::set_case_locked,
            commands::entity_provenance,
            commands::edge_detail,
            commands::run_transform_with_params,
            commands::export_case_stix,
            commands::export_case_report_html,
            commands::import_case_graphml,
            commands::get_proxy_config,
            commands::set_proxy_config,
            commands::get_rate_limit_config,
            commands::set_rate_limit_config,
            commands::list_machine_schedules,
            commands::create_machine_schedule,
            commands::set_machine_schedule_enabled,
            commands::delete_machine_schedule,
            commands::list_schedule_runs,
            commands::list_tags_for_case,
            commands::list_tags_for_entity,
            commands::add_tag,
            commands::remove_tag,
            commands::snapshot_entity,
            commands::restore_entities,
            commands::install_plugin_from_directory,
            commands::install_plugin_from_tarball,
            commands::uninstall_plugin,
            commands::bootstrap_plugin_venv,
            commands::list_notifications,
            commands::unread_notification_count,
            commands::mark_notifications_read,
            commands::get_webhook_config,
            commands::set_webhook_config,
            commands::test_webhooks,
            commands::update_edge_relationship,
            commands::export_case_backup,
            commands::restore_case_backup,
            commands::list_saved_searches,
            commands::create_saved_search,
            commands::update_saved_search,
            commands::delete_saved_search,
            commands::list_case_snapshots,
            commands::create_case_snapshot,
            commands::delete_case_snapshot,
            commands::diff_case_snapshots,
            commands::reload_machines,
            commands::export_case_geojson,
            commands::export_case_kml,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Run the standalone HTTP server. Used by the `maltrace-server` binary.
///
/// `static_dir` is optional: when present, the server also serves the React
/// SPA at `/`. Pass `None` to expose only `/api/*` (useful behind a separate
/// static host).
/// CLI hook: encrypt the on-disk database at `path` with `passphrase`.
/// Exposed from the library so both binaries (`app` for Tauri, plus the
/// `maltrace-server`'s `--encrypt-db` subcommand) can call it without
/// duplicating the db.rs surface area.
pub fn encrypt_db_at(path: &std::path::Path, passphrase: &str) -> anyhow::Result<()> {
    db::Db::encrypt_database_in_place(path, passphrase)
}

pub async fn run_server(
    addr: std::net::SocketAddr,
    static_dir: Option<PathBuf>,
) -> anyhow::Result<()> {
    let state = open_state(&server_data_dir())?;
    let sched_state = state.clone();
    tokio::spawn(async move {
        scheduler::run_loop(sched_state).await;
    });
    let app = http::app(state, static_dir);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    eprintln!("maltrace-server listening on http://{addr}");
    axum::serve(listener, app).await?;
    Ok(())
}
