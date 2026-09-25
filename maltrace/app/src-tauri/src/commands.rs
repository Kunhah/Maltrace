//! Tauri command shells. All real work lives in `crate::core` so the desktop
//! and HTTP transports share one implementation.

use crate::ai::ChatReply;
use crate::core::{
    self, AddEntityInput, AiChatInput, AiStatus, CanonicalTypeDto, EdgeDetail,
    EntityProvenance, ImportResult, SeedResult, SharedState, TransformRunResult,
};
use crate::db::{
    Annotation, AuditEvent, Case, CrossCaseMatch, CustomEntityType, Edge, Entity, MachineSchedule,
    MachineScheduleRun, TransformRun,
};
use crate::proxies::ProxyConfig;
use crate::ratelimit::RateLimitConfig;
use crate::machine::{MachineMeta, MachineRunResult};
use crate::manifest::PluginSummary;
use crate::secrets::SecretInfo;
use crate::sidecar::{SidecarStatus, TransformMeta};
use tauri::State;

#[tauri::command]
pub fn list_plugins(state: State<'_, SharedState>) -> Vec<PluginSummary> {
    core::list_plugins(state.inner())
}

#[tauri::command]
pub async fn list_sidecar_status(
    state: State<'_, SharedState>,
) -> Result<Vec<SidecarStatus>, String> {
    Ok(core::sidecar_status(state.inner()).await)
}

#[tauri::command]
pub fn current_case(state: State<'_, SharedState>) -> Result<Case, String> {
    core::current_case(state.inner())
}

#[tauri::command]
pub fn list_cases(state: State<'_, SharedState>) -> Result<Vec<Case>, String> {
    core::list_cases(state.inner())
}

#[tauri::command]
pub fn create_case(state: State<'_, SharedState>, name: String) -> Result<Case, String> {
    core::create_case(state.inner(), &name)
}

#[tauri::command]
pub fn seed_sample_graph(
    state: State<'_, SharedState>,
    case_id: i64,
) -> Result<SeedResult, String> {
    core::seed_sample_graph(state.inner(), case_id)
        .map(|(entities, edges)| SeedResult { entities, edges })
}

#[tauri::command]
pub fn rename_case(
    state: State<'_, SharedState>,
    id: i64,
    name: String,
) -> Result<(), String> {
    core::rename_case(state.inner(), id, &name)
}

#[tauri::command]
pub fn delete_case(state: State<'_, SharedState>, id: i64) -> Result<(), String> {
    core::delete_case(state.inner(), id)
}

#[tauri::command]
pub fn set_case_locked(
    state: State<'_, SharedState>,
    id: i64,
    locked: bool,
) -> Result<Case, String> {
    core::set_case_locked(state.inner(), id, locked, Some("desktop"))
}

#[tauri::command]
pub fn entity_provenance(
    state: State<'_, SharedState>,
    id: String,
) -> Result<EntityProvenance, String> {
    core::entity_provenance(state.inner(), &id)
}

#[tauri::command]
pub fn edge_detail(state: State<'_, SharedState>, id: String) -> Result<EdgeDetail, String> {
    core::edge_detail(state.inner(), &id)
}

#[tauri::command]
pub fn export_case_graphml(
    state: State<'_, SharedState>,
    case_id: i64,
) -> Result<String, String> {
    core::export_case_graphml(state.inner(), case_id)
}

#[tauri::command]
pub fn list_entities(
    state: State<'_, SharedState>,
    case_id: i64,
) -> Result<Vec<Entity>, String> {
    core::list_entities(state.inner(), case_id)
}

#[tauri::command]
pub fn list_edges(
    state: State<'_, SharedState>,
    case_id: i64,
) -> Result<Vec<Edge>, String> {
    core::list_edges(state.inner(), case_id)
}

#[tauri::command]
pub fn list_canonical_types(state: State<'_, SharedState>) -> Vec<CanonicalTypeDto> {
    core::list_canonical_types(state.inner())
}

#[tauri::command]
pub fn create_custom_entity_type(
    state: State<'_, SharedState>,
    label: String,
    kind: String,
    color: String,
) -> Result<CustomEntityType, String> {
    core::create_custom_entity_type(state.inner(), &label, &kind, &color)
}

#[tauri::command]
pub fn delete_custom_entity_type(
    state: State<'_, SharedState>,
    id: String,
) -> Result<(), String> {
    core::delete_custom_entity_type(state.inner(), &id)
}

#[tauri::command]
pub fn add_entity(
    state: State<'_, SharedState>,
    case_id: i64,
    entity_type: String,
    label: String,
    pos_x: Option<f64>,
    pos_y: Option<f64>,
) -> Result<Entity, String> {
    core::add_entity(
        state.inner(),
        AddEntityInput { case_id, entity_type, label, pos_x, pos_y },
        Some("desktop"),
    )
}

#[tauri::command]
pub fn update_entity_position(
    state: State<'_, SharedState>,
    id: String,
    x: f64,
    y: f64,
) -> Result<(), String> {
    core::update_entity_position(state.inner(), &id, x, y)
}

#[tauri::command]
pub fn update_entity_notes(
    state: State<'_, SharedState>,
    id: String,
    notes: Option<String>,
) -> Result<(), String> {
    core::update_entity_notes(state.inner(), &id, notes.as_deref())
}

#[tauri::command]
pub fn update_entity_bookmark(
    state: State<'_, SharedState>,
    id: String,
    bookmarked: bool,
) -> Result<(), String> {
    core::update_entity_bookmark(state.inner(), &id, bookmarked)
}

#[tauri::command]
pub fn update_entity_label(
    state: State<'_, SharedState>,
    id: String,
    label: String,
) -> Result<(), String> {
    core::update_entity_label(state.inner(), &id, &label, Some("desktop"))
}

#[tauri::command]
pub fn update_entity_locked(
    state: State<'_, SharedState>,
    id: String,
    locked: bool,
) -> Result<(), String> {
    core::update_entity_locked(state.inner(), &id, locked)
}

#[tauri::command]
pub fn list_bookmarked(
    state: State<'_, SharedState>,
    case_id: i64,
) -> Result<Vec<Entity>, String> {
    core::list_bookmarked(state.inner(), case_id)
}

#[tauri::command]
pub fn list_transform_runs_for_entity(
    state: State<'_, SharedState>,
    entity_id: String,
    limit: Option<i64>,
) -> Result<Vec<TransformRun>, String> {
    core::list_transform_runs_for_entity(state.inner(), &entity_id, limit.unwrap_or(50))
}

#[tauri::command]
pub fn search_entities_across_cases(
    state: State<'_, SharedState>,
    query: String,
    limit: Option<i64>,
) -> Result<Vec<CrossCaseMatch>, String> {
    core::search_entities_across_cases(state.inner(), &query, limit.unwrap_or(50))
}

#[tauri::command]
pub fn list_audit_events(
    state: State<'_, SharedState>,
    case_id: i64,
    limit: Option<i64>,
) -> Result<Vec<AuditEvent>, String> {
    core::list_audit_events(state.inner(), case_id, limit.unwrap_or(100))
}

#[tauri::command]
pub fn list_annotations(
    state: State<'_, SharedState>,
    case_id: i64,
) -> Result<Vec<Annotation>, String> {
    core::list_annotations(state.inner(), case_id)
}

#[tauri::command]
pub fn create_annotation(
    state: State<'_, SharedState>,
    case_id: i64,
    text: String,
    pos_x: f64,
    pos_y: f64,
    color: Option<String>,
) -> Result<Annotation, String> {
    core::create_annotation(
        state.inner(),
        case_id,
        &text,
        pos_x,
        pos_y,
        color.as_deref(),
    )
}

#[tauri::command]
pub fn update_annotation(
    state: State<'_, SharedState>,
    id: String,
    text: Option<String>,
    pos_x: Option<f64>,
    pos_y: Option<f64>,
    color: Option<String>,
) -> Result<(), String> {
    core::update_annotation(
        state.inner(),
        &id,
        text.as_deref(),
        pos_x,
        pos_y,
        color.as_deref(),
    )
}

#[tauri::command]
pub fn delete_annotation(state: State<'_, SharedState>, id: String) -> Result<(), String> {
    core::delete_annotation(state.inner(), &id)
}

#[tauri::command]
pub fn merge_entities(
    state: State<'_, SharedState>,
    keep_id: String,
    drop_id: String,
) -> Result<(), String> {
    core::merge_entities(state.inner(), &keep_id, &drop_id, Some("desktop"))
}

#[tauri::command]
pub fn delete_entity(state: State<'_, SharedState>, id: String) -> Result<(), String> {
    core::delete_entity(state.inner(), &id, Some("desktop"))
}

#[tauri::command]
pub async fn list_transforms(
    state: State<'_, SharedState>,
) -> Result<Vec<TransformMeta>, String> {
    let state = state.inner().clone();
    core::list_transforms(&state).await
}

#[tauri::command]
pub async fn run_transform(
    state: State<'_, SharedState>,
    entity_id: String,
    transform_id: String,
) -> Result<TransformRunResult, String> {
    let state = state.inner().clone();
    core::run_transform_with_params(&state, &entity_id, &transform_id, None, Some("desktop"))
        .await
}

#[tauri::command]
pub async fn run_transform_batch(
    state: State<'_, SharedState>,
    entity_ids: Vec<String>,
    transform_id: String,
) -> Result<Vec<TransformRunResult>, String> {
    let state = state.inner().clone();
    core::run_transform_batch(&state, &entity_ids, &transform_id, Some("desktop")).await
}

#[tauri::command]
pub async fn run_all_transforms(
    state: State<'_, SharedState>,
    entity_id: String,
    transform_ids: Vec<String>,
) -> Result<Vec<TransformRunResult>, String> {
    let state = state.inner().clone();
    core::run_all_transforms(&state, &entity_id, &transform_ids, Some("desktop")).await
}

#[tauri::command]
pub fn expand_collection(
    state: State<'_, SharedState>,
    collection_id: String,
) -> Result<TransformRunResult, String> {
    core::expand_collection(state.inner(), &collection_id, Some("desktop"))
}

#[tauri::command]
pub async fn list_machines(state: State<'_, SharedState>) -> Result<Vec<MachineMeta>, String> {
    let state = state.inner().clone();
    Ok(core::list_machines(&state).await)
}

#[tauri::command]
pub async fn run_machine(
    state: State<'_, SharedState>,
    machine_id: String,
    entity_id: String,
) -> Result<MachineRunResult, String> {
    let state = state.inner().clone();
    core::run_machine_as(&state, &machine_id, &entity_id, Some("desktop")).await
}

#[tauri::command]
pub fn ai_status(state: State<'_, SharedState>) -> AiStatus {
    core::ai_status(state.inner())
}

#[tauri::command]
pub async fn ai_chat(
    state: State<'_, SharedState>,
    input: AiChatInput,
) -> Result<ChatReply, String> {
    let state = state.inner().clone();
    core::ai_chat(&state, input).await
}

#[tauri::command]
pub fn list_secret_names(state: State<'_, SharedState>) -> Vec<SecretInfo> {
    core::list_secret_names(state.inner())
}

#[tauri::command]
pub fn set_secret(
    state: State<'_, SharedState>,
    name: String,
    value: String,
) -> Result<(), String> {
    core::set_secret(state.inner(), &name, &value)
}

#[tauri::command]
pub fn delete_secret(state: State<'_, SharedState>, name: String) -> Result<(), String> {
    core::delete_secret(state.inner(), &name)
}

#[tauri::command]
pub fn list_transform_runs(
    state: State<'_, SharedState>,
    case_id: i64,
    limit: Option<i64>,
) -> Result<Vec<TransformRun>, String> {
    core::list_transform_runs(state.inner(), case_id, limit.unwrap_or(100))
}

#[tauri::command]
pub fn export_case_entities_csv(
    state: State<'_, SharedState>,
    case_id: i64,
) -> Result<String, String> {
    core::export_case_entities_csv(state.inner(), case_id)
}

#[tauri::command]
pub fn upload_file_to_case(
    state: State<'_, SharedState>,
    case_id: i64,
    file_name: String,
    bytes_base64: String,
) -> Result<Entity, String> {
    core::upload_file_to_case(state.inner(), case_id, &file_name, &bytes_base64, Some("desktop"))
}

#[tauri::command]
pub fn read_entity_file_as_data_url(
    state: State<'_, SharedState>,
    entity_id: String,
) -> Result<String, String> {
    core::read_entity_file_as_data_url(state.inner(), &entity_id)
}

#[tauri::command]
pub async fn run_transform_with_params(
    state: State<'_, SharedState>,
    entity_id: String,
    transform_id: String,
    params: serde_json::Value,
) -> Result<TransformRunResult, String> {
    let state = state.inner().clone();
    let params_json = if params.is_null() {
        None
    } else {
        Some(params.to_string())
    };
    core::run_transform_with_params(&state, &entity_id, &transform_id, params_json, Some("desktop"))
        .await
}

#[tauri::command]
pub fn export_case_stix(
    state: State<'_, SharedState>,
    case_id: i64,
) -> Result<String, String> {
    core::export_case_stix(state.inner(), case_id)
}

#[tauri::command]
pub fn export_case_report_html(
    state: State<'_, SharedState>,
    case_id: i64,
) -> Result<String, String> {
    core::export_case_report_html(state.inner(), case_id)
}

#[tauri::command]
pub fn import_case_graphml(
    state: State<'_, SharedState>,
    case_id: i64,
    xml: String,
) -> Result<ImportResult, String> {
    core::import_case_graphml(state.inner(), case_id, &xml, Some("desktop"))
}

#[tauri::command]
pub fn get_proxy_config(state: State<'_, SharedState>) -> ProxyConfig {
    core::get_proxy_config(state.inner())
}

#[tauri::command]
pub fn set_proxy_config(
    state: State<'_, SharedState>,
    config: ProxyConfig,
) -> Result<(), String> {
    core::set_proxy_config(state.inner(), config)
}

#[tauri::command]
pub fn get_rate_limit_config(state: State<'_, SharedState>) -> RateLimitConfig {
    core::get_rate_limit_config(state.inner())
}

#[tauri::command]
pub fn set_rate_limit_config(
    state: State<'_, SharedState>,
    config: RateLimitConfig,
) -> Result<(), String> {
    core::set_rate_limit_config(state.inner(), config)
}

#[tauri::command]
pub fn list_machine_schedules(
    state: State<'_, SharedState>,
    case_id: i64,
) -> Result<Vec<MachineSchedule>, String> {
    core::list_machine_schedules(state.inner(), case_id)
}

#[tauri::command]
pub fn create_machine_schedule(
    state: State<'_, SharedState>,
    case_id: i64,
    machine_id: String,
    entity_id: String,
    interval_seconds: i64,
) -> Result<MachineSchedule, String> {
    core::create_machine_schedule(
        state.inner(),
        case_id,
        &machine_id,
        &entity_id,
        interval_seconds,
    )
}

#[tauri::command]
pub fn set_machine_schedule_enabled(
    state: State<'_, SharedState>,
    id: i64,
    enabled: bool,
) -> Result<(), String> {
    core::set_machine_schedule_enabled(state.inner(), id, enabled)
}

#[tauri::command]
pub fn delete_machine_schedule(
    state: State<'_, SharedState>,
    id: i64,
) -> Result<(), String> {
    core::delete_machine_schedule(state.inner(), id)
}

#[tauri::command]
pub fn list_schedule_runs(
    state: State<'_, SharedState>,
    schedule_id: i64,
    limit: Option<i64>,
) -> Result<Vec<MachineScheduleRun>, String> {
    core::list_schedule_runs(state.inner(), schedule_id, limit.unwrap_or(50))
}

#[tauri::command]
pub fn list_tags_for_case(
    state: State<'_, SharedState>,
    case_id: i64,
) -> Result<std::collections::HashMap<String, Vec<String>>, String> {
    core::list_tags_for_case(state.inner(), case_id)
}

#[tauri::command]
pub fn list_tags_for_entity(
    state: State<'_, SharedState>,
    entity_id: String,
) -> Result<Vec<String>, String> {
    core::list_tags_for_entity(state.inner(), &entity_id)
}

#[tauri::command]
pub fn add_tag(
    state: State<'_, SharedState>,
    entity_id: String,
    tag: String,
) -> Result<(), String> {
    core::add_tag(state.inner(), &entity_id, &tag)
}

#[tauri::command]
pub fn remove_tag(
    state: State<'_, SharedState>,
    entity_id: String,
    tag: String,
) -> Result<(), String> {
    core::remove_tag(state.inner(), &entity_id, &tag)
}

#[tauri::command]
pub fn snapshot_entity(
    state: State<'_, SharedState>,
    entity_id: String,
) -> Result<crate::core::EntitySnapshot, String> {
    core::snapshot_entity(state.inner(), &entity_id)
}

#[tauri::command]
pub fn restore_entities(
    state: State<'_, SharedState>,
    snapshots: Vec<crate::core::EntitySnapshot>,
) -> Result<Vec<Entity>, String> {
    core::restore_entities(state.inner(), &snapshots)
}

#[tauri::command]
pub fn install_plugin_from_directory(
    state: State<'_, SharedState>,
    source_dir: String,
) -> Result<crate::plugins::InstallResult, String> {
    core::install_plugin_from_directory(state.inner(), &source_dir)
}

#[tauri::command]
pub fn install_plugin_from_tarball(
    state: State<'_, SharedState>,
    bytes_base64: String,
) -> Result<crate::plugins::InstallResult, String> {
    core::install_plugin_from_tarball(state.inner(), &bytes_base64)
}

#[tauri::command]
pub fn uninstall_plugin(
    state: State<'_, SharedState>,
    plugin_id: String,
) -> Result<(), String> {
    core::uninstall_plugin(state.inner(), &plugin_id)
}

#[tauri::command]
pub fn bootstrap_plugin_venv(
    state: State<'_, SharedState>,
    plugin_id: String,
    python_bin: Option<String>,
) -> Result<crate::plugins::BootstrapResult, String> {
    core::bootstrap_plugin_venv(state.inner(), &plugin_id, python_bin.as_deref())
}

#[tauri::command]
pub fn list_notifications(
    state: State<'_, SharedState>,
    case_id: i64,
    limit: Option<i64>,
) -> Result<Vec<crate::db::Notification>, String> {
    core::list_notifications(state.inner(), case_id, limit.unwrap_or(100))
}

#[tauri::command]
pub fn unread_notification_count(
    state: State<'_, SharedState>,
    case_id: i64,
) -> Result<i64, String> {
    core::unread_notification_count(state.inner(), case_id)
}

#[tauri::command]
pub fn mark_notifications_read(
    state: State<'_, SharedState>,
    case_id: i64,
) -> Result<(), String> {
    core::mark_notifications_read(state.inner(), case_id)
}

#[tauri::command]
pub fn get_webhook_config(state: State<'_, SharedState>) -> crate::webhooks::WebhookConfig {
    core::get_webhook_config(state.inner())
}

#[tauri::command]
pub fn set_webhook_config(
    state: State<'_, SharedState>,
    config: crate::webhooks::WebhookConfig,
) -> Result<(), String> {
    core::set_webhook_config(state.inner(), config)
}

#[tauri::command]
pub async fn test_webhooks(state: State<'_, SharedState>) -> Result<usize, String> {
    let state = state.inner().clone();
    core::test_webhooks(&state).await
}

#[tauri::command]
pub fn update_edge_relationship(
    state: State<'_, SharedState>,
    edge_id: String,
    relationship_type: Option<String>,
) -> Result<(), String> {
    core::update_edge_relationship(
        state.inner(),
        &edge_id,
        relationship_type.as_deref(),
    )
}

#[tauri::command]
pub fn export_case_backup(
    state: State<'_, SharedState>,
    case_id: i64,
) -> Result<crate::backup::ExportResult, String> {
    core::export_case_backup(state.inner(), case_id)
}

#[tauri::command]
pub fn restore_case_backup(
    state: State<'_, SharedState>,
    bytes_base64: String,
) -> Result<crate::backup::RestoreResult, String> {
    core::restore_case_backup(state.inner(), &bytes_base64)
}

#[tauri::command]
pub fn list_saved_searches(
    state: State<'_, SharedState>,
    case_id: i64,
) -> Result<Vec<crate::db::SavedSearch>, String> {
    core::list_saved_searches(state.inner(), case_id)
}

#[tauri::command]
pub fn create_saved_search(
    state: State<'_, SharedState>,
    case_id: i64,
    name: String,
    predicate: String,
) -> Result<crate::db::SavedSearch, String> {
    core::create_saved_search(state.inner(), case_id, &name, &predicate)
}

#[tauri::command]
pub fn update_saved_search(
    state: State<'_, SharedState>,
    id: i64,
    name: String,
    predicate: String,
) -> Result<(), String> {
    core::update_saved_search(state.inner(), id, &name, &predicate)
}

#[tauri::command]
pub fn delete_saved_search(
    state: State<'_, SharedState>,
    id: i64,
) -> Result<(), String> {
    core::delete_saved_search(state.inner(), id)
}

#[tauri::command]
pub fn list_case_snapshots(
    state: State<'_, SharedState>,
    case_id: i64,
) -> Result<Vec<crate::db::CaseSnapshotMeta>, String> {
    core::list_case_snapshots(state.inner(), case_id)
}

#[tauri::command]
pub fn create_case_snapshot(
    state: State<'_, SharedState>,
    case_id: i64,
    name: String,
) -> Result<crate::db::CaseSnapshotMeta, String> {
    core::create_case_snapshot(state.inner(), case_id, &name)
}

#[tauri::command]
pub fn delete_case_snapshot(
    state: State<'_, SharedState>,
    id: i64,
) -> Result<(), String> {
    core::delete_case_snapshot(state.inner(), id)
}

#[tauri::command]
pub fn diff_case_snapshots(
    state: State<'_, SharedState>,
    snapshot_id: i64,
    other_id: Option<i64>,
) -> Result<crate::db::CaseDiff, String> {
    core::diff_case_snapshots(state.inner(), snapshot_id, other_id)
}

#[tauri::command]
pub fn reload_machines(state: State<'_, SharedState>) -> usize {
    core::reload_machines(state.inner())
}

#[tauri::command]
pub fn export_case_geojson(
    state: State<'_, SharedState>,
    case_id: i64,
) -> Result<String, String> {
    core::export_case_geojson(state.inner(), case_id)
}

#[tauri::command]
pub fn export_case_kml(
    state: State<'_, SharedState>,
    case_id: i64,
) -> Result<String, String> {
    core::export_case_kml(state.inner(), case_id)
}
