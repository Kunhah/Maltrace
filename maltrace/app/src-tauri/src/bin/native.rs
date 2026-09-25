//! maltrace — native GPU-rendered frontend (egui/eframe, OpenGL via glow).
//!
//! Links the `app_lib` crate directly: no WebView, no HTTP, no IPC. The graph
//! is painted straight through the system OpenGL driver, which works on
//! hardware where the Tauri WebKitGTK webview's DMABUF path fails (notably
//! NVIDIA + Wayland). The core DB/query/mutation functions are the exact same
//! ones the Tauri commands and the axum server call.
//!
//! Run:  cargo run --features native --bin maltrace-native
//!
//! This is the first vertical slice: load the current case and render an
//! interactive graph (pan / zoom / drag / hover-focus / select). Inspector,
//! transforms and the other views come in later passes.

use std::collections::{HashMap, HashSet};

use app_lib::{AppState, SharedState};
use eframe::egui;

const NODE_R: f32 = 18.0; // node radius in world units

fn main() -> eframe::Result<()> {
    let data_dir = app_lib::server_data_dir();
    let state = app_lib::open_state_public(&data_dir)
        .expect("maltrace: failed to open application state / database");

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1400.0, 900.0])
            .with_min_inner_size([720.0, 480.0])
            .with_transparent(false)
            .with_title("maltrace — native"),
        ..Default::default()
    };

    eframe::run_native(
        "maltrace-native",
        options,
        Box::new(|cc| {
            setup_style(&cc.egui_ctx);
            Ok(Box::new(MaltraceApp::new(state)))
        }),
    )
}

/// Dark theme tuned to roughly match the web build (deep navy surfaces, teal
/// accent, soft rounding) so the native app reads as the same product.
/// Shared palette mirroring the React build's `:root` CSS variables so the
/// native UI reads as the same product. Centralised here so every component
/// can pull the exact same colours instead of scattering hex literals.
mod pal {
    use eframe::egui::Color32;
    // Surfaces — deepest → highest.
    pub const BG0: Color32 = Color32::from_rgb(0x05, 0x08, 0x0f);
    pub const BG1: Color32 = Color32::from_rgb(0x0a, 0x0f, 0x1c);
    pub const BG2: Color32 = Color32::from_rgb(0x13, 0x1b, 0x30);
    pub const BG3: Color32 = Color32::from_rgb(0x1e, 0x29, 0x44);
    // Borders.
    pub const BORDER: Color32 = Color32::from_rgb(0x25, 0x2f, 0x44);
    pub const BORDER_STRONG: Color32 = Color32::from_rgb(0x33, 0x40, 0x59);
    // Text.
    pub const TEXT: Color32 = Color32::from_rgb(0xe6, 0xed, 0xf7);
    pub const TEXT_SOFT: Color32 = Color32::from_rgb(0xb6, 0xc2, 0xd6);
    pub const MUTED: Color32 = Color32::from_rgb(0x7a, 0x8a, 0xa3);
    pub const MUTED2: Color32 = Color32::from_rgb(0x54, 0x62, 0x7a);
    // Accents.
    pub const ACCENT: Color32 = Color32::from_rgb(0x5e, 0xea, 0xd4);
    pub const INFO: Color32 = Color32::from_rgb(0x60, 0xa5, 0xfa);
    pub const SUCCESS: Color32 = Color32::from_rgb(0x34, 0xd3, 0x99);
    pub const WARNING: Color32 = Color32::from_rgb(0xfb, 0xbf, 0x24);
    pub const DANGER: Color32 = Color32::from_rgb(0xf8, 0x71, 0x71);
}

fn setup_style(ctx: &egui::Context) {
    use egui::{Color32, Rounding, Stroke};
    let mut v = egui::Visuals::dark();

    v.panel_fill = pal::BG1;
    v.window_fill = pal::BG2;
    v.window_stroke = Stroke::new(1.0, pal::BORDER_STRONG);
    v.extreme_bg_color = pal::BG0; // text-edit / scroll backgrounds
    v.faint_bg_color = Color32::from_rgb(0x10, 0x17, 0x29); // striped grids
    v.code_bg_color = pal::BG0;
    v.override_text_color = Some(pal::TEXT);
    v.hyperlink_color = pal::ACCENT;
    v.selection.bg_fill = fade(pal::ACCENT, 0.28);
    v.selection.stroke = Stroke::new(1.0, pal::ACCENT);

    let r_sm = Rounding::same(6.0);
    let r_md = Rounding::same(10.0);

    // Non-interactive: panel/label backgrounds + the faint separators.
    v.widgets.noninteractive.bg_fill = pal::BG2;
    v.widgets.noninteractive.weak_bg_fill = pal::BG2;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, pal::BORDER);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, pal::TEXT_SOFT);
    v.widgets.noninteractive.rounding = r_sm;

    // Inactive buttons/inputs — subtle raised surface with a hairline border.
    v.widgets.inactive.bg_fill = pal::BG3;
    v.widgets.inactive.weak_bg_fill = Color32::from_rgb(0x17, 0x20, 0x39);
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, pal::BORDER);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, pal::TEXT_SOFT);
    v.widgets.inactive.rounding = r_sm;
    v.widgets.inactive.expansion = 0.0;

    // Hovered — lift toward the accent.
    v.widgets.hovered.bg_fill = Color32::from_rgb(0x26, 0x33, 0x52);
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(0x1f, 0x2b, 0x49);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, fade(pal::ACCENT, 0.45));
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, pal::TEXT);
    v.widgets.hovered.rounding = r_sm;
    v.widgets.hovered.expansion = 1.0;

    // Active/pressed — accent-tinted.
    v.widgets.active.bg_fill = Color32::from_rgb(0x2c, 0x3b, 0x5e);
    v.widgets.active.weak_bg_fill = Color32::from_rgb(0x24, 0x32, 0x53);
    v.widgets.active.bg_stroke = Stroke::new(1.0, fade(pal::ACCENT, 0.7));
    v.widgets.active.fg_stroke = Stroke::new(1.5, pal::TEXT);
    v.widgets.active.rounding = r_sm;
    v.widgets.active.expansion = 1.0;

    // Open (combo/menu) state.
    v.widgets.open.bg_fill = pal::BG3;
    v.widgets.open.bg_stroke = Stroke::new(1.0, pal::BORDER_STRONG);
    v.widgets.open.fg_stroke = Stroke::new(1.0, pal::TEXT);
    v.widgets.open.rounding = r_sm;

    v.window_rounding = r_md;
    v.window_shadow = egui::epaint::Shadow {
        offset: egui::vec2(0.0, 12.0),
        blur: 40.0,
        spread: 0.0,
        color: Color32::from_black_alpha(140),
    };
    v.popup_shadow = egui::epaint::Shadow {
        offset: egui::vec2(0.0, 6.0),
        blur: 20.0,
        spread: 0.0,
        color: Color32::from_black_alpha(120),
    };
    v.menu_rounding = r_md;
    ctx.set_visuals(v);

    let mut style = (*ctx.style()).clone();
    // Generous, web-like spacing.
    style.spacing.item_spacing = egui::vec2(8.0, 7.0);
    style.spacing.button_padding = egui::vec2(11.0, 6.0);
    style.spacing.menu_margin = egui::Margin::same(8.0);
    style.spacing.window_margin = egui::Margin::same(12.0);
    style.spacing.interact_size.y = 26.0;
    style.spacing.scroll = egui::style::ScrollStyle::thin();
    style.visuals.clip_rect_margin = 2.0;
    // Hand cursor over every clickable widget (web-like affordance).
    style.interaction.selectable_labels = true;
    style.visuals.interact_cursor = Some(egui::CursorIcon::PointingHand);

    // Type scale — slightly larger body text, clear heading hierarchy, mono
    // for code. Mirrors the React Inter/JetBrains-Mono pairing in spirit.
    use egui::{FontFamily, FontId, TextStyle};
    style.text_styles = [
        (TextStyle::Heading, FontId::new(17.0, FontFamily::Proportional)),
        (TextStyle::Body, FontId::new(13.0, FontFamily::Proportional)),
        (TextStyle::Button, FontId::new(13.0, FontFamily::Proportional)),
        (TextStyle::Small, FontId::new(11.0, FontFamily::Proportional)),
        (TextStyle::Monospace, FontId::new(12.0, FontFamily::Monospace)),
    ]
    .into();
    ctx.set_style(style);
}

/// One renderable graph node (an entity).
struct NodeView {
    id: String,
    label: String,
    type_label: String,
    color: egui::Color32,
    pos: egui::Pos2, // world coordinates (same space as DB pos_x/pos_y)
    locked: bool,
    bookmarked: bool,
    notes: String,
    /// SpiderFoot event type this entity came in as, if any. Used (with the
    /// canonical type's default event) to decide which transforms apply.
    source_event: Option<String>,
    /// (lat, lng) if this entity exposes a coordinate — for the map view.
    geo: Option<(f64, f64)>,
    /// Creation time (unix millis) — for the timeline view.
    created_at: i64,
    /// Canonical type id (e.g. "Collection") — distinct from the human
    /// `type_label` shown in the UI. Used to detect Collections to expand.
    entity_type: String,
    /// Raw `data` JSON payload from the backend (full_data / extra / collection
    /// items). Parsed lazily for the inspector's Properties section.
    data: Option<String>,
    /// Citation URL extracted from the originating event's `extra.url`.
    source_url: Option<String>,
}

/// Which case view is showing.
#[derive(PartialEq, Clone, Copy)]
enum View {
    Graph,
    Map,
    Timeline,
}

/// Command-palette actions (Ctrl+K).
#[derive(Clone, Copy)]
enum Action {
    ViewGraph,
    ViewMap,
    ViewTimeline,
    Arrange,
    Fit,
    ZoomIn,
    ZoomOut,
    AddEntity,
    Reload,
    ClearSelection,
    DeleteSelected,
}

const PALETTE_ACTIONS: &[(&str, Action)] = &[
    ("Graph view", Action::ViewGraph),
    ("Map view", Action::ViewMap),
    ("Timeline view", Action::ViewTimeline),
    ("Arrange (auto-layout)", Action::Arrange),
    ("Fit to view", Action::Fit),
    ("Zoom in", Action::ZoomIn),
    ("Zoom out", Action::ZoomOut),
    ("Add entity", Action::AddEntity),
    ("Reload graph", Action::Reload),
    ("Clear selection", Action::ClearSelection),
    ("Delete selected", Action::DeleteSelected),
];

/// A transform parameter, flattened from `app_lib::ParameterSpec` into a shape
/// the dialog can edit without depending on the source type being `Clone`.
struct ParamField {
    name: String,
    kind: ParamKind,
    choices: Vec<String>,
    required: bool,
    description: String,
}

#[derive(PartialEq, Clone, Copy)]
enum ParamKind {
    Text,
    Number,
    Bool,
    Choice,
}

/// Open transform-params dialog: collect values, then run with params.
struct ParamsDialog {
    node: usize,
    transform_id: String,
    transform_name: String,
    fields: Vec<ParamField>,
    values: std::collections::HashMap<String, String>,
}

/// One renderable edge.
struct EdgeView {
    id: String,
    source: usize, // index into `nodes`
    target: usize,
    label: String,
    relationship_type: Option<String>,
}

/// A transform compatible with the right-clicked entity, flattened for the
/// pivot menu so it can render a readable name + description without holding
/// a borrow of the full catalogue.
#[derive(Clone)]
struct CompatTransform {
    id: String,
    name: String,
    description: String,
    source: String,
    produces: Vec<String>,
}

struct MaltraceApp {
    state: SharedState,
    /// all cases (for the case-tab strip) + active-case lock flag.
    cases: Vec<app_lib::Case>,
    case_locked: bool,
    /// inline case-rename buffer: (case id, text). None when not renaming.
    case_rename: Option<(i64, String)>,
    case_id: i64,
    case_name: String,
    nodes: Vec<NodeView>,
    edges: Vec<EdgeView>,
    /// node index → indices of adjacent nodes (for hover-focus).
    neighbors: Vec<HashSet<usize>>,

    // viewport transform: screen = world * zoom + offset
    offset: egui::Vec2,
    zoom: f32,
    fitted: bool,

    // interaction state
    dragging_node: Option<usize>,
    panning: bool,
    /// All selected node indices (multi-select). `primary` is the one the
    /// inspector edits (the most recently clicked).
    selected: HashSet<usize>,
    primary: Option<usize>,
    /// shift-drag box-select origin (screen coords), while active.
    box_start: Option<egui::Pos2>,
    status: String,
    /// entity_type ids the user toggled off in the filter strip.
    hidden_types: HashSet<String>,

    /// All canonical type ids+labels, for the add-entity type picker.
    types: Vec<(String, String)>,
    /// Inspector edit buffers, valid for `editing_for` (the selected node).
    editing_for: Option<usize>,
    label_buf: String,
    notes_buf: String,
    tags: Vec<String>,
    tag_input: String,
    /// provenance + run-history for the inspected entity (web parity).
    /// Loaded when the primary selection changes (see `sync_edit_buffers`).
    prov: Option<app_lib::EntityProvenance>,
    runs: Vec<app_lib::TransformRun>,
    /// add-entity form
    add_open: bool,
    add_type: String,
    add_label: String,
    /// find-on-graph query
    search: String,

    // ── transforms (spiderfoot / brazuca / … sidecars) ──
    rt: tokio::runtime::Runtime,
    /// canonical type id → its default SpiderFoot event (for compatibility).
    sf_event_by_type: HashMap<String, String>,
    /// all available transforms, loaded async from the sidecars.
    transforms: Vec<app_lib::TransformMeta>,
    transforms_rx: Option<std::sync::mpsc::Receiver<Vec<app_lib::TransformMeta>>>,
    /// per-sidecar load status, so failed sidecars surface instead of
    /// silently vanishing from the catalogue (parity with web #1).
    sidecar_status: Vec<app_lib::SidecarStatus>,
    sidecar_status_rx: Option<std::sync::mpsc::Receiver<Vec<app_lib::SidecarStatus>>>,
    /// pivot-menu UI state: free-text filter + active source chip ("" = all).
    ctx_filter: String,
    ctx_source: String,
    /// when true, the OSINT-tool picker is shown inline in the inspector
    /// panel (instead of as a floating popup) for the primary selection.
    tools_inline: bool,
    /// right-click pivot menu: (node index, screen anchor).
    ctx_menu: Option<(usize, egui::Pos2)>,
    /// in-flight transform run → (count of new entities | error).
    run_rx: Option<std::sync::mpsc::Receiver<Result<usize, String>>>,
    running: bool,
    /// open transform-params dialog, if any.
    params_dialog: Option<ParamsDialog>,
    /// active case view (graph / map / timeline).
    view: View,
    /// timeline view pan/zoom over time (pixels-per-ms, screen x offset).
    tl_scale: f32,
    tl_offset: f32,
    /// command palette (Ctrl+K).
    palette_open: bool,
    palette_query: String,

    // ── panels (web parity) ──
    /// run-log drawer: recent transform runs for this case.
    runlog_open: bool,
    runlog: Vec<app_lib::TransformRun>,
    /// machines panel.
    machines_open: bool,
    machines: Vec<app_lib::MachineMeta>,
    machines_loaded: bool,
    machine_rx: Option<std::sync::mpsc::Receiver<Vec<app_lib::MachineMeta>>>,
    /// settings panel + which tab.
    settings_open: bool,
    settings_tab: SettingsTab,
    secrets: Vec<app_lib::SecretInfo>,
    secret_name: String,
    secret_value: String,
    new_type_label: String,
    new_type_kind: String,
    new_type_color: String,
    /// settings: network + webhooks buffers (loaded when the tab opens).
    proxy_buf: String,
    rl_concurrent: String,
    rl_interval: String,
    webhooks_buf: String,
    settings_net_loaded: bool,
    /// AI analyst panel.
    ai_open: bool,
    ai_status: Option<app_lib::AiStatus>,
    ai_input: String,
    ai_history: Vec<app_lib::ChatTurn>,
    ai_include_graph: bool,
    ai_busy: bool,
    ai_rx: Option<std::sync::mpsc::Receiver<Result<app_lib::ChatReply, String>>>,
    /// audit-log panel.
    audit_open: bool,
    audit: Vec<app_lib::AuditEvent>,
    /// bookmarks panel.
    bookmarks_open: bool,
    /// import dialog (bulk paste).
    import_open: bool,
    import_type: String,
    import_text: String,
    /// export menu open flag.
    export_open: bool,
    /// last graph-canvas rect, so panels can re-center on a node.
    last_canvas: egui::Rect,
    /// sticky-note annotations on the canvas (web AnnotationEditor).
    annotations: Vec<app_lib::Annotation>,
    /// index into `annotations` currently being dragged.
    dragging_annotation: Option<usize>,
    /// inline annotation editor: (annotation id, text buffer).
    editing_annotation: Option<(String, String)>,
    /// selected edge (index into `edges`) for the edge inspector.
    selected_edge: Option<usize>,
    /// relationship-type edit buffer for the selected edge.
    edge_rel_buf: String,
    /// snapshots panel.
    snapshots_open: bool,
    snapshots: Vec<app_lib::CaseSnapshotMeta>,
    snapshot_name: String,
    /// saved-searches panel (label-filter presets in native).
    searches_open: bool,
    saved_searches: Vec<app_lib::SavedSearch>,
    search_save_name: String,
    /// notifications.
    notifications_open: bool,
    notifications: Vec<app_lib::Notification>,
    unread_count: i64,
    /// AI tool-use toggle (let Claude run transforms autonomously).
    ai_allow_tools: bool,
    /// plugins panel.
    plugins_open: bool,
    plugins: Vec<app_lib::PluginSummary>,
    /// GraphML import-by-path field.
    graphml_path: String,
    /// add-image-by-path field (in the Add window).
    image_path: String,
    /// cached preview texture for the inspected Image entity: (entity id, tex).
    preview: Option<(String, egui::TextureHandle)>,
    /// in-flight AI image analysis → result text | error.
    analyze_rx: Option<std::sync::mpsc::Receiver<(String, Result<String, String>)>>,
    analyzing: bool,
    /// last analysis result (entity id it belongs to + text).
    analysis_result: Option<(String, String)>,
    /// image gallery panel.
    gallery_open: bool,
    /// decoded gallery thumbnails, keyed by entity id (lazy).
    gallery_thumbs: std::collections::HashMap<String, egui::TextureHandle>,
    /// the image currently enlarged in the gallery's detail pane.
    gallery_selected: Option<String>,
    /// pending OPSEC consent: the image entity id awaiting the user's OK to
    /// upload it off-device to the AI. None when no dialog is showing.
    confirm_analyze: Option<String>,
    /// once the user accepts the upload warning, skip it for the rest of the
    /// session (per-process, never persisted — re-warn on every launch).
    ai_upload_consented: bool,
    /// when true the right inspector panel is collapsed to a thin reopen rail
    /// (so it's always recoverable even with nothing selected).
    inspector_collapsed: bool,
}

#[derive(PartialEq, Clone, Copy)]
enum SettingsTab {
    Secrets,
    Types,
    Network,
    Webhooks,
}

impl MaltraceApp {
    fn new(state: SharedState) -> Self {
        let mut app = Self {
            state,
            cases: Vec::new(),
            case_locked: false,
            case_rename: None,
            case_id: 1,
            case_name: String::new(),
            nodes: Vec::new(),
            edges: Vec::new(),
            neighbors: Vec::new(),
            offset: egui::Vec2::ZERO,
            zoom: 1.0,
            fitted: false,
            dragging_node: None,
            panning: false,
            selected: HashSet::new(),
            primary: None,
            box_start: None,
            status: String::new(),
            hidden_types: HashSet::new(),
            types: Vec::new(),
            editing_for: None,
            label_buf: String::new(),
            notes_buf: String::new(),
            tags: Vec::new(),
            tag_input: String::new(),
            prov: None,
            runs: Vec::new(),
            add_open: false,
            add_type: "Domain".to_string(),
            add_label: String::new(),
            search: String::new(),
            rt: tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("tokio runtime"),
            sf_event_by_type: HashMap::new(),
            transforms: Vec::new(),
            transforms_rx: None,
            sidecar_status: Vec::new(),
            sidecar_status_rx: None,
            ctx_filter: String::new(),
            ctx_source: String::new(),
            tools_inline: false,
            ctx_menu: None,
            run_rx: None,
            running: false,
            params_dialog: None,
            view: View::Graph,
            tl_scale: 0.0,
            tl_offset: 0.0,
            palette_open: false,
            palette_query: String::new(),
            runlog_open: false,
            runlog: Vec::new(),
            machines_open: false,
            machines: Vec::new(),
            machines_loaded: false,
            machine_rx: None,
            settings_open: false,
            settings_tab: SettingsTab::Secrets,
            secrets: Vec::new(),
            secret_name: String::new(),
            secret_value: String::new(),
            new_type_label: String::new(),
            new_type_kind: "generic".to_string(),
            new_type_color: "#64748b".to_string(),
            proxy_buf: String::new(),
            rl_concurrent: String::new(),
            rl_interval: String::new(),
            webhooks_buf: String::new(),
            settings_net_loaded: false,
            ai_open: false,
            ai_status: None,
            ai_input: String::new(),
            ai_history: Vec::new(),
            ai_include_graph: true,
            ai_busy: false,
            ai_rx: None,
            audit_open: false,
            audit: Vec::new(),
            bookmarks_open: false,
            import_open: false,
            import_type: "Domain".to_string(),
            import_text: String::new(),
            export_open: false,
            last_canvas: egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0)),
            annotations: Vec::new(),
            dragging_annotation: None,
            editing_annotation: None,
            selected_edge: None,
            edge_rel_buf: String::new(),
            snapshots_open: false,
            snapshots: Vec::new(),
            snapshot_name: String::new(),
            searches_open: false,
            saved_searches: Vec::new(),
            search_save_name: String::new(),
            notifications_open: false,
            notifications: Vec::new(),
            unread_count: 0,
            ai_allow_tools: false,
            plugins_open: false,
            plugins: Vec::new(),
            graphml_path: String::new(),
            image_path: String::new(),
            preview: None,
            analyze_rx: None,
            analyzing: false,
            analysis_result: None,
            gallery_open: false,
            gallery_thumbs: std::collections::HashMap::new(),
            gallery_selected: None,
            confirm_analyze: None,
            ai_upload_consented: false,
            inspector_collapsed: false,
        };
        app.reload();
        app.load_transforms();
        app
    }

    /// Kick off async sidecar transform enumeration (spiderfoot, brazuca, …).
    /// Results arrive via `transforms_rx` and are picked up in `update`.
    fn load_transforms(&mut self) {
        let state = self.state.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        self.transforms_rx = Some(rx);
        self.status = "loading transforms…".into();
        self.rt.spawn(async move {
            let list = app_lib::list_transforms(&state).await.unwrap_or_default();
            let _ = tx.send(list);
        });
        // Per-sidecar load status, fetched in parallel so failed sidecars
        // surface in the UI instead of silently dropping out (web #1).
        let state2 = self.state.clone();
        let (stx, srx) = std::sync::mpsc::channel();
        self.sidecar_status_rx = Some(srx);
        self.rt.spawn(async move {
            let st = app_lib::sidecar_status(&state2).await;
            let _ = stx.send(st);
        });
    }

    /// Kick off async machine enumeration. Results arrive via `machine_rx`.
    fn load_machines(&mut self) {
        let state = self.state.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        self.machine_rx = Some(rx);
        self.rt.spawn(async move {
            let list = app_lib::list_machines(&state).await;
            let _ = tx.send(list);
        });
    }

    /// Transforms whose `consumes` accept this node's event type. Owned so
    /// callers don't hold a borrow of `self`. Sorted by friendly name.
    fn compatible_transforms(&self, node: usize) -> Vec<CompatTransform> {
        let nv = &self.nodes[node];
        let ev = nv
            .source_event
            .clone()
            .or_else(|| self.sf_event_by_type.get(&nv.type_label).cloned())
            .unwrap_or_default();
        let mut out: Vec<CompatTransform> = self
            .transforms
            .iter()
            .filter(|t| t.consumes.iter().any(|c| c == &ev))
            .map(|t| CompatTransform {
                id: t.id.clone(),
                name: if t.name.is_empty() { t.id.clone() } else { t.name.clone() },
                description: t.description.clone(),
                source: t.source.clone(),
                produces: t.produces.clone(),
            })
            .collect();
        out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        out
    }

    /// Render the OSINT-tool picker (filter + source chips + transform rows)
    /// for `node` into `ui`. Returns the chosen transform id (if any) and
    /// whether the picker asked to close. Shared by the inline inspector
    /// picker and the right-click popup so they can't drift.
    fn tools_picker_ui(&mut self, ui: &mut egui::Ui, node: usize) -> (Option<String>, bool) {
        let compat = self.compatible_transforms(node);
        let mut source_counts: Vec<(String, usize)> = Vec::new();
        for t in &compat {
            let s = if t.source.is_empty() { "unknown" } else { t.source.as_str() };
            if let Some(e) = source_counts.iter_mut().find(|(k, _)| k == s) {
                e.1 += 1;
            } else {
                source_counts.push((s.to_string(), 1));
            }
        }
        source_counts.sort_by(|a, b| a.0.cmp(&b.0));
        if !self.ctx_source.is_empty()
            && !source_counts.iter().any(|(k, _)| k == &self.ctx_source)
        {
            self.ctx_source.clear();
        }
        let filt = self.ctx_filter.trim().to_lowercase();
        let visible: Vec<&CompatTransform> = compat
            .iter()
            .filter(|t| {
                self.ctx_source.is_empty()
                    || t.source == self.ctx_source
                    || (self.ctx_source == "unknown" && t.source.is_empty())
            })
            .filter(|t| {
                filt.is_empty()
                    || t.name.to_lowercase().contains(&filt)
                    || t.id.to_lowercase().contains(&filt)
            })
            .collect();
        let mut chosen: Option<String> = None;
        let mut close = false;
        if self.running {
            ui.label("a transform is already running…");
        } else if self.transforms.is_empty() {
            ui.label(
                egui::RichText::new("Transforms still loading — sidecars starting up…").weak(),
            );
        } else if compat.is_empty() {
            ui.label(
                egui::RichText::new("No compatible transforms for this entity type.").weak(),
            );
        } else {
            ui.add(
                egui::TextEdit::singleline(&mut self.ctx_filter)
                    .hint_text("Filter tools…")
                    .desired_width(f32::INFINITY),
            );
            if source_counts.len() > 1 {
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .selectable_label(self.ctx_source.is_empty(), format!("All ({})", compat.len()))
                        .clicked()
                    {
                        self.ctx_source.clear();
                    }
                    for (src, n) in &source_counts {
                        let sel = &self.ctx_source == src;
                        if ui.selectable_label(sel, format!("{src} ({n})")).clicked() {
                            self.ctx_source = if sel { String::new() } else { src.clone() };
                        }
                    }
                });
            }
            ui.separator();
            if visible.is_empty() {
                ui.label(egui::RichText::new("No tools match the filter.").weak());
            }
            egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                for t in &visible {
                    let resp = ui
                        .add(egui::Button::new(transform_row_text(t)).wrap())
                        .on_hover_text(if t.description.is_empty() {
                            t.id.clone()
                        } else {
                            t.description.clone()
                        });
                    if resp.clicked() {
                        chosen = Some(t.id.clone());
                        close = true;
                    }
                }
            });
        }
        (chosen, close)
    }

    /// Execute a command-palette action.
    fn exec_action(&mut self, a: Action) {
        match a {
            Action::ViewGraph => self.view = View::Graph,
            Action::ViewMap => self.view = View::Map,
            Action::ViewTimeline => self.view = View::Timeline,
            Action::Arrange => self.arrange(),
            Action::Fit => self.fitted = false,
            Action::ZoomIn => self.zoom = (self.zoom * 1.2).min(4.0),
            Action::ZoomOut => self.zoom = (self.zoom / 1.2).max(0.1),
            Action::AddEntity => self.add_open = true,
            Action::Reload => self.reload(),
            Action::ClearSelection => {
                self.selected.clear();
                self.primary = None;
            }
            Action::DeleteSelected => {
                let ids: Vec<String> =
                    self.selected.iter().map(|&i| self.nodes[i].id.clone()).collect();
                for id in ids {
                    let _ = app_lib::delete_entity(&self.state, &id, Some("native"));
                }
                self.reload();
            }
        }
    }

    /// Decide how to run `transform_id` on `node`: transforms with parameters
    /// open the params dialog first; the rest run straight away.
    fn begin_transform(&mut self, node: usize, transform_id: String) {
        let (fields, name, values) = {
            match self.transforms.iter().find(|t| t.id == transform_id) {
                None => (Vec::new(), transform_id.clone(), std::collections::HashMap::new()),
                Some(m) => {
                    let mut fields = Vec::new();
                    let mut values = std::collections::HashMap::new();
                    for p in &m.parameters {
                        let kind = match &p.ty {
                            app_lib::ParameterType::Number => ParamKind::Number,
                            app_lib::ParameterType::Boolean => ParamKind::Bool,
                            app_lib::ParameterType::Choice => ParamKind::Choice,
                            app_lib::ParameterType::String => ParamKind::Text,
                        };
                        values.insert(p.name.clone(), default_value_string(&p.default, &kind));
                        fields.push(ParamField {
                            name: p.name.clone(),
                            kind,
                            choices: p.choices.clone(),
                            required: p.required,
                            description: p.description.clone(),
                        });
                    }
                    (fields, m.name.clone(), values)
                }
            }
        };
        if fields.is_empty() {
            self.run_transform(node, transform_id, None);
        } else {
            self.params_dialog = Some(ParamsDialog {
                node,
                transform_id,
                transform_name: name,
                fields,
                values,
            });
        }
    }

    /// Run `transform_id` against node `node` on a background task, optionally
    /// with a JSON params object; the result count (or error) comes back via
    /// `run_rx`.
    fn run_transform(&mut self, node: usize, transform_id: String, params_json: Option<String>) {
        if self.running {
            return;
        }
        // Run on the whole selection when the pivoted node is part of a
        // multi-selection; otherwise just the one node.
        let ids: Vec<String> = if self.selected.len() > 1 && self.selected.contains(&node) {
            self.selected.iter().map(|&i| self.nodes[i].id.clone()).collect()
        } else {
            vec![self.nodes[node].id.clone()]
        };
        let state = self.state.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        self.run_rx = Some(rx);
        self.running = true;
        self.status = format!(
            "running {transform_id} on {} entit{}…",
            ids.len(),
            if ids.len() == 1 { "y" } else { "ies" }
        );
        self.rt.spawn(async move {
            let res = if ids.len() == 1 {
                app_lib::run_transform_with_params(&state, &ids[0], &transform_id, params_json, Some("native"))
                    .await
                    .map(|r| r.new_entities.len())
            } else {
                // batch has no params variant; selection-runs use defaults.
                app_lib::run_transform_batch(&state, &ids, &transform_id, Some("native"))
                    .await
                    .map(|v| v.iter().map(|r| r.new_entities.len()).sum())
            };
            let _ = tx.send(res);
        });
    }

    /// Run every compatible transform on one entity (web "Run all"). Results
    /// stream back via `run_rx` as a single aggregate new-entity count.
    fn begin_run_all(&mut self, node: usize) {
        if self.running {
            return;
        }
        let ids: Vec<String> =
            self.compatible_transforms(node).into_iter().map(|t| t.id).collect();
        if ids.is_empty() {
            self.status = "no compatible transforms to run".into();
            return;
        }
        let entity_id = self.nodes[node].id.clone();
        let state = self.state.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        self.run_rx = Some(rx);
        self.running = true;
        self.status = format!("running all {} transforms…", ids.len());
        self.rt.spawn(async move {
            let res = app_lib::run_all_transforms(&state, &entity_id, &ids, Some("native"))
                .await
                .map(|v| v.iter().map(|r| r.new_entities.len()).sum());
            let _ = tx.send(res);
        });
    }

    /// Switch the active case and reload its graph.
    fn switch_case(&mut self, id: i64) {
        if id == self.case_id {
            return;
        }
        self.case_id = id;
        self.selected.clear();
        self.primary = None;
        self.fitted = false;
        self.reload();
    }

    /// Run a machine on one entity (web MachinesPanel). Reuses `run_rx` to
    /// report the aggregate new-entity count, same as a transform run.
    fn begin_machine(&mut self, machine_id: String, entity_id: String) {
        if self.running {
            return;
        }
        let state = self.state.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        self.run_rx = Some(rx);
        self.running = true;
        self.status = format!("running machine {machine_id}…");
        self.rt.spawn(async move {
            let res = app_lib::run_machine(&state, &machine_id, &entity_id)
                .await
                .map(|r| r.new_entities.len());
            let _ = tx.send(res);
        });
    }

    /// Load proxy / rate-limit / webhook config into the settings buffers.
    fn load_network_settings(&mut self) {
        let proxy = app_lib::get_proxy_config(&self.state);
        self.proxy_buf = proxy.default_proxy;
        let rl = app_lib::get_rate_limit_config(&self.state);
        let d = rl.default.unwrap_or(app_lib::Limits {
            max_concurrent: 4,
            min_interval_ms: 200,
        });
        self.rl_concurrent = d.max_concurrent.to_string();
        self.rl_interval = d.min_interval_ms.to_string();
        let wh = app_lib::get_webhook_config(&self.state);
        self.webhooks_buf = wh.urls.join("\n");
        self.settings_net_loaded = true;
    }

    /// Create a sticky note at the current canvas center and open its editor.
    fn add_annotation_at_center(&mut self) {
        let center_world =
            (self.last_canvas.center() - self.offset) / self.zoom;
        match app_lib::create_annotation(
            &self.state,
            self.case_id,
            "New note",
            center_world.x as f64,
            center_world.y as f64,
            None,
        ) {
            Ok(a) => {
                self.editing_annotation = Some((a.id.clone(), a.text.clone()));
                self.annotations.push(a);
            }
            Err(e) => self.status = format!("add note failed: {e}"),
        }
    }

    /// Center the viewport on one node (used by bookmarks / find jumps).
    fn center_on(&mut self, node: usize) {
        let world = self.nodes[node].pos;
        self.offset = self.last_canvas.center().to_vec2() - world.to_vec2() * self.zoom;
    }

    /// Send the current AI input + history to Claude on a background task.
    fn begin_ai_chat(&mut self) {
        if self.ai_busy {
            return;
        }
        let msg = self.ai_input.trim().to_string();
        if msg.is_empty() {
            return;
        }
        self.ai_history.push(app_lib::ChatTurn { role: "user".into(), content: msg.clone() });
        self.ai_input.clear();
        let input = app_lib::AiChatInput {
            case_id: self.case_id,
            history: self.ai_history.clone(),
            message: msg,
            include_graph: self.ai_include_graph,
            selected_entity_id: self.primary.map(|i| self.nodes[i].id.clone()),
            allow_tools: self.ai_allow_tools,
        };
        let state = self.state.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        self.ai_rx = Some(rx);
        self.ai_busy = true;
        self.rt.spawn(async move {
            let res = app_lib::ai_chat(&state, input).await;
            let _ = tx.send(res);
        });
    }

    /// Bulk-import: one entity per line (# comments skipped), all of the
    /// chosen type. Web ImportPanel "Bulk" mode.
    fn do_bulk_import(&mut self) {
        let mut n = 0;
        let lines: Vec<String> = self
            .import_text
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .collect();
        for label in lines {
            let input = app_lib::AddEntityInput {
                case_id: self.case_id,
                entity_type: self.import_type.clone(),
                label,
                pos_x: None,
                pos_y: None,
            };
            if app_lib::add_entity(&self.state, input, Some("native")).is_ok() {
                n += 1;
            }
        }
        self.import_text.clear();
        self.import_open = false;
        self.status = format!("imported {n} entit{}", if n == 1 { "y" } else { "ies" });
        self.reload();
    }

    /// Read an image file from a path, upload it into the case (creates an
    /// Image entity), so it can be previewed and analyzed.
    fn do_add_image(&mut self) {
        let path = self.image_path.trim().to_string();
        if path.is_empty() {
            return;
        }
        let p = std::path::Path::new(&path);
        let name = p
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("image")
            .to_string();
        match std::fs::read(&path) {
            Ok(bytes) => {
                use base64::Engine;
                let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
                match app_lib::upload_file_to_case(&self.state, self.case_id, &name, &b64, Some("native")) {
                    Ok(_) => {
                        self.image_path.clear();
                        self.status = format!("added image {name}");
                    }
                    Err(e) => self.status = format!("add image failed: {e}"),
                }
            }
            Err(e) => self.status = format!("can't read {path}: {e}"),
        }
    }

    /// Decode the inspected Image entity's bytes into a GPU texture for the
    /// inspector thumbnail, caching by entity id. Best-effort: a decode
    /// failure just leaves `preview` empty (no thumbnail shown).
    fn ensure_preview(&mut self, ctx: &egui::Context, node: usize) {
        let id = self.nodes[node].id.clone();
        if self.preview.as_ref().map(|(pid, _)| pid == &id).unwrap_or(false) {
            return; // already loaded for this entity
        }
        // Resolve path from the entity's data payload (or the label as a path).
        let path = self
            .nodes[node]
            .data
            .as_deref()
            .and_then(|d| serde_json::from_str::<serde_json::Value>(d).ok())
            .and_then(|v| v.get("file_path").and_then(|p| p.as_str()).map(String::from))
            .unwrap_or_else(|| self.nodes[node].label.clone());
        let Ok(bytes) = std::fs::read(&path) else {
            self.preview = None;
            return;
        };
        let Ok(img) = image::load_from_memory(&bytes) else {
            self.preview = None;
            return;
        };
        // Downscale to a reasonable thumbnail to keep GPU memory modest.
        let thumb = img.thumbnail(480, 480).to_rgba8();
        let (w, h) = thumb.dimensions();
        let color = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &thumb);
        let tex = ctx.load_texture(format!("preview-{id}"), color, egui::TextureOptions::LINEAR);
        self.preview = Some((id, tex));
    }

    /// Ensure a gallery thumbnail texture exists for an Image entity (cached
    /// by id). Returns the handle, or None if the file can't be decoded.
    fn gallery_thumb(
        &mut self,
        ctx: &egui::Context,
        id: &str,
        path: &str,
    ) -> Option<egui::TextureHandle> {
        if let Some(t) = self.gallery_thumbs.get(id) {
            return Some(t.clone());
        }
        let bytes = std::fs::read(path).ok()?;
        let img = image::load_from_memory(&bytes).ok()?;
        let thumb = img.thumbnail(512, 512).to_rgba8();
        let (w, h) = thumb.dimensions();
        let color = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &thumb);
        let tex = ctx.load_texture(format!("gal-{id}"), color, egui::TextureOptions::LINEAR);
        self.gallery_thumbs.insert(id.to_string(), tex.clone());
        Some(tex)
    }

    /// Kick off an AI vision analysis of an Image entity on a background task.
    fn begin_analyze(&mut self, entity_id: String, save_to_notes: bool) {
        if self.analyzing {
            return;
        }
        let state = self.state.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        self.analyze_rx = Some(rx);
        self.analyzing = true;
        self.analysis_result = None;
        self.status = "analyzing image with AI…".into();
        self.rt.spawn(async move {
            let res =
                app_lib::analyze_image_entity(&state, &entity_id, "", save_to_notes, Some("native"))
                    .await;
            let _ = tx.send((entity_id, res));
        });
    }

    /// Import a GraphML file from a path typed by the user.
    fn do_graphml_import(&mut self) {
        let path = self.graphml_path.trim().to_string();
        if path.is_empty() {
            return;
        }
        match std::fs::read_to_string(&path) {
            Ok(xml) => {
                match app_lib::import_case_graphml(&self.state, self.case_id, &xml, Some("native")) {
                    Ok(r) => {
                        self.status = format!(
                            "imported {} entities · {} edges",
                            r.new_entity_count, r.new_edge_count
                        );
                        self.graphml_path.clear();
                        self.import_open = false;
                        self.reload();
                    }
                    Err(e) => self.status = format!("GraphML import failed: {e}"),
                }
            }
            Err(e) => self.status = format!("can't read {path}: {e}"),
        }
    }

    /// Export the case to a file in the home dir and report the path.
    fn do_export(&mut self, fmt: &str) {
        let res = match fmt {
            "graphml" => app_lib::export_case_graphml(&self.state, self.case_id),
            "csv" => app_lib::export_case_entities_csv(&self.state, self.case_id),
            "stix" => app_lib::export_case_stix(&self.state, self.case_id),
            "report" => app_lib::export_case_report_html(&self.state, self.case_id),
            _ => Err("unknown format".into()),
        };
        let ext = match fmt {
            "graphml" => "graphml",
            "csv" => "csv",
            "stix" => "json",
            _ => "html",
        };
        match res {
            Ok(body) => {
                let dir = dirs_home();
                let safe: String = self
                    .case_name
                    .chars()
                    .map(|c| if c.is_alphanumeric() { c } else { '_' })
                    .collect();
                let path = format!("{dir}/maltrace_{safe}.{ext}");
                match std::fs::write(&path, body) {
                    Ok(_) => self.status = format!("exported → {path}"),
                    Err(e) => self.status = format!("export write failed: {e}"),
                }
            }
            Err(e) => self.status = format!("export failed: {e}"),
        }
    }

    /// (Re)load the current case's graph from the DB via the core API.
    fn reload(&mut self) {
        let st: &AppState = &self.state;

        // Refresh the case list, then resolve the active case by id (multi-case
        // parity). Fall back to the default case on first load / if the active
        // id vanished (e.g. it was just deleted).
        self.cases = app_lib::list_cases(st).unwrap_or_default();
        let case = match self.cases.iter().find(|c| c.id == self.case_id).cloned() {
            Some(c) => c,
            None => match app_lib::current_case(st) {
                Ok(c) => c,
                Err(e) => {
                    self.status = format!("load case failed: {e}");
                    return;
                }
            },
        };
        self.case_id = case.id;
        self.case_name = case.name.clone();
        self.case_locked = case.locked;

        // entity_type → display colour, from the canonical-type ontology.
        let mut colors: HashMap<String, egui::Color32> = HashMap::new();
        let mut types: Vec<(String, String)> = Vec::new();
        self.sf_event_by_type.clear();
        for t in app_lib::list_canonical_types(st) {
            colors.insert(t.id.clone(), parse_hex(&t.color));
            types.push((t.id.clone(), t.label.clone()));
            self.sf_event_by_type.insert(t.id.clone(), t.default_sf_event.clone());
        }
        self.types = types;

        let entities = app_lib::list_entities(st, self.case_id).unwrap_or_default();
        let edges = app_lib::list_edges(st, self.case_id).unwrap_or_default();

        let n = entities.len().max(1) as f32;
        self.nodes = entities
            .iter()
            .enumerate()
            .map(|(i, e)| {
                // Fall back to a ring layout for entities without saved
                // positions so they don't all stack on the origin.
                let (x, y) = match (e.pos_x, e.pos_y) {
                    (Some(x), Some(y)) => (x as f32, y as f32),
                    _ => {
                        let a = (i as f32 / n) * std::f32::consts::TAU;
                        (a.cos() * 260.0, a.sin() * 260.0)
                    }
                };
                NodeView {
                    id: e.id.clone(),
                    label: e.label.clone(),
                    type_label: e.entity_type.clone(),
                    color: colors
                        .get(&e.entity_type)
                        .copied()
                        .unwrap_or(egui::Color32::from_rgb(0x64, 0x74, 0x8b)),
                    pos: egui::pos2(x, y),
                    locked: e.locked,
                    bookmarked: e.bookmarked,
                    notes: e.notes.clone().unwrap_or_default(),
                    source_event: e.source_event_type.clone(),
                    geo: extract_geo(e),
                    created_at: e.created_at.timestamp_millis(),
                    entity_type: e.entity_type.clone(),
                    data: e.data.clone(),
                    source_url: e.source_url.clone(),
                }
            })
            .collect();

        let index: HashMap<&str, usize> =
            self.nodes.iter().enumerate().map(|(i, nv)| (nv.id.as_str(), i)).collect();

        self.edges = edges
            .iter()
            .filter_map(|e| {
                Some(EdgeView {
                    id: e.id.clone(),
                    source: *index.get(e.source_id.as_str())?,
                    target: *index.get(e.target_id.as_str())?,
                    label: e.label.clone().unwrap_or_default(),
                    relationship_type: e.relationship_type.clone(),
                })
            })
            .collect();

        self.neighbors = vec![HashSet::new(); self.nodes.len()];
        for e in &self.edges {
            self.neighbors[e.source].insert(e.target);
            self.neighbors[e.target].insert(e.source);
        }

        self.annotations = app_lib::list_annotations(st, self.case_id).unwrap_or_default();
        self.unread_count =
            app_lib::unread_notification_count(st, self.case_id).unwrap_or(0);

        self.fitted = false;
        self.selected.clear();
        self.primary = None;
        self.editing_for = None;
        // Invalidate cached image textures so deleted/changed images don't
        // linger; they re-decode lazily on next view.
        self.gallery_thumbs.clear();
        self.preview = None;
        if let Some(sel) = &self.gallery_selected {
            if !self.nodes.iter().any(|n| &n.id == sel) {
                self.gallery_selected = None;
            }
        }
        self.status = format!("{} entities · {} edges", self.nodes.len(), self.edges.len());
    }

    /// Load the inspector edit buffers + tags for the selected node. Called
    /// when the selection changes so the buffers track the right entity.
    fn sync_edit_buffers(&mut self) {
        if self.editing_for == self.primary {
            return;
        }
        self.editing_for = self.primary;
        // Collapse the inline tool picker when the selection changes.
        self.tools_inline = false;
        self.ctx_filter.clear();
        self.ctx_source.clear();
        // Drop any cached image preview / analysis from the previous node.
        self.preview = None;
        self.analysis_result = None;
        if let Some(i) = self.primary {
            let id = self.nodes[i].id.clone();
            self.label_buf = self.nodes[i].label.clone();
            self.notes_buf = self.nodes[i].notes.clone();
            self.tags = app_lib::list_tags_for_entity(&self.state, &id).unwrap_or_default();
            // Provenance ("where did this come from") + this entity's own
            // transform-run history — web EntityInspector parity.
            self.prov = app_lib::entity_provenance(&self.state, &id).ok();
            self.runs =
                app_lib::list_transform_runs_for_entity(&self.state, &id, 25).unwrap_or_default();
        } else {
            self.label_buf.clear();
            self.notes_buf.clear();
            self.tags.clear();
            self.prov = None;
            self.runs.clear();
        }
        self.tag_input.clear();
    }

    /// Map view: an equirectangular coordinate plot (graticule + points) of
    /// every entity that exposes a lat/lng. No basemap tiles — a network-free
    /// coordinate map; real OSM tiles could be layered later.
    fn draw_map(&mut self, ui: &mut egui::Ui) {
        let (resp, painter) =
            ui.allocate_painter(ui.available_size(), egui::Sense::hover());
        let rect = resp.rect;
        let proj = |lat: f64, lng: f64| -> egui::Pos2 {
            egui::pos2(
                rect.left() + ((lng + 180.0) / 360.0) as f32 * rect.width(),
                rect.top() + ((90.0 - lat) / 180.0) as f32 * rect.height(),
            )
        };
        let grid = egui::Color32::from_rgb(0x18, 0x21, 0x3a);
        for lng in (-180..=180).step_by(30) {
            painter.line_segment(
                [proj(90.0, lng as f64), proj(-90.0, lng as f64)],
                egui::Stroke::new(1.0, grid),
            );
        }
        for lat in (-90..=90).step_by(30) {
            painter.line_segment(
                [proj(lat as f64, -180.0), proj(lat as f64, 180.0)],
                egui::Stroke::new(1.0, grid),
            );
        }
        painter.line_segment(
            [proj(0.0, -180.0), proj(0.0, 180.0)],
            egui::Stroke::new(1.0, egui::Color32::from_rgb(0x2a, 0x37, 0x57)),
        );

        let mut count = 0;
        for nv in &self.nodes {
            if let Some((lat, lng)) = nv.geo {
                if self.hidden_types.contains(&nv.type_label) {
                    continue;
                }
                let p = proj(lat, lng);
                painter.circle_filled(p, 6.0, nv.color);
                painter.circle_stroke(p, 6.0, egui::Stroke::new(1.0, egui::Color32::from_gray(0x10)));
                outlined_text(&painter, egui::pos2(p.x, p.y + 8.0), &nv.label, 11.0);
                count += 1;
            }
        }
        if count == 0 {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "No entities with coordinates.\nRun a geo transform (e.g. sfp_geoinfo) on an IP to plot it here.",
                egui::FontId::proportional(14.0),
                egui::Color32::from_gray(0x77),
            );
        }
    }

    /// Timeline view: every entity on a horizontal time axis by creation time.
    /// Drag to pan, scroll to zoom (anchored at the cursor).
    fn draw_timeline(&mut self, ui: &mut egui::Ui) {
        let (resp, painter) =
            ui.allocate_painter(ui.available_size(), egui::Sense::click_and_drag());
        let rect = resp.rect;

        let mut items: Vec<(i64, usize)> = self
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| !self.hidden_types.contains(&n.type_label))
            .map(|(i, n)| (n.created_at, i))
            .collect();
        if items.is_empty() {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "No entities to place on the timeline.",
                egui::FontId::proportional(14.0),
                egui::Color32::from_gray(0x77),
            );
            return;
        }
        items.sort_by_key(|x| x.0);
        let tmin = items.first().unwrap().0;
        let tmax = items.last().unwrap().0.max(tmin + 1);

        if self.tl_scale <= 0.0 {
            self.tl_scale = (rect.width() - 100.0) / (tmax - tmin) as f32;
            self.tl_offset = rect.left() + 50.0;
        }
        if resp.dragged() {
            self.tl_offset += resp.drag_delta().x;
        }
        if resp.hovered() {
            let scroll = ui.input(|i| i.raw_scroll_delta.y)
                + (ui.input(|i| i.zoom_delta()) - 1.0) * 200.0;
            if scroll.abs() > f32::EPSILON {
                let anchor = resp.hover_pos().map(|p| p.x).unwrap_or(rect.center().x);
                let t_before = (anchor - self.tl_offset) / self.tl_scale.max(1e-12);
                self.tl_scale = (self.tl_scale * (1.0 + scroll * 0.001)).max(1e-12);
                self.tl_offset = anchor - t_before * self.tl_scale;
            }
        }
        let (off, scale) = (self.tl_offset, self.tl_scale);
        let x_of = |ts: i64| off + (ts - tmin) as f32 * scale;

        let baseline = rect.center().y + 80.0;
        let axis = egui::Color32::from_rgb(0x33, 0x41, 0x55);
        painter.line_segment(
            [egui::pos2(rect.left(), baseline), egui::pos2(rect.right(), baseline)],
            egui::Stroke::new(1.5, axis),
        );

        // a few date ticks across the visible span
        for k in 0..=6 {
            let ts = tmin + (tmax - tmin) * k / 6;
            let x = x_of(ts);
            if x < rect.left() || x > rect.right() {
                continue;
            }
            painter.line_segment(
                [egui::pos2(x, baseline - 4.0), egui::pos2(x, baseline + 4.0)],
                egui::Stroke::new(1.0, axis),
            );
            if let Some(d) = chrono::DateTime::from_timestamp_millis(ts) {
                painter.text(
                    egui::pos2(x, baseline + 8.0),
                    egui::Align2::CENTER_TOP,
                    d.format("%Y-%m-%d").to_string(),
                    egui::FontId::proportional(10.0),
                    egui::Color32::from_gray(0x77),
                );
            }
        }

        // stems + dots + labels (alternating heights to reduce overlap)
        for (k, (ts, i)) in items.iter().enumerate() {
            let nv = &self.nodes[*i];
            let x = x_of(*ts);
            if x < rect.left() - 30.0 || x > rect.right() + 30.0 {
                continue;
            }
            let h = if k % 2 == 0 { 38.0 } else { 70.0 };
            let top = egui::pos2(x, baseline - h);
            painter.line_segment(
                [top, egui::pos2(x, baseline)],
                egui::Stroke::new(1.0, egui::Color32::from_rgb(0x2a, 0x37, 0x57)),
            );
            painter.circle_filled(top, 5.0, nv.color);
            outlined_text(&painter, egui::pos2(x, top.y - 14.0), &nv.label, 10.0);
        }
    }

    fn world_to_screen(&self, p: egui::Pos2) -> egui::Pos2 {
        egui::pos2(p.x * self.zoom + self.offset.x, p.y * self.zoom + self.offset.y)
    }

    /// Frame the whole graph in `rect` (initial view / "fit" button).
    fn fit(&mut self, rect: egui::Rect) {
        if self.nodes.is_empty() {
            self.zoom = 1.0;
            self.offset = rect.center().to_vec2();
            return;
        }
        let (mut min, mut max) = (egui::pos2(f32::MAX, f32::MAX), egui::pos2(f32::MIN, f32::MIN));
        for nv in &self.nodes {
            min.x = min.x.min(nv.pos.x);
            min.y = min.y.min(nv.pos.y);
            max.x = max.x.max(nv.pos.x);
            max.y = max.y.max(nv.pos.y);
        }
        let pad = 90.0;
        let w = (max.x - min.x).max(1.0) + pad * 2.0;
        let h = (max.y - min.y).max(1.0) + pad * 2.0;
        self.zoom = (rect.width() / w).min(rect.height() / h).clamp(0.1, 3.0);
        let center_world = egui::pos2((min.x + max.x) * 0.5, (min.y + max.y) * 0.5);
        self.offset = rect.center().to_vec2() - center_world.to_vec2() * self.zoom;
    }

    fn node_screen_radius(&self) -> f32 {
        (NODE_R * self.zoom).clamp(5.0, 64.0)
    }

    fn visible(&self, i: usize) -> bool {
        !self.hidden_types.contains(&self.nodes[i].type_label)
    }

    fn node_at(&self, screen: egui::Pos2) -> Option<usize> {
        let r = self.node_screen_radius() + 2.0;
        // Reverse order so topmost (later-drawn) wins.
        self.nodes
            .iter()
            .enumerate()
            .rev()
            .find(|(i, nv)| self.visible(*i) && self.world_to_screen(nv.pos).distance(screen) <= r)
            .map(|(i, _)| i)
    }

    /// Edge nearest the pointer within a few px, for edge selection. Tests
    /// point-to-segment distance against each visible edge.
    fn edge_at(&self, screen: egui::Pos2) -> Option<usize> {
        let tol = 6.0;
        let mut best: Option<(usize, f32)> = None;
        for (i, e) in self.edges.iter().enumerate() {
            if !self.visible(e.source) || !self.visible(e.target) {
                continue;
            }
            let a = self.world_to_screen(self.nodes[e.source].pos);
            let b = self.world_to_screen(self.nodes[e.target].pos);
            let ab = b - a;
            let len2 = ab.length_sq();
            if len2 < 1.0 {
                continue;
            }
            let t = ((screen - a).dot(ab) / len2).clamp(0.0, 1.0);
            let proj = a + ab * t;
            let d = proj.distance(screen);
            if d <= tol && best.map(|(_, bd)| d < bd).unwrap_or(true) {
                best = Some((i, d));
            }
        }
        best.map(|(i, _)| i)
    }

    /// Force-directed (Fruchterman–Reingold style) auto-layout — the native
    /// equivalent of the web build's fcose layout. Repulsion between all
    /// nodes, attraction along edges, with cooling. Persists the result.
    fn arrange(&mut self) {
        let n = self.nodes.len();
        if n < 2 {
            return;
        }
        let k = 130.0_f32; // ideal edge length
        let mut temp = 60.0_f32;
        for _ in 0..400 {
            let mut disp = vec![egui::Vec2::ZERO; n];
            for i in 0..n {
                for j in (i + 1)..n {
                    let mut d = self.nodes[i].pos - self.nodes[j].pos;
                    let mut len = d.length();
                    if len < 0.01 {
                        d = egui::vec2((i as f32 + 1.0).sin(), (j as f32 + 1.0).cos());
                        len = d.length().max(0.01);
                    }
                    let f = (k * k) / len;
                    disp[i] += d / len * f;
                    disp[j] -= d / len * f;
                }
            }
            for e in &self.edges {
                let d = self.nodes[e.source].pos - self.nodes[e.target].pos;
                let len = d.length().max(0.01);
                let f = (len * len) / k;
                disp[e.source] -= d / len * f;
                disp[e.target] += d / len * f;
            }
            for i in 0..n {
                if self.nodes[i].locked {
                    continue;
                }
                let l = disp[i].length();
                if l > 0.0 {
                    self.nodes[i].pos += disp[i] / l * l.min(temp);
                }
            }
            temp *= 0.985;
        }
        for nv in &self.nodes {
            let _ = app_lib::update_entity_position(
                &self.state,
                &nv.id,
                nv.pos.x as f64,
                nv.pos.y as f64,
            );
        }
        self.fitted = false;
    }
}

/// Mix a Color32 toward white (amt>0) or black (amt<0) by |amt| (0..1).
fn shade_col(c: egui::Color32, amt: f32) -> egui::Color32 {
    let t = if amt >= 0.0 { 255.0 } else { 0.0 };
    let a = amt.abs();
    let f = |x: u8| (x as f32 + (t - x as f32) * a) as u8;
    egui::Color32::from_rgb(f(c.r()), f(c.g()), f(c.b()))
}

impl eframe::App for MaltraceApp {
    /// Opaque dark clear colour so the desktop/wallpaper never shows through.
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.027, 0.043, 0.078, 1.0] // ≈ #070b14
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.sync_edit_buffers();

        // command palette toggle (Ctrl/Cmd+K) + Esc to dismiss
        if ctx.input(|i| i.key_pressed(egui::Key::K) && i.modifiers.command) {
            self.palette_open = !self.palette_open;
            self.palette_query.clear();
        }
        if self.palette_open && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.palette_open = false;
        }

        // ── pick up async sidecar results ──
        let mut new_transforms = None;
        if let Some(rx) = self.transforms_rx.as_ref() {
            if let Ok(list) = rx.try_recv() {
                new_transforms = Some(list);
            }
        }
        if let Some(list) = new_transforms {
            self.transforms_rx = None;
            self.status = format!(
                "{} entities · {} edges · {} transforms",
                self.nodes.len(),
                self.edges.len(),
                list.len()
            );
            self.transforms = list;
        }
        // ── pick up per-sidecar load status (web #1) ──
        let mut new_status = None;
        if let Some(rx) = self.sidecar_status_rx.as_ref() {
            if let Ok(st) = rx.try_recv() {
                new_status = Some(st);
            }
        }
        if let Some(st) = new_status {
            self.sidecar_status_rx = None;
            let failed: Vec<&str> =
                st.iter().filter(|s| !s.ok).map(|s| s.id.as_str()).collect();
            if !failed.is_empty() {
                self.status = format!(
                    "⚠ {} sidecar(s) failed to load: {}",
                    failed.len(),
                    failed.join(", ")
                );
            }
            self.sidecar_status = st;
        }
        // ── pick up async machine list ──
        let mut new_machines = None;
        if let Some(rx) = self.machine_rx.as_ref() {
            if let Ok(list) = rx.try_recv() {
                new_machines = Some(list);
            }
        }
        if let Some(list) = new_machines {
            self.machine_rx = None;
            self.machines = list;
            self.machines_loaded = true;
        }
        // ── pick up async AI image-analysis result ──
        let mut analysis = None;
        if let Some(rx) = self.analyze_rx.as_ref() {
            if let Ok(res) = rx.try_recv() {
                analysis = Some(res);
            }
        }
        if let Some((eid, res)) = analysis {
            self.analyze_rx = None;
            self.analyzing = false;
            match res {
                Ok(text) => {
                    self.status = "image analysis complete".into();
                    self.analysis_result = Some((eid, text));
                    // notes may have changed if save_to_notes was set — refresh
                    self.editing_for = None;
                    self.sync_edit_buffers();
                }
                Err(e) => {
                    self.status = format!("analysis failed: {e}");
                    self.analysis_result = Some((eid, format!("Error: {e}")));
                }
            }
        }

        // ── pick up async AI reply ──
        let mut ai_reply = None;
        if let Some(rx) = self.ai_rx.as_ref() {
            if let Ok(res) = rx.try_recv() {
                ai_reply = Some(res);
            }
        }
        if let Some(res) = ai_reply {
            self.ai_rx = None;
            self.ai_busy = false;
            match res {
                Ok(reply) => self.ai_history.push(app_lib::ChatTurn {
                    role: "assistant".into(),
                    content: reply.message,
                }),
                Err(e) => self.ai_history.push(app_lib::ChatTurn {
                    role: "assistant".into(),
                    content: format!("[error] {e}"),
                }),
            }
        }
        let mut run_result = None;
        if let Some(rx) = self.run_rx.as_ref() {
            if let Ok(res) = rx.try_recv() {
                run_result = Some(res);
            }
        }
        if let Some(res) = run_result {
            self.run_rx = None;
            self.running = false;
            match res {
                Ok(n) => {
                    self.status = format!("transform added {n} entit{}", if n == 1 { "y" } else { "ies" });
                    self.reload();
                }
                Err(e) => self.status = format!("transform failed: {e}"),
            }
        }
        if self.running
            || self.transforms_rx.is_some()
            || self.sidecar_status_rx.is_some()
            || self.machine_rx.is_some()
            || self.ai_rx.is_some()
            || self.analyze_rx.is_some()
        {
            ctx.request_repaint(); // keep polling while async work is in flight
        }

        egui::TopBottomPanel::top("topbar")
            .frame(
                egui::Frame::none()
                    .fill(pal::BG2)
                    .inner_margin(egui::Margin::symmetric(10.0, 6.0))
                    .stroke(egui::Stroke::new(1.0, pal::BORDER)),
            )
            .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("◆")
                        .color(pal::ACCENT)
                        .size(14.0),
                );
                ui.label(
                    egui::RichText::new("maltrace")
                        .color(pal::TEXT)
                        .strong()
                        .size(16.0),
                );
                ui.separator();
                for (v, name) in [
                    (View::Graph, "Graph"),
                    (View::Map, "Map"),
                    (View::Timeline, "Timeline"),
                ] {
                    if ui.selectable_label(self.view == v, name).clicked() {
                        self.view = v;
                    }
                }
                ui.separator();
                if ui.button("Arrange").on_hover_text("Auto-layout (force-directed)").clicked() {
                    self.arrange();
                }
                if ui.button("Fit").clicked() {
                    self.fitted = false;
                }
                if ui.button("+").on_hover_text("Zoom in").clicked() {
                    self.zoom = (self.zoom * 1.2).min(4.0);
                }
                if ui.button("-").on_hover_text("Zoom out").clicked() {
                    self.zoom = (self.zoom / 1.2).max(0.1);
                }
                if ui.button("Reload").clicked() {
                    self.reload();
                }
                ui.separator();
                ui.label(egui::RichText::new("Find").weak());
                ui.add(
                    egui::TextEdit::singleline(&mut self.search)
                        .hint_text("label…")
                        .desired_width(150.0),
                );
                if self.running || self.transforms_rx.is_some() || self.machine_rx.is_some() {
                    ui.spinner();
                }
                ui.label(egui::RichText::new(&self.status).color(pal::MUTED));
            });

            // ── second toolbar row: panel toggles, wrapped so they never run
            //    off-screen at narrow widths ──
            ui.add_space(3.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                if ui.selectable_label(self.add_open, "Add").clicked() {
                    self.add_open = !self.add_open;
                }
                if ui.button("Note").on_hover_text("Add a sticky note at center").clicked() {
                    self.add_annotation_at_center();
                }
                ui.separator();
                if ui.selectable_label(self.gallery_open, "🖼 Gallery").on_hover_text("Browse images in this case").clicked() {
                    self.gallery_open = !self.gallery_open;
                }
                if ui.selectable_label(self.runlog_open, "Runs").clicked() {
                    self.runlog_open = !self.runlog_open;
                    if self.runlog_open {
                        self.runlog =
                            app_lib::list_transform_runs(&self.state, self.case_id, 100)
                                .unwrap_or_default();
                    }
                }
                if ui.selectable_label(self.machines_open, "Machines").clicked() {
                    self.machines_open = !self.machines_open;
                    if self.machines_open && !self.machines_loaded {
                        self.load_machines();
                    }
                }
                if ui.selectable_label(self.settings_open, "Settings").clicked() {
                    self.settings_open = !self.settings_open;
                    if self.settings_open {
                        self.secrets = app_lib::list_secret_names(&self.state);
                        self.load_network_settings();
                    }
                }
                if ui.selectable_label(self.ai_open, "AI").clicked() {
                    self.ai_open = !self.ai_open;
                    if self.ai_open && self.ai_status.is_none() {
                        self.ai_status = Some(app_lib::ai_status(&self.state));
                    }
                }
                if ui.selectable_label(self.plugins_open, "Plugins").clicked() {
                    self.plugins_open = !self.plugins_open;
                    if self.plugins_open {
                        self.plugins = app_lib::list_plugins(&self.state);
                    }
                }
                if ui.selectable_label(self.bookmarks_open, "Bookmarks").clicked() {
                    self.bookmarks_open = !self.bookmarks_open;
                }
                if ui.selectable_label(self.audit_open, "Audit").clicked() {
                    self.audit_open = !self.audit_open;
                    if self.audit_open {
                        self.audit =
                            app_lib::list_audit_events(&self.state, self.case_id, 200)
                                .unwrap_or_default();
                    }
                }
                if ui.selectable_label(self.import_open, "Import").clicked() {
                    self.import_open = !self.import_open;
                }
                if ui.selectable_label(self.export_open, "Export").clicked() {
                    self.export_open = !self.export_open;
                }
                if ui.selectable_label(self.snapshots_open, "Snapshots").clicked() {
                    self.snapshots_open = !self.snapshots_open;
                    if self.snapshots_open {
                        self.snapshots =
                            app_lib::list_case_snapshots(&self.state, self.case_id)
                                .unwrap_or_default();
                    }
                }
                if ui.selectable_label(self.searches_open, "Searches").clicked() {
                    self.searches_open = !self.searches_open;
                    if self.searches_open {
                        self.saved_searches =
                            app_lib::list_saved_searches(&self.state, self.case_id)
                                .unwrap_or_default();
                    }
                }
                let bell = if self.unread_count > 0 {
                    format!("🔔 {}", self.unread_count)
                } else {
                    "🔔".to_string()
                };
                if ui.selectable_label(self.notifications_open, bell).clicked() {
                    self.notifications_open = !self.notifications_open;
                    if self.notifications_open {
                        self.notifications =
                            app_lib::list_notifications(&self.state, self.case_id, 100)
                                .unwrap_or_default();
                        let _ = app_lib::mark_notifications_read(&self.state, self.case_id);
                        self.unread_count = 0;
                    }
                }
            });

            // ── case tabs (multi-case parity) ──
            let mut switch_to: Option<i64> = None;
            let mut new_case = false;
            let mut delete_case: Option<i64> = None;
            let mut toggle_lock: Option<(i64, bool)> = None;
            let mut commit_rename: Option<(i64, String)> = None;
            ui.add_space(2.0);
            ui.horizontal_wrapped(|ui| {
                let cases = self.cases.clone();
                for c in &cases {
                    let active = c.id == self.case_id;
                    let renaming =
                        self.case_rename.as_ref().map(|(id, _)| *id == c.id).unwrap_or(false);
                    if renaming {
                        if let Some((_, buf)) = self.case_rename.as_mut() {
                            let r = ui.add(
                                egui::TextEdit::singleline(buf).desired_width(120.0),
                            );
                            r.request_focus();
                            if r.lost_focus() {
                                commit_rename = self.case_rename.take();
                            }
                        }
                    } else {
                        let label = if c.locked {
                            format!("🔒 {}", c.name)
                        } else {
                            c.name.clone()
                        };
                        let resp = ui.selectable_label(active, label);
                        if resp.clicked() {
                            switch_to = Some(c.id);
                        }
                        if resp.double_clicked() {
                            self.case_rename = Some((c.id, c.name.clone()));
                        }
                        resp.context_menu(|ui| {
                            if ui.button("Rename").clicked() {
                                self.case_rename = Some((c.id, c.name.clone()));
                                ui.close_menu();
                            }
                            let lock_txt = if c.locked { "Unlock" } else { "Lock (freeze)" };
                            if ui.button(lock_txt).clicked() {
                                toggle_lock = Some((c.id, !c.locked));
                                ui.close_menu();
                            }
                            if ui.button("Delete case").clicked() {
                                delete_case = Some(c.id);
                                ui.close_menu();
                            }
                        });
                    }
                }
                if ui.button("+ Case").clicked() {
                    new_case = true;
                }
            });
            if let Some(id) = switch_to {
                self.switch_case(id);
            }
            if new_case {
                let n = self.cases.len() + 1;
                match app_lib::create_case(&self.state, &format!("Investigation {n}")) {
                    Ok(c) => self.switch_case(c.id),
                    Err(e) => self.status = format!("create case failed: {e}"),
                }
            }
            if let Some((id, name)) = commit_rename {
                let name = name.trim().to_string();
                if !name.is_empty() {
                    let _ = app_lib::rename_case(&self.state, id, &name);
                }
                self.reload();
            }
            if let Some((id, locked)) = toggle_lock {
                let _ = app_lib::set_case_locked(&self.state, id, locked, Some("native"));
                self.reload();
            }
            if let Some(id) = delete_case {
                if self.cases.len() > 1 {
                    let _ = app_lib::delete_case(&self.state, id);
                    if id == self.case_id {
                        // active case gone → fall back to default on reload.
                        self.case_id = self.cases.iter().map(|c| c.id).find(|&x| x != id).unwrap_or(1);
                    }
                    self.reload();
                } else {
                    self.status = "can't delete the last case".into();
                }
            }

            // ── failed-sidecar banner (web #1): name any sidecar that didn't
            //    load so its transforms aren't silently missing ──
            let failed: Vec<&app_lib::SidecarStatus> =
                self.sidecar_status.iter().filter(|s| !s.ok).collect();
            if !failed.is_empty() {
                ui.add_space(2.0);
                let names = failed
                    .iter()
                    .map(|s| s.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                let resp = ui.label(
                    egui::RichText::new(format!(
                        "⚠ {} sidecar(s) failed to load: {names}",
                        failed.len()
                    ))
                    .color(egui::Color32::from_rgb(0xf6, 0xb0, 0x73))
                    .size(12.0),
                );
                let detail = failed
                    .iter()
                    .map(|s| format!("{}: {}", s.id, s.error.clone().unwrap_or_default()))
                    .collect::<Vec<_>>()
                    .join("\n");
                resp.on_hover_text(detail);
            }

            // ── type filter strip: one chip per entity type, click to hide ──
            let mut counts: Vec<(String, egui::Color32, usize)> = Vec::new();
            {
                let mut idx: HashMap<&str, usize> = HashMap::new();
                for nv in &self.nodes {
                    match idx.get(nv.type_label.as_str()) {
                        Some(&p) => counts[p].2 += 1,
                        None => {
                            idx.insert(&nv.type_label, counts.len());
                            counts.push((nv.type_label.clone(), nv.color, 1));
                        }
                    }
                }
            }
            counts.sort_by(|a, b| b.2.cmp(&a.2));
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                for (ty, color, count) in counts {
                    let on = !self.hidden_types.contains(&ty);
                    if type_chip(ui, &ty, color, count, on).clicked() {
                        if on {
                            self.hidden_types.insert(ty);
                        } else {
                            self.hidden_types.remove(&ty);
                        }
                    }
                }
            });
            ui.add_space(2.0);
        });

        // ── right inspector: ALWAYS present. When collapsed, a thin rail with
        //    a reopen button; otherwise context-sensitive content (multi-select
        //    / entity / edge / empty state). Guarantees the panel is never
        //    permanently lost. ──
        if self.inspector_collapsed {
            egui::SidePanel::right("inspector-rail")
                .resizable(false)
                .exact_width(30.0)
                .show(ctx, |ui| {
                    ui.add_space(8.0);
                    if ui
                        .button("◀")
                        .on_hover_text("Show inspector")
                        .clicked()
                    {
                        self.inspector_collapsed = false;
                    }
                });
        } else if self.selected.len() > 1 {
            let count = self.selected.len();
            let mut run_sel = false;
            let mut del_sel = false;
            egui::SidePanel::right("inspector")
                .resizable(true)
                .default_width(330.0)
                .show(ctx, |ui| {
                    ui.add_space(6.0);
                    ui.heading(format!("{count} selected"));
                    ui.separator();
                    ui.label(
                        egui::RichText::new("Shift-click a node or shift-drag a box to adjust.")
                            .weak()
                            .small(),
                    );
                    ui.add_space(10.0);
                    run_sel = ui.button("🔍 Run tool on selection…").clicked();
                    ui.add_space(4.0);
                    del_sel = danger_button(ui, "Delete selected").clicked();
                });
            if run_sel {
                if let Some(n) = self.primary.or_else(|| self.selected.iter().copied().next()) {
                    self.ctx_menu = Some((n, egui::pos2(40.0, 130.0)));
                }
            }
            if del_sel {
                let ids: Vec<String> =
                    self.selected.iter().map(|&i| self.nodes[i].id.clone()).collect();
                for id in ids {
                    let _ = app_lib::delete_entity(&self.state, &id, Some("native"));
                }
                self.reload();
            }
        } else if let Some(sel) = self.primary {
            let mut expand_collection: Option<String> = None;
            let mut run_all: Option<usize> = None;
            let mut inline_tool: Option<(usize, String)> = None;
            let mut open_gallery = false;
            let mut collapse_inspector = false;
            egui::SidePanel::right("inspector")
                .resizable(true)
                .default_width(330.0)
                .show(ctx, |ui| {
                    let id = self.nodes[sel].id.clone();
                    let color = self.nodes[sel].color;
                    let type_label = self.nodes[sel].type_label.clone();
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        badge(ui, &type_label, color);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("▶").on_hover_text("Hide inspector").clicked() {
                                collapse_inspector = true;
                            }
                        });
                    });
                    if let Some(se) = &self.nodes[sel].source_event {
                        ui.label(
                            egui::RichText::new(se).color(pal::MUTED2).monospace().size(10.0),
                        );
                    }
                    ui.add_space(4.0);
                    ui.separator();

                    // ── label (rename on commit) ──
                    field_label(ui, "Label");
                    let le = ui.add(
                        egui::TextEdit::singleline(&mut self.label_buf).desired_width(f32::INFINITY),
                    );
                    if le.lost_focus() {
                        let new = self.label_buf.trim().to_string();
                        if !new.is_empty() && new != self.nodes[sel].label {
                            if app_lib::update_entity_label(&self.state, &id, &new, Some("native")).is_ok() {
                                self.nodes[sel].label = new;
                            }
                        }
                    }

                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        let mut bm = self.nodes[sel].bookmarked;
                        if ui.checkbox(&mut bm, "Bookmark").changed() {
                            let _ = app_lib::update_entity_bookmark(&self.state, &id, bm);
                            self.nodes[sel].bookmarked = bm;
                        }
                        let mut lk = self.nodes[sel].locked;
                        if ui.checkbox(&mut lk, "Lock").changed() {
                            let _ = app_lib::update_entity_locked(&self.state, &id, lk);
                            self.nodes[sel].locked = lk;
                        }
                    });

                    // ── notes (save on commit) ──
                    ui.add_space(6.0);
                    field_label(ui, "Notes");
                    let nr = ui.add(
                        egui::TextEdit::multiline(&mut self.notes_buf)
                            .desired_rows(4)
                            .desired_width(f32::INFINITY),
                    );
                    if nr.lost_focus() {
                        let n = self.notes_buf.clone();
                        let opt = if n.trim().is_empty() { None } else { Some(n.as_str()) };
                        if app_lib::update_entity_notes(&self.state, &id, opt).is_ok() {
                            self.nodes[sel].notes = n;
                        }
                    }

                    // ── tags ──
                    ui.add_space(6.0);
                    field_label(ui, "Tags");
                    let mut remove: Option<String> = None;
                    ui.horizontal_wrapped(|ui| {
                        for t in &self.tags {
                            if ui.small_button(format!("{t}  x")).clicked() {
                                remove = Some(t.clone());
                            }
                        }
                    });
                    if let Some(t) = remove {
                        let _ = app_lib::remove_tag(&self.state, &id, &t);
                        self.tags.retain(|x| x != &t);
                    }
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.tag_input)
                                .hint_text("new tag")
                                .desired_width(160.0),
                        );
                        if ui.button("Add").clicked() {
                            let t = self.tag_input.trim().to_string();
                            if !t.is_empty()
                                && app_lib::add_tag(&self.state, &id, &t).is_ok()
                                && !self.tags.contains(&t)
                            {
                                self.tags.push(t);
                            }
                            self.tag_input.clear();
                        }
                    });

                    // ── Collection expand (web parity) ──
                    if self.nodes[sel].entity_type == "Collection" {
                        ui.add_space(8.0);
                        ui.separator();
                        if ui.button("⊞ Expand collection").clicked() {
                            expand_collection = Some(id.clone());
                        }
                    }

                    // ── Image: a compact preview + a jump to the Gallery, which
                    //    is where viewing/analysis actually happens now. ──
                    if self.nodes[sel].entity_type == "Image" {
                        ui.add_space(8.0);
                        ui.separator();
                        let ctxc = ui.ctx().clone();
                        self.ensure_preview(&ctxc, sel);
                        if let Some((_, tex)) = &self.preview {
                            let avail = ui.available_width().min(280.0);
                            let aspect = tex.size()[1] as f32 / tex.size()[0] as f32;
                            ui.add(
                                egui::Image::new(tex)
                                    .fit_to_exact_size(egui::vec2(avail, avail * aspect))
                                    .rounding(egui::Rounding::same(6.0)),
                            );
                        }
                        ui.add_space(6.0);
                        if ui.button("🖼 Open in Gallery").clicked() {
                            open_gallery = true;
                        }
                    }

                    // ── Actions: run OSINT tools on this entity (inline picker) ──
                    if self.nodes[sel].entity_type != "Collection" {
                        ui.add_space(8.0);
                        ui.separator();
                        let n_compat = self.compatible_transforms(sel).len();
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(format!("OSINT tools  ·  {n_compat}"))
                                    .color(pal::ACCENT)
                                    .strong(),
                            );
                            // toggle the inline picker open/closed
                            let label = if self.tools_inline { "Hide" } else { "🔍 Run tool…" };
                            if ui
                                .add_enabled(n_compat > 0, egui::Button::new(label))
                                .on_hover_text("Pick a compatible OSINT tool to run on this entity")
                                .clicked()
                            {
                                self.tools_inline = !self.tools_inline;
                                self.ctx_filter.clear();
                                self.ctx_source.clear();
                            }
                            if ui
                                .add_enabled(!self.running && n_compat > 0, egui::Button::new("▶ Run all"))
                                .on_hover_text("Run every compatible transform on this entity")
                                .clicked()
                            {
                                run_all = Some(sel);
                            }
                        });
                        if n_compat == 0 {
                            ui.label(
                                egui::RichText::new(
                                    "No compatible tools yet — sidecars may still be loading.",
                                )
                                .weak()
                                .small(),
                            );
                        }
                        if self.running {
                            ui.label(egui::RichText::new("running…").weak().small());
                        }
                        // Inline tool picker, rendered right here in the panel.
                        if self.tools_inline {
                            ui.add_space(4.0);
                            egui::Frame::group(ui.style()).show(ui, |ui| {
                                let (chosen, close) = self.tools_picker_ui(ui, sel);
                                if let Some(tid) = chosen {
                                    inline_tool = Some((sel, tid));
                                }
                                if close {
                                    self.tools_inline = false;
                                }
                            });
                        }
                    }

                    // ── Properties (parsed from the data payload) ──
                    let props = node_properties(self.nodes[sel].data.as_deref());
                    if !props.is_empty() {
                        ui.add_space(8.0);
                        egui::CollapsingHeader::new("Properties")
                            .default_open(true)
                            .show(ui, |ui| {
                                egui::Grid::new("props-grid")
                                    .num_columns(2)
                                    .striped(true)
                                    .show(ui, |ui| {
                                        for (k, v) in &props {
                                            ui.label(egui::RichText::new(k).weak().small());
                                            ui.label(egui::RichText::new(v).small());
                                            ui.end_row();
                                        }
                                    });
                            });
                    }

                    // ── Source citation link ──
                    if let Some(url) = self.nodes[sel].source_url.clone() {
                        ui.add_space(6.0);
                        ui.label(egui::RichText::new("Source").weak());
                        ui.hyperlink(url);
                    }

                    // ── Provenance: where did this come from ──
                    if let Some(prov) = &self.prov {
                        ui.add_space(8.0);
                        egui::CollapsingHeader::new("Provenance")
                            .default_open(false)
                            .show(ui, |ui| {
                                if let Some(tid) = &prov.transform_id {
                                    ui.label(egui::RichText::new("Transform").weak().small());
                                    ui.label(egui::RichText::new(tid).monospace().small());
                                }
                                if let Some(run) = &prov.run {
                                    if let Some(sl) = &run.source_label {
                                        ui.label(egui::RichText::new("Seed").weak().small());
                                        ui.label(egui::RichText::new(sl).small());
                                    }
                                    ui.label(
                                        egui::RichText::new(format!(
                                            "run #{} · {} · {} ms",
                                            run.id, run.status, run.duration_ms
                                        ))
                                        .weak()
                                        .small(),
                                    );
                                }
                                if prov.run.is_none() && prov.transform_id.is_none() {
                                    ui.label(
                                        egui::RichText::new("Manually added (no transform).")
                                            .weak()
                                            .small(),
                                    );
                                }
                                ui.label(
                                    egui::RichText::new(format!(
                                        "{} in · {} out",
                                        prov.incoming.len(),
                                        prov.outgoing.len()
                                    ))
                                    .weak()
                                    .small(),
                                );
                            });
                    }

                    // ── This entity's transform-run history ──
                    if !self.runs.is_empty() {
                        ui.add_space(4.0);
                        egui::CollapsingHeader::new(format!("Runs ({})", self.runs.len()))
                            .default_open(false)
                            .show(ui, |ui| {
                                for r in &self.runs {
                                    let ok = r.status == "ok";
                                    let mark = if ok { "✓" } else { "✕" };
                                    let col = if ok {
                                        egui::Color32::from_rgb(0x86, 0xef, 0xac)
                                    } else {
                                        egui::Color32::from_rgb(0xf8, 0x71, 0x71)
                                    };
                                    let line = format!(
                                        "{mark} {} (+{}) {} ms",
                                        r.transform_id, r.new_entity_count, r.duration_ms
                                    );
                                    let resp = ui.label(egui::RichText::new(line).color(col).small());
                                    if let Some(err) = &r.error_message {
                                        resp.on_hover_text(err);
                                    }
                                }
                            });
                    }

                    ui.add_space(10.0);
                    ui.label(
                        egui::RichText::new(format!("{} connection(s)", self.neighbors[sel].len()))
                            .color(pal::MUTED),
                    );
                    field_label(ui, "ID");
                    ui.label(egui::RichText::new(&id).monospace().small().color(pal::MUTED2));

                    ui.add_space(12.0);
                    if danger_button(ui, "Delete entity").clicked() {
                        let _ = app_lib::delete_entity(&self.state, &id, Some("native"));
                        self.reload();
                    }
                });
            if let Some(cid) = expand_collection {
                match app_lib::expand_collection(&self.state, &cid, Some("native")) {
                    Ok(_) => self.reload(),
                    Err(e) => self.status = format!("expand failed: {e}"),
                }
            }
            if let Some(n) = run_all {
                self.begin_run_all(n);
            }
            if let Some((n, tid)) = inline_tool {
                self.tools_inline = false;
                self.ctx_filter.clear();
                self.ctx_source.clear();
                self.begin_transform(n, tid);
            }
            if open_gallery {
                self.gallery_open = true;
            }
            if collapse_inspector {
                self.inspector_collapsed = true;
            }
        } else if let Some(ei) = self.selected_edge {
            // ── edge inspector (web EdgeInspector) ──
            let mut close = false;
            let mut save_rel = false;
            egui::SidePanel::right("edge-inspector")
                .resizable(true)
                .default_width(300.0)
                .show(ctx, |ui| {
                    ui.add_space(6.0);
                    ui.heading("Edge");
                    ui.separator();
                    let e = &self.edges[ei];
                    let src = self.nodes[e.source].label.clone();
                    let tgt = self.nodes[e.target].label.clone();
                    ui.label(egui::RichText::new("From").weak().small());
                    ui.label(&src);
                    ui.label(egui::RichText::new("To").weak().small());
                    ui.label(&tgt);
                    if !e.label.is_empty() {
                        ui.add_space(6.0);
                        ui.label(egui::RichText::new("Produced by").weak().small());
                        ui.label(egui::RichText::new(&e.label).monospace().small());
                    }
                    ui.add_space(8.0);
                    ui.label(egui::RichText::new("Relationship type").weak().small());
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.edge_rel_buf)
                                .hint_text("e.g. owns, resolves_to")
                                .desired_width(160.0),
                        );
                        save_rel = ui.button("Save").clicked();
                    });
                    ui.add_space(10.0);
                    if ui.button("Close").clicked() {
                        close = true;
                    }
                });
            if save_rel {
                let id = self.edges[ei].id.clone();
                let rel = self.edge_rel_buf.trim().to_string();
                let opt = if rel.is_empty() { None } else { Some(rel.as_str()) };
                if app_lib::update_edge_relationship(&self.state, &id, opt).is_ok() {
                    self.edges[ei].relationship_type =
                        if rel.is_empty() { None } else { Some(rel) };
                }
            }
            if close {
                self.selected_edge = None;
            }
        } else {
            // ── empty state: nothing selected. Keep the panel visible so it's
            //    always there, with a hint + quick actions. ──
            egui::SidePanel::right("inspector")
                .resizable(true)
                .default_width(300.0)
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.add_space(2.0);
                        ui.heading("Inspector");
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("▶").on_hover_text("Hide inspector").clicked() {
                                self.inspector_collapsed = true;
                            }
                        });
                    });
                    ui.separator();
                    ui.add_space(20.0);
                    ui.vertical_centered(|ui| {
                        ui.label(egui::RichText::new("◆").color(fade(pal::ACCENT, 0.5)).size(28.0));
                        ui.add_space(8.0);
                        ui.label(
                            egui::RichText::new("Select a node or edge")
                                .color(pal::TEXT_SOFT)
                                .strong(),
                        );
                        ui.label(
                            egui::RichText::new("to inspect it, run OSINT tools, and view details.")
                                .color(pal::MUTED)
                                .small(),
                        );
                    });
                    ui.add_space(16.0);
                    ui.separator();
                    ui.add_space(6.0);
                    if ui.button("➕  Add entity").clicked() {
                        self.add_open = true;
                    }
                    ui.add_space(2.0);
                    if ui.button("🖼  Open image gallery").clicked() {
                        self.gallery_open = true;
                    }
                    ui.add_space(2.0);
                    if ui.button("✦  AI analyst").clicked() {
                        self.ai_open = true;
                        if self.ai_status.is_none() {
                            self.ai_status = Some(app_lib::ai_status(&self.state));
                        }
                    }
                });
        }

        // ── add-entity form (toggled from the toolbar) ──
        if self.add_open {
            let mut open = true;
            let mut added = false;
            let mut add_image = false;
            egui::Window::new("Add entity")
                .collapsible(false)
                .resizable(false)
                .open(&mut open)
                .show(ctx, |ui| {
                    egui::ComboBox::from_label("Type")
                        .selected_text(self.add_type.clone())
                        .show_ui(ui, |ui| {
                            for (tid, tlabel) in &self.types {
                                ui.selectable_value(&mut self.add_type, tid.clone(), tlabel);
                            }
                        });
                    ui.add(
                        egui::TextEdit::singleline(&mut self.add_label)
                            .hint_text("value / label")
                            .desired_width(260.0),
                    );
                    if ui.button("Add to graph").clicked() {
                        let label = self.add_label.trim().to_string();
                        if !label.is_empty() {
                            let input = app_lib::AddEntityInput {
                                case_id: self.case_id,
                                entity_type: self.add_type.clone(),
                                label,
                                pos_x: Some(0.0),
                                pos_y: Some(0.0),
                            };
                            match app_lib::add_entity(&self.state, input, Some("native")) {
                                Ok(_) => {
                                    self.add_label.clear();
                                    added = true;
                                }
                                Err(e) => self.status = format!("add failed: {e}"),
                            }
                        }
                    }

                    ui.add_space(8.0);
                    ui.separator();
                    ui.label(
                        egui::RichText::new("Add an image (for AI / face-search analysis):")
                            .weak()
                            .small(),
                    );
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.image_path)
                                .hint_text("/path/to/photo.jpg")
                                .desired_width(160.0),
                        );
                        if ui.button("Browse…").clicked() {
                            if let Some(p) = pick_file(&[("Images", &["jpg", "jpeg", "png", "gif", "webp", "bmp"])]) {
                                self.image_path = p;
                            }
                        }
                        if ui.button("📷 Add").clicked() {
                            add_image = true;
                        }
                    });
                });
            if add_image {
                self.do_add_image();
                added = true;
            }
            if added {
                self.reload();
                self.add_open = false;
            } else {
                self.add_open = open;
            }
        }

        // ── run-log bottom drawer (web RunLogPanel) ──
        if self.runlog_open {
            egui::TopBottomPanel::bottom("runlog")
                .resizable(true)
                .default_height(180.0)
                .frame(
                    egui::Frame::none()
                        .fill(pal::BG2)
                        .inner_margin(egui::Margin::symmetric(12.0, 8.0))
                        .stroke(egui::Stroke::new(1.0, pal::BORDER)),
                )
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.heading("Transform runs");
                        let ok = self.runlog.iter().filter(|r| r.status == "ok").count();
                        let err = self.runlog.len() - ok;
                        ui.label(
                            egui::RichText::new(format!("{ok} ok · {err} error")).weak().small(),
                        );
                        if ui.button("Refresh").clicked() {
                            self.runlog =
                                app_lib::list_transform_runs(&self.state, self.case_id, 100)
                                    .unwrap_or_default();
                        }
                    });
                    ui.separator();
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        if self.runlog.is_empty() {
                            ui.label(egui::RichText::new("No runs yet.").weak());
                        }
                        for r in &self.runlog {
                            let ok = r.status == "ok";
                            let col = if ok {
                                egui::Color32::from_rgb(0x86, 0xef, 0xac)
                            } else {
                                egui::Color32::from_rgb(0xf8, 0x71, 0x71)
                            };
                            let line = format!(
                                "{} {}  ·  {}  ·  +{}  ·  {} ms",
                                if ok { "✓" } else { "✕" },
                                r.transform_id,
                                r.source_label.clone().unwrap_or_default(),
                                r.new_entity_count,
                                r.duration_ms,
                            );
                            let resp = ui.label(egui::RichText::new(line).color(col).small());
                            if let Some(err) = &r.error_message {
                                resp.on_hover_text(err);
                            } else if let Some(se) = &r.stderr_excerpt {
                                if !se.is_empty() {
                                    resp.on_hover_text(se);
                                }
                            }
                        }
                    });
                });
        }

        // ── machines panel (web MachinesPanel) ──
        if self.machines_open {
            let mut open = true;
            let mut run_machine: Option<String> = None;
            let primary_id = self.primary.map(|i| self.nodes[i].id.clone());
            egui::Window::new("Machines")
                .open(&mut open)
                .default_width(360.0)
                .show(ctx, |ui| {
                    ui.label(
                        egui::RichText::new(
                            "Run a multi-step machine on the selected entity.",
                        )
                        .weak()
                        .small(),
                    );
                    ui.separator();
                    if !self.machines_loaded {
                        ui.label("loading machines…");
                    } else if self.machines.is_empty() {
                        ui.label(egui::RichText::new("No machines installed.").weak());
                    }
                    egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                        let machines = self.machines.clone();
                        for m in &machines {
                            ui.horizontal(|ui| {
                                let run = ui
                                    .add_enabled(
                                        primary_id.is_some() && !self.running,
                                        egui::Button::new("▶"),
                                    )
                                    .on_hover_text("Run on selected entity")
                                    .clicked();
                                ui.label(egui::RichText::new(&m.name).strong());
                                ui.label(
                                    egui::RichText::new(format!("{} steps", m.step_count))
                                        .weak()
                                        .small(),
                                );
                                if run {
                                    run_machine = Some(m.id.clone());
                                }
                            });
                            if !m.description.is_empty() {
                                ui.label(egui::RichText::new(&m.description).weak().small());
                            }
                            ui.separator();
                        }
                    });
                });
            if let (Some(mid), Some(eid)) = (run_machine, primary_id) {
                self.begin_machine(mid, eid);
            }
            self.machines_open = open;
        }

        // ── settings panel (web SettingsPanel — Secrets + Types tabs) ──
        if self.settings_open {
            let mut open = true;
            let mut reload_after = false;
            let mut save_network = false;
            let mut save_webhooks = false;
            let mut test_webhooks = false;
            egui::Window::new("Settings")
                .open(&mut open)
                .default_width(420.0)
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut self.settings_tab, SettingsTab::Secrets, "Secrets");
                        ui.selectable_value(&mut self.settings_tab, SettingsTab::Types, "Entity types");
                        ui.selectable_value(&mut self.settings_tab, SettingsTab::Network, "Network");
                        ui.selectable_value(&mut self.settings_tab, SettingsTab::Webhooks, "Webhooks");
                    });
                    ui.separator();
                    match self.settings_tab {
                        SettingsTab::Secrets => {
                            ui.label(
                                egui::RichText::new("API keys for sidecars (stored in the OS keyring).")
                                    .weak()
                                    .small(),
                            );
                            let mut delete: Option<String> = None;
                            egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                                let secrets = self.secrets.clone();
                                for s in &secrets {
                                    ui.horizontal(|ui| {
                                        ui.label(egui::RichText::new(&s.name).monospace());
                                        ui.label(
                                            egui::RichText::new(if s.set { "set" } else { "—" })
                                                .weak()
                                                .small(),
                                        );
                                        if s.set && ui.small_button("x").clicked() {
                                            delete = Some(s.name.clone());
                                        }
                                    });
                                }
                            });
                            if let Some(name) = delete {
                                let _ = app_lib::delete_secret(&self.state, &name);
                                self.secrets = app_lib::list_secret_names(&self.state);
                            }
                            ui.separator();
                            ui.horizontal(|ui| {
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.secret_name)
                                        .hint_text("KEY_NAME")
                                        .desired_width(150.0),
                                );
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.secret_value)
                                        .hint_text("value")
                                        .password(true)
                                        .desired_width(150.0),
                                );
                                if ui.button("Save").clicked() {
                                    let n = self.secret_name.trim().to_string();
                                    let v = self.secret_value.clone();
                                    if !n.is_empty()
                                        && app_lib::set_secret(&self.state, &n, &v).is_ok()
                                    {
                                        self.secret_name.clear();
                                        self.secret_value.clear();
                                        self.secrets = app_lib::list_secret_names(&self.state);
                                    }
                                }
                            });
                        }
                        SettingsTab::Types => {
                            ui.label(
                                egui::RichText::new("Custom entity types (canonical ontology).")
                                    .weak()
                                    .small(),
                            );
                            egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
                                let types = self.types.clone();
                                for (id, label) in &types {
                                    ui.horizontal(|ui| {
                                        ui.label(label);
                                        ui.label(egui::RichText::new(id).weak().small());
                                        if ui.small_button("x").clicked() {
                                            let _ = app_lib::delete_custom_entity_type(&self.state, id);
                                            reload_after = true;
                                        }
                                    });
                                }
                            });
                            ui.separator();
                            ui.horizontal(|ui| {
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.new_type_label)
                                        .hint_text("Type label")
                                        .desired_width(130.0),
                                );
                                egui::ComboBox::from_id_salt("type-kind")
                                    .selected_text(self.new_type_kind.clone())
                                    .show_ui(ui, |ui| {
                                        for k in
                                            ["identity", "asset", "evidence", "threat", "generic"]
                                        {
                                            ui.selectable_value(
                                                &mut self.new_type_kind,
                                                k.to_string(),
                                                k,
                                            );
                                        }
                                    });
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.new_type_color)
                                        .hint_text("#rrggbb")
                                        .desired_width(70.0),
                                );
                                if ui.button("Add").clicked() {
                                    let l = self.new_type_label.trim().to_string();
                                    if !l.is_empty()
                                        && app_lib::create_custom_entity_type(
                                            &self.state,
                                            &l,
                                            &self.new_type_kind,
                                            &self.new_type_color,
                                        )
                                        .is_ok()
                                    {
                                        self.new_type_label.clear();
                                        reload_after = true;
                                    }
                                }
                            });
                        }
                        SettingsTab::Network => {
                            ui.label(
                                egui::RichText::new(
                                    "Default proxy (e.g. socks5://127.0.0.1:9050) and rate limits applied to all sidecars.",
                                )
                                .weak()
                                .small(),
                            );
                            ui.add_space(4.0);
                            ui.label(egui::RichText::new("Default proxy").weak().small());
                            ui.add(
                                egui::TextEdit::singleline(&mut self.proxy_buf)
                                    .hint_text("(none)")
                                    .desired_width(f32::INFINITY),
                            );
                            ui.add_space(6.0);
                            ui.horizontal(|ui| {
                                ui.label("Max concurrent");
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.rl_concurrent)
                                        .desired_width(50.0),
                                );
                                ui.label("Min interval (ms)");
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.rl_interval)
                                        .desired_width(60.0),
                                );
                            });
                            ui.add_space(8.0);
                            if ui.button("Save network settings").clicked() {
                                save_network = true;
                            }
                        }
                        SettingsTab::Webhooks => {
                            ui.label(
                                egui::RichText::new("Webhook URLs (one per line). Each event POSTs to all.")
                                    .weak()
                                    .small(),
                            );
                            ui.add(
                                egui::TextEdit::multiline(&mut self.webhooks_buf)
                                    .hint_text("https://hooks.example.com/…")
                                    .desired_rows(5)
                                    .desired_width(f32::INFINITY),
                            );
                            ui.horizontal(|ui| {
                                if ui.button("Save webhooks").clicked() {
                                    save_webhooks = true;
                                }
                                if ui.button("Send test").clicked() {
                                    test_webhooks = true;
                                }
                            });
                        }
                    }
                });
            if save_network {
                let cfg = app_lib::ProxyConfig {
                    default_proxy: self.proxy_buf.trim().to_string(),
                    per_sidecar: std::collections::HashMap::new(),
                };
                let _ = app_lib::set_proxy_config(&self.state, cfg);
                let limits = app_lib::Limits {
                    max_concurrent: self.rl_concurrent.trim().parse().unwrap_or(4),
                    min_interval_ms: self.rl_interval.trim().parse().unwrap_or(200),
                };
                let rlcfg = app_lib::RateLimitConfig {
                    default: Some(limits),
                    per_sidecar: std::collections::HashMap::new(),
                };
                let _ = app_lib::set_rate_limit_config(&self.state, rlcfg);
                self.status = "network settings saved".into();
            }
            if save_webhooks {
                let urls: Vec<String> = self
                    .webhooks_buf
                    .lines()
                    .map(|l| l.trim().to_string())
                    .filter(|l| !l.is_empty())
                    .collect();
                let _ = app_lib::set_webhook_config(&self.state, app_lib::WebhookConfig { urls });
                self.status = "webhooks saved".into();
            }
            if test_webhooks {
                let state = self.state.clone();
                let n = self.rt.block_on(async { app_lib::test_webhooks(&state).await });
                self.status = match n {
                    Ok(c) => format!("fired {c} webhook(s)"),
                    Err(e) => format!("webhook test failed: {e}"),
                };
            }
            if reload_after {
                self.reload();
            }
            self.settings_open = open;
        }

        // ── AI analyst panel (web AiPanel) ──
        if self.ai_open {
            let mut open = true;
            let mut send = false;
            egui::Window::new("AI analyst")
                .open(&mut open)
                .default_width(420.0)
                .show(ctx, |ui| {
                    let configured =
                        self.ai_status.as_ref().map(|s| s.configured).unwrap_or(false);
                    if !configured {
                        ui.label(
                            egui::RichText::new(
                                "ANTHROPIC_API_KEY not set — chat is disabled.",
                            )
                            .color(egui::Color32::from_rgb(0xf6, 0xb0, 0x73)),
                        );
                    }
                    if let Some(s) = &self.ai_status {
                        ui.label(egui::RichText::new(format!("model: {}", s.model)).weak().small());
                    }
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut self.ai_include_graph, "Graph context");
                        ui.checkbox(&mut self.ai_allow_tools, "Allow tools")
                            .on_hover_text("Let Claude run transforms / search the graph autonomously");
                    });
                    ui.separator();
                    egui::ScrollArea::vertical().max_height(280.0).stick_to_bottom(true).show(
                        ui,
                        |ui| {
                            for t in &self.ai_history {
                                let user = t.role == "user";
                                let (who, accent) = if user {
                                    ("You", pal::INFO)
                                } else {
                                    ("Claude", pal::ACCENT)
                                };
                                egui::Frame::none()
                                    .fill(if user { fade(pal::INFO, 0.08) } else { pal::BG3 })
                                    .stroke(egui::Stroke::new(
                                        1.0,
                                        if user { fade(pal::INFO, 0.25) } else { pal::BORDER },
                                    ))
                                    .rounding(egui::Rounding::same(8.0))
                                    .inner_margin(egui::Margin::symmetric(10.0, 7.0))
                                    .outer_margin(egui::Margin {
                                        left: if user { 28.0 } else { 0.0 },
                                        right: if user { 0.0 } else { 28.0 },
                                        top: 0.0,
                                        bottom: 6.0,
                                    })
                                    .show(ui, |ui| {
                                        ui.label(
                                            egui::RichText::new(who).color(accent).strong().size(10.0),
                                        );
                                        ui.label(&t.content);
                                    });
                            }
                            if self.ai_busy {
                                ui.add_space(2.0);
                                ui.horizontal(|ui| {
                                    ui.spinner();
                                    ui.label(
                                        egui::RichText::new("Claude is thinking…")
                                            .color(pal::MUTED),
                                    );
                                });
                            }
                        },
                    );
                    ui.separator();
                    ui.horizontal(|ui| {
                        let te = ui.add(
                            egui::TextEdit::multiline(&mut self.ai_input)
                                .hint_text("Ask about this case…")
                                .desired_rows(2)
                                .desired_width(320.0),
                        );
                        let enter = te.lost_focus()
                            && ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.shift);
                        if (ui.add_enabled(configured && !self.ai_busy, egui::Button::new("Send")).clicked()
                            || enter)
                            && !self.ai_input.trim().is_empty()
                        {
                            send = true;
                        }
                    });
                });
            if send {
                self.begin_ai_chat();
            }
            self.ai_open = open;
        }

        // ── bookmarks panel (web BookmarksPanel) ──
        if self.bookmarks_open {
            let mut open = true;
            let mut focus: Option<String> = None;
            egui::Window::new("Bookmarks")
                .open(&mut open)
                .default_width(300.0)
                .show(ctx, |ui| {
                    let marked: Vec<(usize, String, String)> = self
                        .nodes
                        .iter()
                        .enumerate()
                        .filter(|(_, n)| n.bookmarked)
                        .map(|(i, n)| (i, n.label.clone(), n.type_label.clone()))
                        .collect();
                    ui.label(egui::RichText::new(format!("{} bookmarked", marked.len())).weak());
                    ui.separator();
                    egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                        for (i, label, ty) in marked {
                            if ui
                                .button(format!("★ {label}"))
                                .on_hover_text(ty)
                                .clicked()
                            {
                                focus = Some(self.nodes[i].id.clone());
                            }
                        }
                    });
                });
            if let Some(id) = focus {
                if let Some(i) = self.nodes.iter().position(|n| n.id == id) {
                    self.selected.clear();
                    self.selected.insert(i);
                    self.primary = Some(i);
                    self.center_on(i);
                }
            }
            self.bookmarks_open = open;
        }

        // ── audit-log panel (web AuditPanel) ──
        if self.audit_open {
            let mut open = true;
            egui::Window::new("Audit log")
                .open(&mut open)
                .default_width(420.0)
                .show(ctx, |ui| {
                    ui.label(
                        egui::RichText::new("Append-only record of case mutations.")
                            .weak()
                            .small(),
                    );
                    ui.separator();
                    egui::ScrollArea::vertical().max_height(380.0).show(ui, |ui| {
                        if self.audit.is_empty() {
                            ui.label(egui::RichText::new("No audit events.").weak());
                        }
                        for a in &self.audit {
                            let when = a.created_at.format("%Y-%m-%d %H:%M");
                            ui.label(
                                egui::RichText::new(format!("{when}  {}", a.kind)).small(),
                            );
                            if let Some(p) = &a.payload {
                                ui.label(egui::RichText::new(p).weak().small());
                            }
                        }
                    });
                });
            self.audit_open = open;
        }

        // ── import dialog (web ImportPanel — bulk paste) ──
        if self.import_open {
            let mut open = true;
            let mut do_import = false;
            let mut do_graphml = false;
            egui::Window::new("Import entities")
                .open(&mut open)
                .default_width(360.0)
                .show(ctx, |ui| {
                    ui.label(
                        egui::RichText::new("Bulk: one value per line (# comments allowed).")
                            .weak()
                            .small(),
                    );
                    egui::ComboBox::from_label("Type")
                        .selected_text(self.import_type.clone())
                        .show_ui(ui, |ui| {
                            let types = self.types.clone();
                            for (tid, tlabel) in &types {
                                ui.selectable_value(&mut self.import_type, tid.clone(), tlabel);
                            }
                        });
                    ui.add(
                        egui::TextEdit::multiline(&mut self.import_text)
                            .hint_text("example.com\n8.8.8.8\n…")
                            .desired_rows(8)
                            .desired_width(f32::INFINITY),
                    );
                    if ui.button("Import").clicked() {
                        do_import = true;
                    }
                    ui.separator();
                    ui.label(
                        egui::RichText::new("Or import a GraphML file by path:").weak().small(),
                    );
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.graphml_path)
                                .hint_text("/path/to/case.graphml")
                                .desired_width(180.0),
                        );
                        if ui.button("Browse…").clicked() {
                            if let Some(p) = pick_file(&[("GraphML", &["graphml", "xml"])]) {
                                self.graphml_path = p;
                            }
                        }
                        if ui.button("Import").clicked() {
                            do_graphml = true;
                        }
                    });
                });
            if do_import {
                self.do_bulk_import();
            }
            if do_graphml {
                self.do_graphml_import();
            }
            self.import_open = open;
        }

        // ── export menu (web CaseTabs export submenu) ──
        if self.export_open {
            let mut open = true;
            let mut chosen: Option<&'static str> = None;
            egui::Window::new("Export case")
                .open(&mut open)
                .default_width(260.0)
                .show(ctx, |ui| {
                    ui.label(
                        egui::RichText::new("Writes to your home directory.").weak().small(),
                    );
                    ui.separator();
                    if ui.button("GraphML (.graphml)").clicked() {
                        chosen = Some("graphml");
                    }
                    if ui.button("Entities CSV (.csv)").clicked() {
                        chosen = Some("csv");
                    }
                    if ui.button("STIX bundle (.json)").clicked() {
                        chosen = Some("stix");
                    }
                    if ui.button("HTML report (.html)").clicked() {
                        chosen = Some("report");
                    }
                });
            if let Some(fmt) = chosen {
                self.do_export(fmt);
                self.export_open = false;
            } else {
                self.export_open = open;
            }
        }

        // ── annotation editor (web AnnotationEditor) ──
        if self.editing_annotation.is_some() {
            let mut open = true;
            let mut save = false;
            let mut delete = false;
            egui::Window::new("Edit note")
                .open(&mut open)
                .default_width(280.0)
                .show(ctx, |ui| {
                    if let Some((_, buf)) = self.editing_annotation.as_mut() {
                        ui.add(
                            egui::TextEdit::multiline(buf)
                                .desired_rows(4)
                                .desired_width(f32::INFINITY),
                        );
                    }
                    ui.horizontal(|ui| {
                        save = ui.button("Save").clicked();
                        if ui
                            .button(
                                egui::RichText::new("Delete")
                                    .color(egui::Color32::from_rgb(0xf8, 0x71, 0x71)),
                            )
                            .clicked()
                        {
                            delete = true;
                        }
                    });
                });
            if let Some((aid, buf)) = self.editing_annotation.clone() {
                if save {
                    let _ = app_lib::update_annotation(
                        &self.state,
                        &aid,
                        Some(&buf),
                        None,
                        None,
                        None,
                    );
                    if let Some(a) = self.annotations.iter_mut().find(|a| a.id == aid) {
                        a.text = buf;
                    }
                    self.editing_annotation = None;
                } else if delete {
                    let _ = app_lib::delete_annotation(&self.state, &aid);
                    self.annotations.retain(|a| a.id != aid);
                    self.editing_annotation = None;
                } else if !open {
                    self.editing_annotation = None;
                }
            }
        }

        // ── snapshots panel (web SnapshotsPanel) ──
        if self.snapshots_open {
            let mut open = true;
            let mut create = false;
            let mut delete: Option<i64> = None;
            egui::Window::new("Snapshots")
                .open(&mut open)
                .default_width(340.0)
                .show(ctx, |ui| {
                    ui.label(
                        egui::RichText::new("Point-in-time captures of this case.")
                            .weak()
                            .small(),
                    );
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.snapshot_name)
                                .hint_text("snapshot name")
                                .desired_width(180.0),
                        );
                        if ui.button("Create").clicked() {
                            create = true;
                        }
                    });
                    ui.separator();
                    egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                        let snaps = self.snapshots.clone();
                        for s in &snaps {
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new(&s.name).strong());
                                ui.label(
                                    egui::RichText::new(format!(
                                        "{} ent · {} edg",
                                        s.entity_count, s.edge_count
                                    ))
                                    .weak()
                                    .small(),
                                );
                                if ui.small_button("x").clicked() {
                                    delete = Some(s.id);
                                }
                            });
                        }
                    });
                });
            if create {
                let name = if self.snapshot_name.trim().is_empty() {
                    "snapshot".to_string()
                } else {
                    self.snapshot_name.trim().to_string()
                };
                if app_lib::create_case_snapshot(&self.state, self.case_id, &name).is_ok() {
                    self.snapshot_name.clear();
                    self.snapshots =
                        app_lib::list_case_snapshots(&self.state, self.case_id).unwrap_or_default();
                }
            }
            if let Some(id) = delete {
                let _ = app_lib::delete_case_snapshot(&self.state, id);
                self.snapshots.retain(|s| s.id != id);
            }
            self.snapshots_open = open;
        }

        // ── saved searches panel (label-filter presets) ──
        if self.searches_open {
            let mut open = true;
            let mut save = false;
            let mut apply: Option<String> = None;
            let mut delete: Option<i64> = None;
            egui::Window::new("Saved searches")
                .open(&mut open)
                .default_width(320.0)
                .show(ctx, |ui| {
                    ui.label(
                        egui::RichText::new("Save the current Find filter for one-click recall.")
                            .weak()
                            .small(),
                    );
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.search_save_name)
                                .hint_text("name")
                                .desired_width(140.0),
                        );
                        if ui
                            .add_enabled(!self.search.trim().is_empty(), egui::Button::new("Save current"))
                            .on_hover_text(format!("filter = \"{}\"", self.search))
                            .clicked()
                        {
                            save = true;
                        }
                    });
                    ui.separator();
                    egui::ScrollArea::vertical().max_height(280.0).show(ui, |ui| {
                        let searches = self.saved_searches.clone();
                        for s in &searches {
                            ui.horizontal(|ui| {
                                if ui.button(&s.name).clicked() {
                                    apply = Some(s.predicate.clone());
                                }
                                if ui.small_button("x").clicked() {
                                    delete = Some(s.id);
                                }
                            });
                        }
                    });
                });
            if save {
                // Predicate is opaque JSON to the backend; native stores a
                // minimal {"label_contains": "<find text>"} object.
                let pred = serde_json::json!({ "label_contains": self.search }).to_string();
                let name = if self.search_save_name.trim().is_empty() {
                    self.search.trim().to_string()
                } else {
                    self.search_save_name.trim().to_string()
                };
                if app_lib::create_saved_search(&self.state, self.case_id, &name, &pred).is_ok() {
                    self.search_save_name.clear();
                    self.saved_searches =
                        app_lib::list_saved_searches(&self.state, self.case_id).unwrap_or_default();
                }
            }
            if let Some(pred) = apply {
                // Pull label_contains back out and drop it into the Find box.
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&pred) {
                    if let Some(s) = v.get("label_contains").and_then(|x| x.as_str()) {
                        self.search = s.to_string();
                    }
                }
            }
            if let Some(id) = delete {
                let _ = app_lib::delete_saved_search(&self.state, id);
                self.saved_searches.retain(|s| s.id != id);
            }
            self.searches_open = open;
        }

        // ── notifications panel (web NotificationsBell) ──
        if self.notifications_open {
            let mut open = true;
            egui::Window::new("Notifications")
                .open(&mut open)
                .default_width(360.0)
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                        if self.notifications.is_empty() {
                            ui.label(egui::RichText::new("No notifications.").weak());
                        }
                        for n in &self.notifications {
                            ui.label(egui::RichText::new(&n.title).strong());
                            if let Some(d) = &n.detail {
                                ui.label(egui::RichText::new(d).weak().small());
                            }
                            ui.separator();
                        }
                    });
                });
            self.notifications_open = open;
        }

        // ── plugins panel (web PluginsPopover) ──
        if self.plugins_open {
            let mut open = true;
            egui::Window::new("Plugins")
                .open(&mut open)
                .default_width(380.0)
                .show(ctx, |ui| {
                    ui.label(
                        egui::RichText::new("Installed sidecars contributing transforms.")
                            .weak()
                            .small(),
                    );
                    ui.separator();
                    egui::ScrollArea::vertical().max_height(380.0).show(ui, |ui| {
                        if self.plugins.is_empty() {
                            ui.label(egui::RichText::new("No plugins found.").weak());
                        }
                        for p in &self.plugins {
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new(&p.name).strong());
                                ui.label(egui::RichText::new(format!("v{}", p.version)).weak().small());
                                if !p.runnable {
                                    ui.label(
                                        egui::RichText::new("⚠ venv not set up")
                                            .color(egui::Color32::from_rgb(0xf6, 0xb0, 0x73))
                                            .small(),
                                    );
                                } else if let Some(n) = p.transform_count {
                                    ui.label(egui::RichText::new(format!("{n} transforms")).weak().small());
                                }
                            });
                            if !p.description.is_empty() {
                                ui.label(egui::RichText::new(&p.description).weak().small());
                            }
                            ui.separator();
                        }
                    });
                });
            self.plugins_open = open;
        }

        // ── image gallery: a dedicated home for images (thumbnail grid +
        //    detail pane with AI analysis) so they don't live in the inspector ──
        if self.gallery_open {
            let mut open = true;
            // (entity_id, label, path) for every Image entity in the case.
            let images: Vec<(String, String, String)> = self
                .nodes
                .iter()
                .filter(|n| n.entity_type == "Image")
                .map(|n| (n.id.clone(), n.label.clone(), image_path_for(n)))
                .collect();
            // Default the detail selection to the first image.
            if self.gallery_selected.is_none() {
                self.gallery_selected = images.first().map(|(id, _, _)| id.clone());
            }
            let ctxc = ctx.clone();
            let mut want_analyze: Option<String> = None;
            let mut add_image_now = false;
            egui::Window::new("🖼  Image gallery")
                .open(&mut open)
                .default_size(egui::vec2(720.0, 520.0))
                .show(ctx, |ui| {
                    if images.is_empty() {
                        ui.add_space(12.0);
                        ui.vertical_centered(|ui| {
                            ui.label(egui::RichText::new("No images in this case yet.").color(pal::MUTED));
                            ui.add_space(6.0);
                            ui.label(
                                egui::RichText::new("Add one below, then analyze it with AI.")
                                    .color(pal::MUTED2)
                                    .small(),
                            );
                        });
                    }
                    // Add-image row (also available here, not just the Add window).
                    ui.horizontal(|ui| {
                        if ui.button("📁 Browse…").clicked() {
                            if let Some(p) = pick_file(&[("Images", &["jpg", "jpeg", "png", "gif", "webp", "bmp"])]) {
                                self.image_path = p;
                            }
                        }
                        ui.add(
                            egui::TextEdit::singleline(&mut self.image_path)
                                .hint_text("/path/to/photo.jpg")
                                .desired_width(220.0),
                        );
                        if ui.button("📷 Add image").clicked() {
                            add_image_now = true;
                        }
                    });
                    ui.separator();

                    ui.horizontal_top(|ui| {
                        // ── left: thumbnail grid ──
                        egui::ScrollArea::vertical()
                            .id_salt("gal-grid")
                            .max_width(260.0)
                            .show(ui, |ui| {
                                ui.horizontal_wrapped(|ui| {
                                    for (id, label, path) in &images {
                                        let selected = self.gallery_selected.as_deref() == Some(id);
                                        let tex = self.gallery_thumb(&ctxc, id, path);
                                        let resp = if let Some(tex) = tex {
                                            ui.add(
                                                egui::ImageButton::new(
                                                    egui::Image::new(&tex)
                                                        .fit_to_exact_size(egui::vec2(72.0, 72.0))
                                                        .rounding(egui::Rounding::same(6.0)),
                                                )
                                                .selected(selected),
                                            )
                                        } else {
                                            ui.add_sized([72.0, 72.0], egui::Button::new("?"))
                                        };
                                        resp.clone().on_hover_text(label);
                                        if resp.clicked() {
                                            self.gallery_selected = Some(id.clone());
                                        }
                                    }
                                });
                            });

                        ui.separator();

                        // ── right: detail pane for the selected image ──
                        egui::ScrollArea::vertical().id_salt("gal-detail").show(ui, |ui| {
                            let sel = self.gallery_selected.clone();
                            if let Some(sel_id) = sel {
                                if let Some((id, label, path)) =
                                    images.iter().find(|(id, _, _)| id == &sel_id)
                                {
                                    ui.label(egui::RichText::new(label).strong().size(14.0));
                                    ui.add_space(6.0);
                                    if let Some(tex) = self.gallery_thumb(&ctxc, id, path) {
                                        let avail = ui.available_width().min(380.0).max(120.0);
                                        let aspect =
                                            tex.size()[1] as f32 / tex.size()[0] as f32;
                                        ui.add(
                                            egui::Image::new(&tex)
                                                .fit_to_exact_size(egui::vec2(avail, avail * aspect))
                                                .rounding(egui::Rounding::same(8.0)),
                                        );
                                    }
                                    ui.add_space(8.0);
                                    let ai_ready = self
                                        .ai_status
                                        .get_or_insert_with(|| app_lib::ai_status(&self.state))
                                        .configured;
                                    ui.horizontal(|ui| {
                                        if ui
                                            .add_enabled(
                                                ai_ready && !self.analyzing,
                                                egui::Button::new("✨ Analyze with AI"),
                                            )
                                            .on_hover_text(if ai_ready {
                                                "Claude vision OSINT analysis (saved to the node's notes)"
                                            } else {
                                                "Set ANTHROPIC_API_KEY to enable"
                                            })
                                            .clicked()
                                        {
                                            want_analyze = Some(id.clone());
                                        }
                                        if self.analyzing {
                                            ui.spinner();
                                        }
                                    });
                                    if !ai_ready {
                                        ui.label(
                                            egui::RichText::new("ANTHROPIC_API_KEY not set.")
                                                .color(pal::WARNING)
                                                .small(),
                                        );
                                    }
                                    // Show the analysis result for THIS image.
                                    if let Some((rid, text)) = &self.analysis_result {
                                        if rid == id {
                                            ui.add_space(6.0);
                                            egui::Frame::none()
                                                .fill(pal::BG3)
                                                .stroke(egui::Stroke::new(1.0, fade(pal::ACCENT, 0.3)))
                                                .rounding(egui::Rounding::same(8.0))
                                                .inner_margin(egui::Margin::same(8.0))
                                                .show(ui, |ui| {
                                                    ui.label(
                                                        egui::RichText::new("AI analysis")
                                                            .color(pal::ACCENT)
                                                            .strong()
                                                            .size(10.0),
                                                    );
                                                    ui.label(egui::RichText::new(text).size(12.0));
                                                });
                                        }
                                    }
                                }
                            } else {
                                ui.label(egui::RichText::new("Select an image.").color(pal::MUTED));
                            }
                        });
                    });
                });
            if add_image_now {
                self.do_add_image();
                self.reload();
            }
            if let Some(eid) = want_analyze {
                // Gate the upload behind an explicit OPSEC consent (once per
                // session). The image leaves the device — the user must opt in.
                if self.ai_upload_consented {
                    self.begin_analyze(eid, true);
                } else {
                    self.confirm_analyze = Some(eid);
                }
            }
            self.gallery_open = open;
        }

        // ── OPSEC consent dialog: sending evidence off-device to a third
        //    party. Modal; nothing leaves the machine until the user agrees. ──
        if let Some(eid) = self.confirm_analyze.clone() {
            let mut decided: Option<bool> = None;
            egui::Window::new("⚠  Send image off-device?")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .show(ctx, |ui| {
                    ui.set_max_width(400.0);
                    ui.label(
                        egui::RichText::new(
                            "AI analysis uploads this image to Anthropic's servers (a third party) \
                             for processing. The image — and anything visible in it — leaves this \
                             machine and may be retained per Anthropic's policies.",
                        )
                        .color(pal::TEXT_SOFT),
                    );
                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new("This is logged to the case audit trail.")
                            .color(pal::MUTED)
                            .small(),
                    );
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new("Send & analyze").color(pal::TEXT),
                                )
                                .fill(fade(pal::WARNING, 0.18))
                                .stroke(egui::Stroke::new(1.0, fade(pal::WARNING, 0.55))),
                            )
                            .clicked()
                        {
                            decided = Some(true);
                        }
                        if ui.button("Cancel").clicked() {
                            decided = Some(false);
                        }
                        ui.checkbox(
                            &mut self.ai_upload_consented,
                            "Don't ask again this session",
                        );
                    });
                });
            if let Some(go) = decided {
                self.confirm_analyze = None;
                if go {
                    self.begin_analyze(eid, true);
                }
            }
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(pal::BG0))
            .show(ctx, |ui| {
                if self.view == View::Map {
                    self.draw_map(ui);
                    return;
                }
                if self.view == View::Timeline {
                    self.draw_timeline(ui);
                    return;
                }
                let (resp, painter) =
                    ui.allocate_painter(ui.available_size(), egui::Sense::click_and_drag());
                let rect = resp.rect;
                self.last_canvas = rect;
                let mut annotation_to_delete: Option<String> = None;

                if !self.fitted {
                    self.fit(rect);
                    self.fitted = true;
                }

                let pointer = resp.hover_pos();
                let hovered = pointer.and_then(|p| self.node_at(p));

                // ── zoom (wheel / pinch), anchored at the pointer ──
                if resp.hovered() {
                    let (scroll, zoom_d) =
                        ctx.input(|i| (i.raw_scroll_delta.y, i.zoom_delta()));
                    let factor = zoom_d * (1.0 + scroll * 0.0015);
                    if (factor - 1.0).abs() > f32::EPSILON {
                        let anchor = pointer.unwrap_or(rect.center());
                        let world_before = (anchor - self.offset) / self.zoom;
                        self.zoom = (self.zoom * factor).clamp(0.1, 4.0);
                        self.offset = anchor - world_before * self.zoom;
                    }
                }

                let shift = ui.input(|i| i.modifiers.shift);

                // ── drag: node move · box-select (shift) · background pan ──
                if resp.drag_started() {
                    if let Some(idx) = pointer.and_then(|p| self.node_at(p)) {
                        if self.nodes[idx].locked {
                            self.panning = true;
                        } else {
                            self.dragging_node = Some(idx);
                            if !self.selected.contains(&idx) {
                                self.selected.clear();
                                self.selected.insert(idx);
                            }
                            self.primary = Some(idx);
                        }
                    } else if shift {
                        self.box_start = pointer;
                    } else {
                        self.panning = true;
                    }
                }
                if resp.dragged() {
                    let d = resp.drag_delta();
                    if let Some(idx) = self.dragging_node {
                        self.nodes[idx].pos += d / self.zoom;
                    } else if self.box_start.is_none() && self.panning {
                        self.offset += d;
                    }
                }
                if resp.drag_stopped() {
                    if let Some(idx) = self.dragging_node.take() {
                        let (id, p) = (self.nodes[idx].id.clone(), self.nodes[idx].pos);
                        if let Err(e) = app_lib::update_entity_position(
                            &self.state,
                            &id,
                            p.x as f64,
                            p.y as f64,
                        ) {
                            self.status = format!("save position failed: {e}");
                        }
                    }
                    // finish a box-select: select every node inside the rect
                    if let (Some(start), Some(end)) = (self.box_start.take(), pointer) {
                        let rect = egui::Rect::from_two_pos(start, end);
                        if !shift {
                            self.selected.clear();
                        }
                        for i in 0..self.nodes.len() {
                            if self.visible(i) && rect.contains(self.world_to_screen(self.nodes[i].pos)) {
                                self.selected.insert(i);
                            }
                        }
                        self.primary = self.selected.iter().copied().next();
                    }
                    self.panning = false;
                }

                // ── click to (de)select; shift toggles (and dismiss menu) ──
                if resp.clicked() {
                    let hit = pointer.and_then(|p| self.node_at(p));
                    match hit {
                        Some(idx) => {
                            if shift {
                                if !self.selected.remove(&idx) {
                                    self.selected.insert(idx);
                                }
                            } else {
                                self.selected.clear();
                                self.selected.insert(idx);
                            }
                            self.primary = Some(idx);
                            self.selected_edge = None;
                        }
                        None => {
                            // No node — try an edge before clearing.
                            let edge_hit = pointer.and_then(|p| self.edge_at(p));
                            if let Some(ei) = edge_hit {
                                self.selected_edge = Some(ei);
                                self.edge_rel_buf =
                                    self.edges[ei].relationship_type.clone().unwrap_or_default();
                                self.selected.clear();
                                self.primary = None;
                            } else if !shift {
                                self.selected.clear();
                                self.primary = None;
                                self.selected_edge = None;
                            }
                        }
                    }
                    self.ctx_menu = None;
                }

                // ── right-click a node → open the pivot (run-transform) menu ──
                if resp.secondary_clicked() {
                    self.ctx_menu = pointer.and_then(|p| {
                        self.node_at(p).map(|idx| {
                            if !self.selected.contains(&idx) {
                                self.selected.clear();
                                self.selected.insert(idx);
                            }
                            self.primary = Some(idx);
                            (idx, p)
                        })
                    });
                }

                // ── focus set (for dimming): hovered node + neighbours, or,
                //    when nothing is hovered, the find-box matches ──
                let query = self.search.trim().to_lowercase();
                let focus: Option<HashSet<usize>> = if let Some(h) = hovered {
                    let mut s = self.neighbors[h].clone();
                    s.insert(h);
                    Some(s)
                } else if !query.is_empty() {
                    Some(
                        self.nodes
                            .iter()
                            .enumerate()
                            .filter(|(_, n)| n.label.to_lowercase().contains(&query))
                            .map(|(i, _)| i)
                            .collect(),
                    )
                } else {
                    None
                };
                let dim = |i: usize| matches!(&focus, Some(set) if !set.contains(&i));

                let teal = pal::ACCENT;
                let r = self.node_screen_radius();

                // ── dot-grid background: world-anchored so it pans/zooms with
                //    the graph, giving a sense of an infinite canvas (like the
                //    web build's subtle grid). Skipped when zoomed far out so
                //    it never turns into noise. ──
                let grid_world = 56.0_f32;
                let step = grid_world * self.zoom;
                if step >= 18.0 {
                    let dot = fade(pal::TEXT, 0.05);
                    // first grid line >= rect.left, aligned to world origin
                    let start_x = rect.left()
                        - ((rect.left() - self.offset.x).rem_euclid(step));
                    let start_y = rect.top()
                        - ((rect.top() - self.offset.y).rem_euclid(step));
                    let mut gx = start_x;
                    while gx <= rect.right() {
                        let mut gy = start_y;
                        while gy <= rect.bottom() {
                            painter.circle_filled(egui::pos2(gx, gy), 1.0, dot);
                            gy += step;
                        }
                        gx += step;
                    }
                }

                // edges first, under the nodes
                for (ei, e) in self.edges.iter().enumerate() {
                    if !self.visible(e.source) || !self.visible(e.target) {
                        continue;
                    }
                    let a = self.world_to_screen(self.nodes[e.source].pos);
                    let b = self.world_to_screen(self.nodes[e.target].pos);
                    let sel = self.selected_edge == Some(ei);
                    let lit = matches!(&focus, Some(set) if set.contains(&e.source) && set.contains(&e.target));
                    let faded = matches!(&focus, Some(_)) && !lit;
                    let col = if sel {
                        egui::Color32::from_rgb(0xfb, 0xbf, 0x24)
                    } else if lit {
                        teal
                    } else if faded {
                        egui::Color32::from_rgb(0x22, 0x2c, 0x3c)
                    } else {
                        egui::Color32::from_rgb(0x33, 0x41, 0x55)
                    };
                    let width = if sel { 3.0 } else if lit { 2.2 } else { 1.3 };
                    painter.line_segment([a, b], egui::Stroke::new(width, col));
                    // typed-relationship label at the edge midpoint
                    if let Some(rt) = &e.relationship_type {
                        if self.zoom > 0.4 && !faded {
                            let mid = egui::pos2((a.x + b.x) * 0.5, (a.y + b.y) * 0.5);
                            outlined_text(&painter, mid, rt, 10.0);
                        }
                    }
                }

                // nodes
                for (i, nv) in self.nodes.iter().enumerate() {
                    if !self.visible(i) {
                        continue;
                    }
                    let c = self.world_to_screen(nv.pos);
                    let faded = dim(i);
                    let is_hov = hovered == Some(i);
                    let is_sel = self.selected.contains(&i);
                    let rr = if is_hov { r * 1.18 } else { r };

                    if faded {
                        painter.circle_filled(c, rr, fade(nv.color, 0.20));
                    } else {
                        // soft drop shadow for depth
                        painter.circle_filled(
                            c + egui::vec2(0.0, 1.5),
                            rr + 1.0,
                            egui::Color32::from_black_alpha(70),
                        );
                        if is_hov || is_sel {
                            painter.circle_filled(c, rr + 6.0, fade(teal, 0.16));
                        }
                        // Flat fill (no gloss/highlight).
                        painter.circle_filled(c, rr, nv.color);
                    }
                    let border = if is_sel || is_hov {
                        egui::Stroke::new(2.5, teal)
                    } else {
                        egui::Stroke::new(1.5, fade(pal::TEXT, 0.20))
                    };
                    painter.circle_stroke(c, rr, border);
                    // bookmarked → gold ring (matches the web build's star ring)
                    if nv.bookmarked && !faded {
                        painter.circle_stroke(c, rr + 3.0, egui::Stroke::new(2.0, pal::WARNING));
                    }

                    // label (skip when zoomed way out, or when dimmed) — a
                    // rounded translucent pill behind the text so it stays
                    // readable over edges/dots without the old 4-way outline.
                    if self.zoom > 0.33 && !faded {
                        node_label(&painter, egui::pos2(c.x, c.y + rr + 5.0), &nv.label, is_hov || is_sel);
                    }
                }

                // ── sticky-note annotations (web AnnotationEditor) ──
                let note_w = 160.0 * self.zoom.clamp(0.5, 1.5);
                for (ai, an) in self.annotations.iter().enumerate() {
                    let top_left = self.world_to_screen(egui::pos2(an.pos_x as f32, an.pos_y as f32));
                    let lines = an.text.lines().count().max(1) as f32;
                    let note_h = (18.0 + lines * 15.0) * self.zoom.clamp(0.5, 1.5);
                    let nr = egui::Rect::from_min_size(top_left, egui::vec2(note_w, note_h));
                    let bg = parse_hex(&an.color);
                    painter.rect_filled(nr, 4.0, fade(bg, 0.92));
                    painter.rect_stroke(
                        nr,
                        4.0,
                        egui::Stroke::new(1.0, egui::Color32::from_black_alpha(60)),
                    );
                    painter.text(
                        nr.min + egui::vec2(6.0, 4.0),
                        egui::Align2::LEFT_TOP,
                        &an.text,
                        egui::FontId::proportional(12.0 * self.zoom.clamp(0.6, 1.3)),
                        egui::Color32::from_rgb(0x1a, 0x1a, 0x1a),
                    );
                    // drag to move; double-click to edit
                    let id = egui::Id::new(("annot", ai));
                    let r2 = ui.interact(nr, id, egui::Sense::click_and_drag());
                    if r2.drag_started() {
                        self.dragging_annotation = Some(ai);
                    }
                    if r2.double_clicked() {
                        self.editing_annotation = Some((an.id.clone(), an.text.clone()));
                    }
                    r2.context_menu(|ui| {
                        if ui.button("Edit").clicked() {
                            self.editing_annotation = Some((an.id.clone(), an.text.clone()));
                            ui.close_menu();
                        }
                        if ui.button("Delete note").clicked() {
                            annotation_to_delete = Some(an.id.clone());
                            ui.close_menu();
                        }
                    });
                }
                if let Some(ai) = self.dragging_annotation {
                    if resp.dragged() && self.dragging_node.is_none() {
                        let d = resp.drag_delta() / self.zoom;
                        self.annotations[ai].pos_x += d.x as f64;
                        self.annotations[ai].pos_y += d.y as f64;
                    }
                    if resp.drag_stopped() {
                        let a = &self.annotations[ai];
                        let _ = app_lib::update_annotation(
                            &self.state,
                            &a.id,
                            None,
                            Some(a.pos_x),
                            Some(a.pos_y),
                            None,
                        );
                        self.dragging_annotation = None;
                    }
                }

                // box-select rubber-band
                if let (Some(start), Some(now)) = (self.box_start, pointer) {
                    let r = egui::Rect::from_two_pos(start, now);
                    painter.rect_filled(r, 0.0, fade(teal, 0.08));
                    painter.rect_stroke(r, 0.0, egui::Stroke::new(1.0, teal));
                }

                if let Some(aid) = annotation_to_delete {
                    let _ = app_lib::delete_annotation(&self.state, &aid);
                    self.annotations.retain(|a| a.id != aid);
                }
            });

        // ── right-click pivot popup: floats at the cursor. (The inspector's
        //    "Run OSINT tool…" button uses the SAME picker but renders it
        //    inline inside the inspector panel — see the inspector section.) ──
        if let Some((node, pos)) = self.ctx_menu {
            let title = self.nodes[node].label.clone();
            let mut chosen: Option<String> = None;
            let mut close = false;
            egui::Area::new(egui::Id::new("pivot-menu"))
                .order(egui::Order::Foreground)
                .fixed_pos(pos)
                .show(ctx, |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_max_width(360.0);
                        ui.label(egui::RichText::new("Run OSINT tool").strong());
                        ui.label(egui::RichText::new(title).weak().small());
                        ui.separator();
                        let (c, cl) = self.tools_picker_ui(ui, node);
                        chosen = c;
                        close = cl;
                        ui.separator();
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    });
                });
            if let Some(tid) = chosen {
                self.begin_transform(node, tid);
            }
            if close {
                self.ctx_menu = None;
                self.ctx_filter.clear();
                self.ctx_source.clear();
            }
        }

        // ── transform parameters dialog ──
        let mut dialog = self.params_dialog.take();
        let mut launch: Option<(usize, String, String)> = None;
        let mut keep_open = false;
        if let Some(dlg) = dialog.as_mut() {
            let mut do_run = false;
            let mut cancel = false;
            egui::Window::new(format!("Parameters · {}", dlg.transform_name))
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .show(ctx, |ui| {
                    ui.set_max_width(360.0);
                    for f in &dlg.fields {
                        ui.add_space(4.0);
                        ui.label(egui::RichText::new(&f.name).strong());
                        if !f.description.is_empty() {
                            ui.label(egui::RichText::new(&f.description).weak().small());
                        }
                        let val = dlg.values.entry(f.name.clone()).or_default();
                        match f.kind {
                            ParamKind::Bool => {
                                let mut b = val == "true";
                                if ui.checkbox(&mut b, "enabled").changed() {
                                    *val = b.to_string();
                                }
                            }
                            ParamKind::Choice => {
                                egui::ComboBox::from_id_source(format!("param_{}", f.name))
                                    .selected_text(val.clone())
                                    .show_ui(ui, |ui| {
                                        for c in &f.choices {
                                            ui.selectable_value(val, c.clone(), c);
                                        }
                                    });
                            }
                            _ => {
                                ui.add(
                                    egui::TextEdit::singleline(val).desired_width(f32::INFINITY),
                                );
                            }
                        }
                    }
                    ui.separator();
                    ui.horizontal(|ui| {
                        if ui.button("Run").clicked() {
                            do_run = true;
                        }
                        if ui.button("Cancel").clicked() {
                            cancel = true;
                        }
                    });
                });
            if do_run {
                launch = Some((dlg.node, dlg.transform_id.clone(), build_params_json(dlg)));
            } else if !cancel {
                keep_open = true;
            }
        }
        if keep_open {
            self.params_dialog = dialog;
        }
        if let Some((node, tid, params)) = launch {
            self.run_transform(node, tid, Some(params));
        }

        // ── command palette ──
        if self.palette_open {
            let q = self.palette_query.to_lowercase();
            let filtered: Vec<(&'static str, Action)> = PALETTE_ACTIONS
                .iter()
                .filter(|(name, _)| q.is_empty() || name.to_lowercase().contains(&q))
                .map(|(n, a)| (*n, *a))
                .collect();
            let enter = ctx.input(|i| i.key_pressed(egui::Key::Enter));
            let mut chosen: Option<Action> = None;
            egui::Window::new("command-palette")
                .title_bar(false)
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 90.0))
                .fixed_size(egui::vec2(430.0, 0.0))
                .show(ctx, |ui| {
                    let te = ui.add(
                        egui::TextEdit::singleline(&mut self.palette_query)
                            .hint_text("Type a command…")
                            .desired_width(f32::INFINITY),
                    );
                    te.request_focus();
                    ui.separator();
                    egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                        for (i, (name, act)) in filtered.iter().enumerate() {
                            if ui.selectable_label(i == 0, *name).clicked() {
                                chosen = Some(*act);
                            }
                            if enter && i == 0 {
                                chosen = Some(*act);
                            }
                        }
                    });
                });
            if let Some(a) = chosen {
                self.exec_action(a);
                self.palette_open = false;
            }
        }
    }
}

/// Build the JSON params object the sidecar expects from the dialog's values,
/// coercing each value to its declared type.
/// Build a multi-line button label for one pivot-menu transform row: the
/// friendly name (bold), an optional wrapped description (dim), and a meta
/// line with the cryptic module id and what it produces (web #3 row design).
fn transform_row_text(t: &CompatTransform) -> egui::text::LayoutJob {
    use egui::{text::LayoutJob, Color32, FontId, TextFormat};
    let mut job = LayoutJob::default();
    job.wrap.max_width = 330.0;
    job.append(
        &t.name,
        0.0,
        TextFormat {
            font_id: FontId::proportional(14.0),
            color: Color32::from_rgb(225, 230, 240),
            ..Default::default()
        },
    );
    if !t.description.is_empty() {
        let desc: String = t.description.chars().take(160).collect();
        job.append(
            &format!("\n{desc}"),
            0.0,
            TextFormat {
                font_id: FontId::proportional(11.0),
                color: Color32::from_rgb(150, 158, 172),
                ..Default::default()
            },
        );
    }
    let produces = t.produces.iter().take(3).cloned().collect::<Vec<_>>().join(", ");
    let meta = if produces.is_empty() {
        t.id.clone()
    } else {
        format!("{}  →  {produces}", t.id)
    };
    job.append(
        &format!("\n{meta}"),
        0.0,
        TextFormat {
            font_id: FontId::monospace(9.5),
            color: Color32::from_rgb(120, 128, 142),
            ..Default::default()
        },
    );
    job
}

/// A small uppercase, letter-spaced, muted section label — the native
/// equivalent of the web inspector's `<dt>`/section headings.
fn field_label(ui: &mut egui::Ui, text: &str) {
    ui.add_space(2.0);
    ui.label(
        egui::RichText::new(text.to_uppercase())
            .color(pal::MUTED)
            .size(10.0),
    );
}

/// A coloured pill badge (type chip used at the top of the inspector).
fn badge(ui: &mut egui::Ui, label: &str, color: egui::Color32) {
    let font = egui::FontId::proportional(11.0);
    let galley = ui.painter().layout_no_wrap(
        label.to_uppercase(),
        font,
        color,
    );
    let pad_x = 10.0;
    let dot_r = 4.0;
    let gap = 7.0;
    let h = 22.0;
    let w = pad_x + dot_r * 2.0 + gap + galley.size().x + pad_x;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, h), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect(
        rect,
        egui::Rounding::same(11.0),
        fade(color, 0.12),
        egui::Stroke::new(1.0, fade(color, 0.45)),
    );
    let cy = rect.center().y;
    painter.circle_filled(egui::pos2(rect.left() + pad_x + dot_r, cy), dot_r, color);
    painter.galley(
        egui::pos2(rect.left() + pad_x + dot_r * 2.0 + gap, cy - galley.size().y * 0.5),
        galley,
        color,
    );
}

/// A danger-styled button (red fill/text) for destructive actions.
fn danger_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let btn = egui::Button::new(egui::RichText::new(text).color(pal::DANGER))
        .fill(fade(pal::DANGER, 0.10))
        .stroke(egui::Stroke::new(1.0, fade(pal::DANGER, 0.45)));
    ui.add(btn)
}

/// A pill-shaped entity-type filter chip: colored dot + type label + count,
/// mirroring the React `.badge` style. `on` = type currently shown. Returns
/// the click response.
fn type_chip(
    ui: &mut egui::Ui,
    label: &str,
    color: egui::Color32,
    count: usize,
    on: bool,
) -> egui::Response {
    let text = format!("{label}  {count}");
    let font = egui::FontId::proportional(12.0);
    let galley =
        ui.painter().layout_no_wrap(text.clone(), font.clone(), egui::Color32::WHITE);
    let dot_r = 4.0;
    let pad_x = 9.0;
    let gap = 7.0;
    let h = 22.0;
    let w = pad_x + dot_r * 2.0 + gap + galley.size().x + pad_x;
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, h), egui::Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    let hovered = resp.hovered();
    let bg = if on {
        if hovered { fade(color, 0.20) } else { fade(color, 0.12) }
    } else if hovered {
        pal::BG3
    } else {
        pal::BG2
    };
    let stroke_col = if on { fade(color, 0.55) } else { pal::BORDER };
    let painter = ui.painter();
    painter.rect(rect, egui::Rounding::same(11.0), bg, egui::Stroke::new(1.0, stroke_col));
    let cy = rect.center().y;
    let dot_c = egui::pos2(rect.left() + pad_x + dot_r, cy);
    let dot_col = if on { color } else { pal::MUTED2 };
    painter.circle_filled(dot_c, dot_r, dot_col);
    let text_col = if on { pal::TEXT } else { pal::MUTED };
    painter.galley(
        egui::pos2(dot_c.x + dot_r + gap, cy - galley.size().y * 0.5),
        galley,
        text_col,
    );
    resp.on_hover_text("Click to show/hide this type")
}

/// Open a native file-picker (blocking) and return the chosen path, or None
/// if cancelled. `filters` is a list of (label, &[extensions]) tuples.
fn pick_file(filters: &[(&str, &[&str])]) -> Option<String> {
    let mut dlg = rfd::FileDialog::new();
    for (label, exts) in filters {
        dlg = dlg.add_filter(*label, exts);
    }
    dlg.pick_file().map(|p| p.to_string_lossy().into_owned())
}

/// Resolve an Image entity's on-disk file path: from the `file_path` in its
/// `data` JSON payload, or the label as a fallback (manual path entities).
fn image_path_for(node: &NodeView) -> String {
    node.data
        .as_deref()
        .and_then(|d| serde_json::from_str::<serde_json::Value>(d).ok())
        .and_then(|v| v.get("file_path").and_then(|p| p.as_str()).map(String::from))
        .unwrap_or_else(|| node.label.clone())
}

/// Home directory for file exports, falling back to the current dir.
fn dirs_home() -> String {
    std::env::var("HOME").unwrap_or_else(|_| ".".to_string())
}

/// Flatten an entity's `data` JSON payload into display key/value pairs for
/// the inspector's Properties section. Mirrors the web inspector: surface
/// `full_data`, scalar top-level keys, and any `extra` sub-object.
fn node_properties(data: Option<&str>) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let Some(raw) = data else { return out };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(raw) else {
        return out;
    };
    let scalar = |val: &serde_json::Value| -> Option<String> {
        match val {
            serde_json::Value::String(s) => Some(s.clone()),
            serde_json::Value::Number(n) => Some(n.to_string()),
            serde_json::Value::Bool(b) => Some(b.to_string()),
            _ => None,
        }
    };
    if let Some(obj) = v.as_object() {
        // Collections: summarise item count rather than dumping the array.
        if let Some(items) = obj.get("items").and_then(|i| i.as_array()) {
            out.push(("items".into(), format!("{} collected", items.len())));
        }
        for (k, val) in obj {
            if k == "extra" {
                if let Some(ex) = val.as_object() {
                    for (ek, ev) in ex {
                        if let Some(s) = scalar(ev) {
                            if !s.is_empty() {
                                out.push((ek.clone(), s));
                            }
                        }
                    }
                }
                continue;
            }
            if k == "items" {
                continue;
            }
            if let Some(s) = scalar(val) {
                if !s.is_empty() {
                    out.push((k.clone(), s));
                }
            }
        }
    }
    // De-dup keys (extra.url vs source_url etc.), keep first, cap length.
    let mut seen = std::collections::HashSet::new();
    out.retain(|(k, _)| seen.insert(k.clone()));
    for (_, val) in out.iter_mut() {
        if val.chars().count() > 200 {
            *val = val.chars().take(200).collect::<String>() + "…";
        }
    }
    out
}

fn build_params_json(dlg: &ParamsDialog) -> String {
    let mut map = serde_json::Map::new();
    for f in &dlg.fields {
        let v = dlg.values.get(&f.name).cloned().unwrap_or_default();
        let jv = match f.kind {
            ParamKind::Bool => serde_json::Value::Bool(v == "true"),
            ParamKind::Number => v
                .parse::<f64>()
                .ok()
                .and_then(serde_json::Number::from_f64)
                .map(serde_json::Value::Number)
                .unwrap_or(serde_json::Value::String(v.clone())),
            _ => serde_json::Value::String(v.clone()),
        };
        map.insert(f.name.clone(), jv);
    }
    serde_json::Value::Object(map).to_string()
}

/// White-ish label with a dark outline so it reads on any background — the
/// painter has no text-shadow, so we stamp the dark copy 4-ways then the light
/// one on top.
/// A graph node's label, drawn as text on a rounded translucent pill so it
/// stays legible over edges and the dot-grid. `lit` brightens it for the
/// hovered/selected node. `pos` is the top-center anchor below the node.
fn node_label(painter: &egui::Painter, pos: egui::Pos2, text: &str, lit: bool) {
    let font = egui::FontId::proportional(12.0);
    let max = 26usize;
    let shown: String = if text.chars().count() > max {
        format!("{}…", text.chars().take(max - 1).collect::<String>())
    } else {
        text.to_string()
    };
    let galley = painter.layout_no_wrap(shown, font, pal::TEXT);
    let pad = egui::vec2(6.0, 2.0);
    let size = galley.size() + pad * 2.0;
    let rect = egui::Rect::from_min_size(
        egui::pos2(pos.x - size.x * 0.5, pos.y),
        size,
    );
    let bg = if lit { fade(pal::BG3, 0.95) } else { fade(pal::BG1, 0.82) };
    let stroke = if lit {
        egui::Stroke::new(1.0, fade(pal::ACCENT, 0.55))
    } else {
        egui::Stroke::new(1.0, fade(pal::BORDER, 0.7))
    };
    painter.rect(rect, egui::Rounding::same(5.0), bg, stroke);
    let col = if lit { pal::TEXT } else { pal::TEXT_SOFT };
    painter.galley(rect.min + pad, galley, col);
}

#[allow(dead_code)]
fn outlined_text(painter: &egui::Painter, pos: egui::Pos2, text: &str, size: f32) {
    let font = egui::FontId::proportional(size);
    let dark = egui::Color32::from_rgb(0x04, 0x07, 0x0e);
    for off in [
        egui::vec2(-1.0, 0.0),
        egui::vec2(1.0, 0.0),
        egui::vec2(0.0, -1.0),
        egui::vec2(0.0, 1.0),
    ] {
        painter.text(pos + off, egui::Align2::CENTER_TOP, text, font.clone(), dark);
    }
    painter.text(
        pos,
        egui::Align2::CENTER_TOP,
        text,
        font,
        egui::Color32::from_rgb(0xf3, 0xf7, 0xfd),
    );
}

fn fade(c: egui::Color32, a: f32) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (a * 255.0) as u8)
}

/// Initial string value for a parameter edit field from its JSON default.
fn default_value_string(default: &Option<serde_json::Value>, kind: &ParamKind) -> String {
    match default {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Bool(b)) => b.to_string(),
        Some(serde_json::Value::Number(n)) => n.to_string(),
        _ => match kind {
            ParamKind::Bool => "false".to_string(),
            _ => String::new(),
        },
    }
}

/// Parse a `"lat,lng"` string into validated coordinates.
fn parse_latlng(s: &str) -> Option<(f64, f64)> {
    let (a, b) = s.trim().split_once(',')?;
    let lat: f64 = a.trim().parse().ok()?;
    let lng: f64 = b.trim().parse().ok()?;
    if (-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lng) {
        Some((lat, lng))
    } else {
        None
    }
}

/// Extract a (lat, lng) for an entity — mirrors the web GeoView: a bare
/// `"LAT,LNG"` label (Geolocation entities), structured `data.extra.{lat,lng |
/// latitude,longitude}`, or a `data.full_data` "LAT,LNG" string.
fn extract_geo(e: &app_lib::Entity) -> Option<(f64, f64)> {
    if let Some(ll) = parse_latlng(&e.label) {
        return Some(ll);
    }
    let data = e.data.as_ref()?;
    let v: serde_json::Value = serde_json::from_str(data).ok()?;
    let extra = v.get("extra").unwrap_or(&v);
    let pick = |k: &str| extra.get(k).and_then(|x| x.as_f64());
    if let (Some(lat), Some(lng)) = (
        pick("lat").or_else(|| pick("latitude")),
        pick("lng").or_else(|| pick("longitude")),
    ) {
        if (-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lng) {
            return Some((lat, lng));
        }
    }
    if let Some(fd) = v.get("full_data").and_then(|x| x.as_str()) {
        return parse_latlng(fd);
    }
    None
}

/// Parse `#rrggbb` / `#rgb` into a Color32; falls back to a neutral slate.
fn parse_hex(s: &str) -> egui::Color32 {
    let h = s.trim().trim_start_matches('#');
    let full = match h.len() {
        3 => h.chars().flat_map(|c| [c, c]).collect::<String>(),
        6 => h.to_string(),
        _ => return egui::Color32::from_rgb(0x64, 0x74, 0x8b),
    };
    let n = u32::from_str_radix(&full, 16).unwrap_or(0x64748b);
    egui::Color32::from_rgb((n >> 16) as u8, (n >> 8) as u8, n as u8)
}
