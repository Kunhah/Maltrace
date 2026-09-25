//! Axum HTTP layer exposing the same operations as the Tauri commands.
//!
//! Used by the `maltrace-server` binary so the web frontend can talk to a
//! native backend over `fetch`. All handlers are thin shells over `crate::core`.

use crate::auth::{self, UsersStore};
use crate::core::{self, AddEntityInput, AiChatInput, PositionInput, SharedState};
use crate::proxies::ProxyConfig;
use crate::ratelimit::RateLimitConfig;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, patch, post},
    Json, Router,
};
use serde::Deserialize;
use std::path::PathBuf;
use tower_http::{cors::CorsLayer, services::ServeDir, trace::TraceLayer};

#[derive(Debug, Deserialize)]
pub struct CaseQuery {
    pub case_id: i64,
}

#[derive(Debug, Deserialize)]
pub struct RunTransformBody {
    pub entity_id: String,
}

#[derive(Debug, Deserialize)]
pub struct RunTransformBatchBody {
    pub entity_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct RunMachineBody {
    pub entity_id: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateCaseBody {
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct RenameCaseBody {
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct SetSecretBody {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Deserialize)]
pub struct RunsQuery {
    #[serde(default)]
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    pub q: String,
    #[serde(default)]
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct UploadFileBody {
    pub file_name: String,
    /// Base64 of the file's bytes. May include the `data:...;base64,`
    /// prefix from `FileReader.readAsDataURL`; the core strips it.
    pub bytes_base64: String,
}

#[derive(Debug, Deserialize)]
pub struct RunAllBody {
    pub transform_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateCustomTypeBody {
    pub label: String,
    pub kind: String,
    pub color: String,
}

#[derive(Debug, Deserialize)]
pub struct NotesBody {
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct BookmarkBody {
    pub bookmarked: bool,
}

#[derive(Debug, Deserialize)]
pub struct LabelBody {
    pub label: String,
}

#[derive(Debug, Deserialize)]
pub struct LockedBody {
    pub locked: bool,
}

#[derive(Debug, Deserialize)]
pub struct CaseLockBody {
    pub locked: bool,
}

#[derive(Debug, Deserialize)]
pub struct ImportGraphmlBody {
    pub xml: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateScheduleBody {
    pub machine_id: String,
    pub entity_id: String,
    pub interval_seconds: i64,
}

#[derive(Debug, Deserialize)]
pub struct PatchScheduleBody {
    pub enabled: bool,
}

#[derive(Debug, Deserialize)]
pub struct RunWithParamsBody {
    pub entity_id: String,
    #[serde(default)]
    pub params: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct MergeBody {
    pub keep_id: String,
    pub drop_id: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateAnnotationBody {
    pub text: String,
    pub pos_x: f64,
    pub pos_y: f64,
    pub color: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateAnnotationBody {
    pub text: Option<String>,
    pub pos_x: Option<f64>,
    pub pos_y: Option<f64>,
    pub color: Option<String>,
}

/// Build the JSON API router. Mounted under `/api` by [`app`].
fn api_router(state: SharedState) -> Router {
    Router::new()
        .route("/case", get(get_case))
        .route(
            "/cases",
            get(list_cases_handler).post(create_case_handler),
        )
        .route(
            "/cases/:id",
            patch(rename_case_handler).delete(delete_case_handler),
        )
        .route("/cases/:id/lock", patch(patch_case_lock_handler))
        .route("/cases/:id/graphml", get(export_graphml_handler))
        .route("/cases/:id/graphml/import", post(import_graphml_handler))
        .route("/cases/:id/stix.json", get(export_stix_handler))
        .route("/cases/:id/report.html", get(export_report_handler))
        .route(
            "/cases/:id/schedules",
            get(list_schedules_handler).post(create_schedule_handler),
        )
        .route(
            "/schedules/:id",
            patch(patch_schedule_handler).delete(delete_schedule_handler),
        )
        .route("/schedules/:id/runs", get(list_schedule_runs_handler))
        .route(
            "/settings/proxies",
            get(get_proxies_handler).put(set_proxies_handler),
        )
        .route(
            "/settings/rate-limits",
            get(get_rate_limits_handler).put(set_rate_limits_handler),
        )
        .route(
            "/transforms/:transform_id/run-with-params",
            post(run_transform_with_params_handler),
        )
        .route(
            "/cases/:id/notifications",
            get(list_notifications_handler).post(mark_read_handler),
        )
        .route("/cases/:id/notifications/unread", get(unread_count_handler))
        .route(
            "/settings/webhooks",
            get(get_webhooks_handler).put(set_webhooks_handler),
        )
        .route("/settings/webhooks/test", post(test_webhooks_handler))
        .route(
            "/edges/:id/relationship",
            patch(patch_edge_relationship_handler),
        )
        .route("/cases/:id/backup", get(export_backup_handler))
        .route("/cases/backup/restore", post(restore_backup_handler))
        .route(
            "/cases/:id/saved-searches",
            get(list_saved_searches_handler).post(create_saved_search_handler),
        )
        .route(
            "/saved-searches/:id",
            patch(patch_saved_search_handler).delete(delete_saved_search_handler),
        )
        .route(
            "/cases/:id/snapshots",
            get(list_snapshots_handler).post(create_snapshot_handler),
        )
        .route(
            "/snapshots/:id",
            axum::routing::delete(delete_snapshot_handler),
        )
        .route("/snapshots/:id/diff", get(diff_snapshot_handler))
        .route("/machines/reload", post(reload_machines_handler))
        .route("/cases/:id/geo.geojson", get(export_geojson_handler))
        .route("/cases/:id/geo.kml", get(export_kml_handler))
        .route("/events", get(events_stream_handler))
        .route("/cases/:id/seed-sample", post(seed_sample_handler))
        .route("/cases/:id/entities.csv", get(export_csv_handler))
        .route("/cases/:id/files", post(upload_file_handler))
        .route("/cases/:id/runs", get(list_runs_handler))
        .route("/entities/:id/file", get(read_file_handler))
        .route("/entities/:id/run-all", post(run_all_handler))
        .route(
            "/secrets",
            get(list_secrets_handler).post(set_secret_handler),
        )
        .route(
            "/secrets/:name",
            axum::routing::delete(delete_secret_handler),
        )
        .route("/entities", get(list_entities).post(add_entity))
        .route("/entities/:id", axum::routing::delete(delete_entity))
        .route("/entities/:id/position", patch(update_entity_position))
        .route("/entities/:id/notes", patch(patch_notes_handler))
        .route("/entities/:id/bookmark", patch(patch_bookmark_handler))
        .route("/entities/:id/label", patch(patch_label_handler))
        .route("/entities/:id/locked", patch(patch_locked_handler))
        .route("/entities/:id/runs", get(list_runs_for_entity_handler))
        .route("/entities/search", get(search_entities_handler))
        .route("/cases/:id/audit", get(list_audit_handler))
        .route(
            "/entities/:id/provenance",
            get(entity_provenance_handler),
        )
        .route("/edges/:id", get(edge_detail_handler))
        .route("/entities/merge", post(merge_entities_handler))
        .route(
            "/cases/:id/bookmarks",
            get(list_bookmarks_handler),
        )
        .route(
            "/cases/:id/annotations",
            get(list_annotations_handler).post(create_annotation_handler),
        )
        .route(
            "/annotations/:id",
            patch(patch_annotation_handler).delete(delete_annotation_handler),
        )
        .route("/edges", get(list_edges))
        .route(
            "/canonical-types",
            get(list_canonical_types).post(create_custom_type_handler),
        )
        .route(
            "/canonical-types/:id",
            axum::routing::delete(delete_custom_type_handler),
        )
        .route("/auth/whoami", get(whoami_handler))
        .route("/auth/login", post(login_handler))
        .route("/tags", get(list_tags_for_case_handler))
        .route("/entities/:id/tags", get(list_tags_for_entity_handler))
        .route("/entities/:id/tags/:tag", post(add_tag_handler).delete(remove_tag_handler))
        .route("/entities/:id/snapshot", get(snapshot_entity_handler))
        .route("/entities/restore", post(restore_entities_handler))
        .route("/plugins/install/directory", post(install_plugin_dir_handler))
        .route("/plugins/install/tarball", post(install_plugin_tarball_handler))
        .route("/plugins/:id/uninstall", post(uninstall_plugin_handler))
        .route("/plugins/:id/bootstrap-venv", post(bootstrap_venv_handler))
        .route("/transforms", get(list_transforms))
        .route("/sidecar-status", get(sidecar_status_handler))
        .route("/transforms/:transform_id/run", post(run_transform))
        .route("/transforms/:transform_id/run-batch", post(run_transform_batch))
        .route("/collections/:id/expand", post(expand_collection))
        .route("/machines", get(list_machines))
        .route("/machines/:machine_id/run", post(run_machine))
        .route("/ai/status", get(ai_status))
        .route("/ai/chat", post(ai_chat))
        .with_state(state)
}

/// Full app router with the JSON API at `/api` and (optionally) static frontend
/// files served from `static_dir` at `/`.
pub fn app(state: SharedState, static_dir: Option<PathBuf>) -> Router {
    // Attach the auth middleware so every /api/* route is gated except
    // /api/auth/*. Both the middleware and the AppState live in
    // request extensions; the middleware reads the UsersStore from
    // there so it can read .bypass without a global.
    let users = state.users.clone();
    let api = api_router(state)
        .layer(axum::middleware::from_fn(auth::require_auth))
        .layer(axum::middleware::from_fn(
            move |mut req: axum::extract::Request<axum::body::Body>, next: axum::middleware::Next| {
                let users = users.clone();
                async move {
                    req.extensions_mut().insert(users);
                    next.run(req).await
                }
            },
        ));
    let mut router = Router::new().nest("/api", api);
    if let Some(dir) = static_dir {
        // SPA: serve index.html as the fallback so client-side navigation works.
        let index = dir.join("index.html");
        router = router
            .fallback_service(ServeDir::new(dir.clone()).fallback(
                tower::service_fn(move |_req: axum::http::Request<axum::body::Body>| {
                    let index = index.clone();
                    async move {
                        match tokio::fs::read(&index).await {
                            Ok(bytes) => Ok::<_, std::convert::Infallible>(
                                ([("content-type", "text/html")], bytes).into_response(),
                            ),
                            Err(_) => Ok((StatusCode::NOT_FOUND, "not found").into_response()),
                        }
                    }
                }),
            ));
    }
    router.layer(CorsLayer::permissive()).layer(TraceLayer::new_for_http())
}

async fn get_case(State(state): State<SharedState>) -> ApiResult<impl IntoResponse> {
    Ok(Json(core::current_case(&state).map_err(server_err)?))
}

async fn list_cases_handler(
    State(state): State<SharedState>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(core::list_cases(&state).map_err(server_err)?))
}

async fn create_case_handler(
    State(state): State<SharedState>,
    Json(body): Json<CreateCaseBody>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(core::create_case(&state, &body.name).map_err(server_err)?))
}

async fn rename_case_handler(
    State(state): State<SharedState>,
    Path(id): Path<i64>,
    Json(body): Json<RenameCaseBody>,
) -> ApiResult<impl IntoResponse> {
    core::rename_case(&state, id, &body.name).map_err(server_err)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_case_handler(
    State(state): State<SharedState>,
    Path(id): Path<i64>,
) -> ApiResult<impl IntoResponse> {
    core::delete_case(&state, id).map_err(server_err)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn patch_case_lock_handler(
    State(state): State<SharedState>,
    Path(id): Path<i64>,
    user: Option<axum::Extension<auth::AuthedUser>>,
    Json(body): Json<CaseLockBody>,
) -> ApiResult<impl IntoResponse> {
    let actor = user.as_ref().map(|u| u.0.name.as_str()).or(Some("anonymous"));
    Ok(Json(
        core::set_case_locked(&state, id, body.locked, actor).map_err(server_err)?,
    ))
}

async fn entity_provenance_handler(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(core::entity_provenance(&state, &id).map_err(server_err)?))
}

async fn edge_detail_handler(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(core::edge_detail(&state, &id).map_err(server_err)?))
}

async fn export_graphml_handler(
    State(state): State<SharedState>,
    Path(id): Path<i64>,
) -> ApiResult<impl IntoResponse> {
    let body = core::export_case_graphml(&state, id).map_err(server_err)?;
    Ok((
        [
            ("content-type", "application/xml"),
            (
                "content-disposition",
                "attachment; filename=\"maltrace-case.graphml\"",
            ),
        ],
        body,
    )
        .into_response())
}

async fn seed_sample_handler(
    State(state): State<SharedState>,
    Path(id): Path<i64>,
) -> ApiResult<impl IntoResponse> {
    let (entities, edges) = core::seed_sample_graph(&state, id).map_err(server_err)?;
    Ok(Json(serde_json::json!({ "entities": entities, "edges": edges })))
}

async fn export_csv_handler(
    State(state): State<SharedState>,
    Path(id): Path<i64>,
) -> ApiResult<impl IntoResponse> {
    let body = core::export_case_entities_csv(&state, id).map_err(server_err)?;
    Ok((
        [
            ("content-type", "text/csv; charset=utf-8"),
            (
                "content-disposition",
                "attachment; filename=\"maltrace-entities.csv\"",
            ),
        ],
        body,
    )
        .into_response())
}

async fn list_runs_handler(
    State(state): State<SharedState>,
    Path(id): Path<i64>,
    Query(q): Query<RunsQuery>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(
        core::list_transform_runs(&state, id, q.limit.unwrap_or(100))
            .map_err(server_err)?,
    ))
}

async fn list_secrets_handler(
    State(state): State<SharedState>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(core::list_secret_names(&state)))
}

async fn set_secret_handler(
    State(state): State<SharedState>,
    Json(body): Json<SetSecretBody>,
) -> ApiResult<impl IntoResponse> {
    core::set_secret(&state, &body.name, &body.value).map_err(server_err)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_secret_handler(
    State(state): State<SharedState>,
    Path(name): Path<String>,
) -> ApiResult<impl IntoResponse> {
    core::delete_secret(&state, &name).map_err(server_err)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn upload_file_handler(
    State(state): State<SharedState>,
    Path(case_id): Path<i64>,
    user: Option<axum::Extension<auth::AuthedUser>>,
    Json(body): Json<UploadFileBody>,
) -> ApiResult<impl IntoResponse> {
    let actor = user.as_ref().map(|u| u.0.name.as_str()).or(Some("anonymous"));
    Ok(Json(
        core::upload_file_to_case(&state, case_id, &body.file_name, &body.bytes_base64, actor)
            .map_err(server_err)?,
    ))
}

async fn read_file_handler(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> ApiResult<impl IntoResponse> {
    Ok(core::read_entity_file_as_data_url(&state, &id).map_err(server_err)?)
}

async fn run_all_handler(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    user: Option<axum::Extension<auth::AuthedUser>>,
    Json(body): Json<RunAllBody>,
) -> ApiResult<impl IntoResponse> {
    let actor = user.as_ref().map(|u| u.0.name.as_str()).or(Some("anonymous"));
    Ok(Json(
        core::run_all_transforms(&state, &id, &body.transform_ids, actor)
            .await
            .map_err(server_err)?,
    ))
}

async fn create_custom_type_handler(
    State(state): State<SharedState>,
    Json(body): Json<CreateCustomTypeBody>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(
        core::create_custom_entity_type(&state, &body.label, &body.kind, &body.color)
            .map_err(server_err)?,
    ))
}

async fn delete_custom_type_handler(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> ApiResult<impl IntoResponse> {
    core::delete_custom_entity_type(&state, &id).map_err(server_err)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn patch_notes_handler(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(body): Json<NotesBody>,
) -> ApiResult<impl IntoResponse> {
    core::update_entity_notes(&state, &id, body.notes.as_deref()).map_err(server_err)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn patch_bookmark_handler(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(body): Json<BookmarkBody>,
) -> ApiResult<impl IntoResponse> {
    core::update_entity_bookmark(&state, &id, body.bookmarked).map_err(server_err)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn patch_label_handler(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    user: Option<axum::Extension<auth::AuthedUser>>,
    Json(body): Json<LabelBody>,
) -> ApiResult<impl IntoResponse> {
    let actor = user.as_ref().map(|u| u.0.name.as_str()).or(Some("anonymous"));
    core::update_entity_label(&state, &id, &body.label, actor).map_err(server_err)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn patch_locked_handler(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(body): Json<LockedBody>,
) -> ApiResult<impl IntoResponse> {
    core::update_entity_locked(&state, &id, body.locked).map_err(server_err)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_runs_for_entity_handler(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Query(q): Query<RunsQuery>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(
        core::list_transform_runs_for_entity(&state, &id, q.limit.unwrap_or(50))
            .map_err(server_err)?,
    ))
}

async fn search_entities_handler(
    State(state): State<SharedState>,
    Query(q): Query<SearchQuery>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(
        core::search_entities_across_cases(&state, &q.q, q.limit.unwrap_or(50))
            .map_err(server_err)?,
    ))
}

async fn list_audit_handler(
    State(state): State<SharedState>,
    Path(id): Path<i64>,
    Query(q): Query<RunsQuery>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(
        core::list_audit_events(&state, id, q.limit.unwrap_or(100))
            .map_err(server_err)?,
    ))
}

async fn merge_entities_handler(
    State(state): State<SharedState>,
    user: Option<axum::Extension<auth::AuthedUser>>,
    Json(body): Json<MergeBody>,
) -> ApiResult<impl IntoResponse> {
    let actor = user.as_ref().map(|u| u.0.name.as_str()).or(Some("anonymous"));
    core::merge_entities(&state, &body.keep_id, &body.drop_id, actor).map_err(server_err)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_bookmarks_handler(
    State(state): State<SharedState>,
    Path(id): Path<i64>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(core::list_bookmarked(&state, id).map_err(server_err)?))
}

async fn list_annotations_handler(
    State(state): State<SharedState>,
    Path(id): Path<i64>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(core::list_annotations(&state, id).map_err(server_err)?))
}

async fn create_annotation_handler(
    State(state): State<SharedState>,
    Path(id): Path<i64>,
    Json(body): Json<CreateAnnotationBody>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(
        core::create_annotation(
            &state,
            id,
            &body.text,
            body.pos_x,
            body.pos_y,
            body.color.as_deref(),
        )
        .map_err(server_err)?,
    ))
}

async fn patch_annotation_handler(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(body): Json<UpdateAnnotationBody>,
) -> ApiResult<impl IntoResponse> {
    core::update_annotation(
        &state,
        &id,
        body.text.as_deref(),
        body.pos_x,
        body.pos_y,
        body.color.as_deref(),
    )
    .map_err(server_err)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_annotation_handler(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> ApiResult<impl IntoResponse> {
    core::delete_annotation(&state, &id).map_err(server_err)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_entities(
    State(state): State<SharedState>,
    Query(q): Query<CaseQuery>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(core::list_entities(&state, q.case_id).map_err(server_err)?))
}

async fn list_edges(
    State(state): State<SharedState>,
    Query(q): Query<CaseQuery>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(core::list_edges(&state, q.case_id).map_err(server_err)?))
}

async fn list_canonical_types(
    State(state): State<SharedState>,
) -> impl IntoResponse {
    Json(core::list_canonical_types(&state))
}

async fn add_entity(
    State(state): State<SharedState>,
    user: Option<axum::Extension<auth::AuthedUser>>,
    Json(input): Json<AddEntityInput>,
) -> ApiResult<impl IntoResponse> {
    let actor = user.as_ref().map(|u| u.0.name.as_str()).or(Some("anonymous"));
    Ok(Json(core::add_entity(&state, input, actor).map_err(server_err)?))
}

async fn update_entity_position(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(input): Json<PositionInput>,
) -> ApiResult<impl IntoResponse> {
    core::update_entity_position(&state, &id, input.x, input.y).map_err(server_err)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_entity(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    user: Option<axum::Extension<auth::AuthedUser>>,
) -> ApiResult<impl IntoResponse> {
    let actor = user.as_ref().map(|u| u.0.name.as_str()).or(Some("anonymous"));
    core::delete_entity(&state, &id, actor).map_err(server_err)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_transforms(
    State(state): State<SharedState>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(core::list_transforms(&state).await.map_err(server_err)?))
}

async fn run_transform(
    State(state): State<SharedState>,
    Path(transform_id): Path<String>,
    user: Option<axum::Extension<auth::AuthedUser>>,
    Json(body): Json<RunTransformBody>,
) -> ApiResult<impl IntoResponse> {
    let actor = user.as_ref().map(|u| u.0.name.as_str()).or(Some("anonymous"));
    Ok(Json(
        core::run_transform_with_params(&state, &body.entity_id, &transform_id, None, actor)
            .await
            .map_err(server_err)?,
    ))
}

async fn run_transform_batch(
    State(state): State<SharedState>,
    Path(transform_id): Path<String>,
    user: Option<axum::Extension<auth::AuthedUser>>,
    Json(body): Json<RunTransformBatchBody>,
) -> ApiResult<impl IntoResponse> {
    let actor = user.as_ref().map(|u| u.0.name.as_str()).or(Some("anonymous"));
    Ok(Json(
        core::run_transform_batch(&state, &body.entity_ids, &transform_id, actor)
            .await
            .map_err(server_err)?,
    ))
}

async fn expand_collection(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    user: Option<axum::Extension<auth::AuthedUser>>,
) -> ApiResult<impl IntoResponse> {
    let actor = user.as_ref().map(|u| u.0.name.as_str()).or(Some("anonymous"));
    Ok(Json(
        core::expand_collection(&state, &id, actor).map_err(server_err)?,
    ))
}

async fn list_machines(
    State(state): State<SharedState>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(core::list_machines(&state).await))
}

async fn run_machine(
    State(state): State<SharedState>,
    Path(machine_id): Path<String>,
    user: Option<axum::Extension<auth::AuthedUser>>,
    Json(body): Json<RunMachineBody>,
) -> ApiResult<impl IntoResponse> {
    let actor = user.as_ref().map(|u| u.0.name.as_str()).or(Some("anonymous"));
    Ok(Json(
        core::run_machine_as(&state, &machine_id, &body.entity_id, actor)
            .await
            .map_err(server_err)?,
    ))
}

async fn ai_status(State(state): State<SharedState>) -> impl IntoResponse {
    Json(core::ai_status(&state))
}

async fn ai_chat(
    State(state): State<SharedState>,
    Json(input): Json<AiChatInput>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(core::ai_chat(&state, input).await.map_err(server_err)?))
}

type ApiResult<T> = Result<T, ApiError>;

pub struct ApiError {
    status: StatusCode,
    message: String,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(serde_json::json!({ "error": self.message }))).into_response()
    }
}

fn server_err(e: String) -> ApiError {
    ApiError {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        message: e,
    }
}

async fn import_graphml_handler(
    State(state): State<SharedState>,
    Path(case_id): Path<i64>,
    user: Option<axum::Extension<auth::AuthedUser>>,
    Json(body): Json<ImportGraphmlBody>,
) -> ApiResult<impl IntoResponse> {
    let actor = user.as_ref().map(|u| u.0.name.as_str()).or(Some("anonymous"));
    Ok(Json(
        core::import_case_graphml(&state, case_id, &body.xml, actor).map_err(server_err)?,
    ))
}

async fn export_stix_handler(
    State(state): State<SharedState>,
    Path(id): Path<i64>,
) -> ApiResult<impl IntoResponse> {
    let body = core::export_case_stix(&state, id).map_err(server_err)?;
    Ok((
        [
            ("content-type", "application/json"),
            (
                "content-disposition",
                "attachment; filename=\"maltrace-case-stix.json\"",
            ),
        ],
        body,
    )
        .into_response())
}

async fn export_report_handler(
    State(state): State<SharedState>,
    Path(id): Path<i64>,
) -> ApiResult<impl IntoResponse> {
    let body = core::export_case_report_html(&state, id).map_err(server_err)?;
    Ok((
        [
            ("content-type", "text/html; charset=utf-8"),
            (
                "content-disposition",
                "attachment; filename=\"maltrace-case-report.html\"",
            ),
        ],
        body,
    )
        .into_response())
}

async fn list_schedules_handler(
    State(state): State<SharedState>,
    Path(id): Path<i64>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(
        core::list_machine_schedules(&state, id).map_err(server_err)?,
    ))
}

async fn create_schedule_handler(
    State(state): State<SharedState>,
    Path(case_id): Path<i64>,
    Json(body): Json<CreateScheduleBody>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(
        core::create_machine_schedule(
            &state,
            case_id,
            &body.machine_id,
            &body.entity_id,
            body.interval_seconds,
        )
        .map_err(server_err)?,
    ))
}

async fn patch_schedule_handler(
    State(state): State<SharedState>,
    Path(id): Path<i64>,
    Json(body): Json<PatchScheduleBody>,
) -> ApiResult<impl IntoResponse> {
    core::set_machine_schedule_enabled(&state, id, body.enabled).map_err(server_err)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_schedule_handler(
    State(state): State<SharedState>,
    Path(id): Path<i64>,
) -> ApiResult<impl IntoResponse> {
    core::delete_machine_schedule(&state, id).map_err(server_err)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_schedule_runs_handler(
    State(state): State<SharedState>,
    Path(id): Path<i64>,
    Query(q): Query<RunsQuery>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(
        core::list_schedule_runs(&state, id, q.limit.unwrap_or(50)).map_err(server_err)?,
    ))
}

async fn get_proxies_handler(State(state): State<SharedState>) -> impl IntoResponse {
    Json(core::get_proxy_config(&state))
}

/// Per-sidecar load status (which sidecars listed their transforms vs.
/// failed to start). Powers the startup warning toast that names any
/// sidecar that silently dropped out of the catalogue.
async fn sidecar_status_handler(State(state): State<SharedState>) -> impl IntoResponse {
    Json(core::sidecar_status(&state).await)
}

async fn set_proxies_handler(
    State(state): State<SharedState>,
    Json(body): Json<ProxyConfig>,
) -> ApiResult<impl IntoResponse> {
    core::set_proxy_config(&state, body).map_err(server_err)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn get_rate_limits_handler(State(state): State<SharedState>) -> impl IntoResponse {
    Json(core::get_rate_limit_config(&state))
}

async fn set_rate_limits_handler(
    State(state): State<SharedState>,
    Json(body): Json<RateLimitConfig>,
) -> ApiResult<impl IntoResponse> {
    core::set_rate_limit_config(&state, body).map_err(server_err)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn run_transform_with_params_handler(
    State(state): State<SharedState>,
    Path(transform_id): Path<String>,
    user: Option<axum::Extension<auth::AuthedUser>>,
    Json(body): Json<RunWithParamsBody>,
) -> ApiResult<impl IntoResponse> {
    let params_json = body.params.map(|v| v.to_string());
    let actor = user.as_ref().map(|u| u.0.name.as_str()).or(Some("anonymous"));
    Ok(Json(
        core::run_transform_with_params(&state, &body.entity_id, &transform_id, params_json, actor)
            .await
            .map_err(server_err)?,
    ))
}

// ─── Auth ────────────────────────────────────────────────────────
#[derive(Debug, Deserialize)]
pub struct LoginBody {
    pub token: String,
}

async fn whoami_handler(req: axum::extract::Request<axum::body::Body>) -> impl IntoResponse {
    // The auth middleware skips this route, so `AuthedUser` may be
    // absent. We use it when set; otherwise report whether auth is
    // currently bypassed so the UI can decide whether to show a login
    // prompt.
    let user: Option<&crate::auth::AuthedUser> = req.extensions().get();
    let users: Option<&UsersStore> = req.extensions().get();
    let bypass = users.map(|u| u.bypass()).unwrap_or(true);
    Json(serde_json::json!({
        "bypass": bypass,
        "authenticated": user.is_some() && !bypass,
        "name": user.map(|u| u.name.clone()),
        "role": user.map(|u| match u.role {
            crate::auth::Role::Analyst => "analyst",
            crate::auth::Role::Reader  => "reader",
        }),
    }))
}

async fn login_handler(
    req: axum::extract::Request<axum::body::Body>,
) -> ApiResult<impl IntoResponse> {
    // Manual body parse so we can pull the UsersStore from extensions
    // first — middleware order means `Json` would consume the body
    // before we can read extensions.
    let (parts, body) = req.into_parts();
    let users = parts
        .extensions
        .get::<UsersStore>()
        .cloned()
        .ok_or_else(|| server_err("auth not initialised".into()))?;
    let bytes = axum::body::to_bytes(body, 1024 * 16)
        .await
        .map_err(|e| server_err(format!("reading login body: {e}")))?;
    let body: LoginBody = serde_json::from_slice(&bytes)
        .map_err(|e| server_err(format!("parsing login body: {e}")))?;
    if let Some(u) = users.lookup(&body.token) {
        Ok(Json(serde_json::json!({
            "name": u.name,
            "role": match u.role {
                crate::auth::Role::Analyst => "analyst",
                crate::auth::Role::Reader => "reader",
            },
        })))
    } else if users.bypass() {
        Ok(Json(serde_json::json!({
            "name": "anonymous",
            "role": "analyst",
        })))
    } else {
        Err(ApiError {
            status: StatusCode::UNAUTHORIZED,
            message: "invalid token".into(),
        })
    }
}

// ─── Tags ────────────────────────────────────────────────────────
async fn list_tags_for_case_handler(
    State(state): State<SharedState>,
    Query(q): Query<CaseQuery>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(
        core::list_tags_for_case(&state, q.case_id).map_err(server_err)?,
    ))
}

async fn list_tags_for_entity_handler(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(
        core::list_tags_for_entity(&state, &id).map_err(server_err)?,
    ))
}

async fn add_tag_handler(
    State(state): State<SharedState>,
    Path((id, tag)): Path<(String, String)>,
) -> ApiResult<impl IntoResponse> {
    core::add_tag(&state, &id, &tag).map_err(server_err)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn remove_tag_handler(
    State(state): State<SharedState>,
    Path((id, tag)): Path<(String, String)>,
) -> ApiResult<impl IntoResponse> {
    core::remove_tag(&state, &id, &tag).map_err(server_err)?;
    Ok(StatusCode::NO_CONTENT)
}

// ─── Snapshot / restore ─────────────────────────────────────────
async fn snapshot_entity_handler(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(core::snapshot_entity(&state, &id).map_err(server_err)?))
}

#[derive(Debug, Deserialize)]
pub struct RestoreBody {
    pub snapshots: Vec<core::EntitySnapshot>,
}

async fn restore_entities_handler(
    State(state): State<SharedState>,
    Json(body): Json<RestoreBody>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(
        core::restore_entities(&state, &body.snapshots).map_err(server_err)?,
    ))
}

// ─── Plugins ────────────────────────────────────────────────────
#[derive(Debug, Deserialize)]
pub struct InstallDirBody {
    pub source_dir: String,
}

#[derive(Debug, Deserialize)]
pub struct InstallTarballBody {
    pub bytes_base64: String,
}

#[derive(Debug, Deserialize)]
pub struct BootstrapVenvBody {
    #[serde(default)]
    pub python_bin: Option<String>,
}

async fn install_plugin_dir_handler(
    State(state): State<SharedState>,
    Json(body): Json<InstallDirBody>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(
        core::install_plugin_from_directory(&state, &body.source_dir).map_err(server_err)?,
    ))
}

async fn install_plugin_tarball_handler(
    State(state): State<SharedState>,
    Json(body): Json<InstallTarballBody>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(
        core::install_plugin_from_tarball(&state, &body.bytes_base64).map_err(server_err)?,
    ))
}

async fn uninstall_plugin_handler(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> ApiResult<impl IntoResponse> {
    core::uninstall_plugin(&state, &id).map_err(server_err)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn bootstrap_venv_handler(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(body): Json<BootstrapVenvBody>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(
        core::bootstrap_plugin_venv(&state, &id, body.python_bin.as_deref()).map_err(server_err)?,
    ))
}

// ─── Notifications / webhooks / typed edges / backup ───────────

#[derive(Debug, Deserialize)]
pub struct PatchEdgeRelationshipBody {
    pub relationship_type: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RestoreBackupBody {
    pub bytes_base64: String,
}

async fn list_notifications_handler(
    State(state): State<SharedState>,
    Path(id): Path<i64>,
    Query(q): Query<RunsQuery>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(
        core::list_notifications(&state, id, q.limit.unwrap_or(100)).map_err(server_err)?,
    ))
}

async fn unread_count_handler(
    State(state): State<SharedState>,
    Path(id): Path<i64>,
) -> ApiResult<impl IntoResponse> {
    let n = core::unread_notification_count(&state, id).map_err(server_err)?;
    Ok(Json(serde_json::json!({ "unread": n })))
}

async fn mark_read_handler(
    State(state): State<SharedState>,
    Path(id): Path<i64>,
) -> ApiResult<impl IntoResponse> {
    core::mark_notifications_read(&state, id).map_err(server_err)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn get_webhooks_handler(State(state): State<SharedState>) -> impl IntoResponse {
    Json(core::get_webhook_config(&state))
}

async fn set_webhooks_handler(
    State(state): State<SharedState>,
    Json(body): Json<crate::webhooks::WebhookConfig>,
) -> ApiResult<impl IntoResponse> {
    core::set_webhook_config(&state, body).map_err(server_err)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn test_webhooks_handler(
    State(state): State<SharedState>,
) -> ApiResult<impl IntoResponse> {
    let n = core::test_webhooks(&state).await.map_err(server_err)?;
    Ok(Json(serde_json::json!({ "fired": n })))
}

async fn patch_edge_relationship_handler(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(body): Json<PatchEdgeRelationshipBody>,
) -> ApiResult<impl IntoResponse> {
    core::update_edge_relationship(&state, &id, body.relationship_type.as_deref())
        .map_err(server_err)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn export_backup_handler(
    State(state): State<SharedState>,
    Path(id): Path<i64>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(core::export_case_backup(&state, id).map_err(server_err)?))
}

async fn restore_backup_handler(
    State(state): State<SharedState>,
    Json(body): Json<RestoreBackupBody>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(
        core::restore_case_backup(&state, &body.bytes_base64).map_err(server_err)?,
    ))
}

// ─── Saved searches + machine reload ───────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreateSavedSearchBody {
    pub name: String,
    pub predicate: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateSavedSearchBody {
    pub name: String,
    pub predicate: String,
}

async fn list_saved_searches_handler(
    State(state): State<SharedState>,
    Path(id): Path<i64>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(core::list_saved_searches(&state, id).map_err(server_err)?))
}

async fn create_saved_search_handler(
    State(state): State<SharedState>,
    Path(case_id): Path<i64>,
    Json(body): Json<CreateSavedSearchBody>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(
        core::create_saved_search(&state, case_id, &body.name, &body.predicate)
            .map_err(server_err)?,
    ))
}

async fn patch_saved_search_handler(
    State(state): State<SharedState>,
    Path(id): Path<i64>,
    Json(body): Json<UpdateSavedSearchBody>,
) -> ApiResult<impl IntoResponse> {
    core::update_saved_search(&state, id, &body.name, &body.predicate)
        .map_err(server_err)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_saved_search_handler(
    State(state): State<SharedState>,
    Path(id): Path<i64>,
) -> ApiResult<impl IntoResponse> {
    core::delete_saved_search(&state, id).map_err(server_err)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
pub struct CreateSnapshotBody {
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct DiffSnapshotQuery {
    /// Optional other-snapshot id. When omitted, diff is against the
    /// case's live state.
    pub other: Option<i64>,
}

async fn list_snapshots_handler(
    State(state): State<SharedState>,
    Path(id): Path<i64>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(
        core::list_case_snapshots(&state, id).map_err(server_err)?,
    ))
}

async fn create_snapshot_handler(
    State(state): State<SharedState>,
    Path(case_id): Path<i64>,
    Json(body): Json<CreateSnapshotBody>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(
        core::create_case_snapshot(&state, case_id, &body.name).map_err(server_err)?,
    ))
}

async fn delete_snapshot_handler(
    State(state): State<SharedState>,
    Path(id): Path<i64>,
) -> ApiResult<impl IntoResponse> {
    core::delete_case_snapshot(&state, id).map_err(server_err)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn diff_snapshot_handler(
    State(state): State<SharedState>,
    Path(id): Path<i64>,
    axum::extract::Query(q): axum::extract::Query<DiffSnapshotQuery>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(
        core::diff_case_snapshots(&state, id, q.other).map_err(server_err)?,
    ))
}

async fn reload_machines_handler(
    State(state): State<SharedState>,
) -> impl IntoResponse {
    let n = core::reload_machines(&state);
    Json(serde_json::json!({ "count": n }))
}

// ─── Geo + SSE ─────────────────────────────────────────────────

async fn export_geojson_handler(
    State(state): State<SharedState>,
    Path(id): Path<i64>,
) -> ApiResult<impl IntoResponse> {
    let body = core::export_case_geojson(&state, id).map_err(server_err)?;
    Ok((
        [
            ("content-type", "application/geo+json"),
            (
                "content-disposition",
                "attachment; filename=\"maltrace-case.geojson\"",
            ),
        ],
        body,
    )
        .into_response())
}

async fn export_kml_handler(
    State(state): State<SharedState>,
    Path(id): Path<i64>,
) -> ApiResult<impl IntoResponse> {
    let body = core::export_case_kml(&state, id).map_err(server_err)?;
    Ok((
        [
            ("content-type", "application/vnd.google-earth.kml+xml"),
            (
                "content-disposition",
                "attachment; filename=\"maltrace-case.kml\"",
            ),
        ],
        body,
    )
        .into_response())
}

/// Server-Sent Events stream of every mutation event published to the
/// in-process bus. Subscribed once per browser tab; auto-reconnects on
/// the client side (EventSource is built-in). Connection is kept alive
/// with a heartbeat comment every 25s (under most reverse-proxy
/// idle-timeout cutoffs).
async fn events_stream_handler(
    State(state): State<SharedState>,
) -> impl IntoResponse {
    use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
    use futures::stream::Stream;
    use std::convert::Infallible;
    use std::pin::Pin;
    use std::task::{Context as TaskContext, Poll};
    use std::time::Duration;
    use tokio_stream::wrappers::BroadcastStream;
    use tokio_stream::StreamExt;

    let rx = state.event_bus.subscribe();
    // BroadcastStream surfaces `Lagged(n)` errors when a slow consumer
    // misses N events; we map them to a synthetic Heartbeat so the
    // frontend just triggers its standard re-fetch and we don't bubble
    // a JSON-parse error to it.
    let stream: Pin<Box<dyn Stream<Item = Result<SseEvent, Infallible>> + Send>> = Box::pin(
        BroadcastStream::new(rx).map(|item| {
            let evt = match item {
                Ok(e) => e,
                Err(_) => crate::event_bus::Event::Heartbeat,
            };
            let json = serde_json::to_string(&evt).unwrap_or_else(|_| "{}".into());
            Ok(SseEvent::default().data(json))
        }),
    );

    // A second stream emits a comment ":heartbeat" every 25s so the
    // connection survives proxies. Wrap both via KeepAlive.
    let _ = Duration::from_secs(25);
    let _ = (TaskContext::from_waker, Poll::<()>::Pending);
    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(25)).text("heartbeat"))
}
