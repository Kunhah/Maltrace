import {
  forwardRef,
  useEffect,
  useImperativeHandle,
  useMemo,
  useRef,
} from "react";
import cytoscape, {
  type Core,
  type ElementDefinition,
  type StylesheetCSS,
} from "cytoscape";
// @ts-expect-error — no shipped types
import dagre from "cytoscape-dagre";
// @ts-expect-error — no shipped types
import fcose from "cytoscape-fcose";
import type { Annotation, CanonicalType, Edge, Entity } from "../types";

/// Layout extensions are registered once at module load. Cytoscape silently
/// ignores duplicate registrations, so the second hot-reload doesn't blow up.
let layoutsRegistered = false;
function ensureLayoutsRegistered() {
  if (layoutsRegistered) return;
  cytoscape.use(dagre);
  cytoscape.use(fcose);
  layoutsRegistered = true;
}

/// Layouts maltrace exposes. Names match Cytoscape extension ids.
///   - `preset` is the saved-positions layout (no re-positioning).
///   - `fcose` is a force-directed organic layout — the default for big
///     dense graphs because it untangles them.
///   - `dagre` is the layered hierarchical layout — the obvious choice when
///     the graph is a clear pivot-tree from one root.
///   - `concentric` / `circle` / `grid` are Maltego classics for small
///     graphs and clusters.
export type LayoutKind =
  | "preset"
  | "fcose"
  | "dagre"
  | "concentric"
  | "circle"
  | "grid";

interface Props {
  entities: Entity[];
  edges: Edge[];
  /// Annotations are rendered as Cytoscape nodes alongside entities so
  /// they pan/zoom with the graph for free. They use a distinct id
  /// namespace (the annotation's UUID with a `ann:` prefix at the
  /// cytoscape layer) so selection/find/filter can tell them apart from
  /// real entities.
  annotations?: Annotation[];
  /// Authoritative selection set, by entity id. The canvas mirrors this into
  /// Cytoscape's own selection state on every change.
  selectedIds: string[];
  canonicalTypes: CanonicalType[];
  /// Optional transform_id → sidecar source map. Currently unused by the
  /// stylesheet (per-sidecar colouring was superseded by per-transform
  /// colouring via `transformColors`) — kept on the prop in case a future
  /// view wants to group/colour by sidecar instead.
  transformSources?: Record<string, string>;
  /// Optional transform_id → hex/HSL colour map. Each edge gets its own
  /// `tcolor` data attribute, which the stylesheet reads via `data(...)`
  /// so every individual transform is visually distinguishable. Maltego's
  /// "one colour per transform" pattern.
  transformColors?: Record<string, string>;
  /// Canonical-type ids the user has toggled off in the filter strip.
  /// Nodes with these types (and their incident edges) get `display: none`
  /// — they're still in the DB, just hidden on the canvas.
  hiddenTypes?: string[];
  /// When true, every non-bookmarked node is hidden on the canvas. Edges
  /// follow incident-node visibility automatically.
  bookmarkedOnly?: boolean;
  /// Force performance mode regardless of node count. When undefined,
  /// it auto-enables past ~1000 entities. Disables animations,
  /// motion blur, edge labels, and uses a coarser texture so large
  /// graphs pan/zoom at native frame rate.
  performanceMode?: boolean;
  /// Use Cytoscape's GPU-accelerated WebGL renderer instead of the default
  /// Canvas2D one. This is the single biggest pan/zoom smoothness win on
  /// large graphs — the GPU rasterises nodes/edges from texture atlases
  /// instead of the CPU repainting every element each frame. Defaults to
  /// on; falls back to Canvas2D automatically if WebGL context creation
  /// throws (old GPUs / blocklisted drivers / headless). Changing this at
  /// runtime re-creates the Cytoscape instance (you can't hot-swap a live
  /// renderer), which resets the viewport — it's a deliberate, rare action.
  webgl?: boolean;
  /// Called whenever the canvas's selection changes (from a click, a
  /// ctrl-click, or a box-select).
  onSelectionChange: (ids: string[]) => void;
  /// Selection of annotations is reported separately so the parent can
  /// route the inspector accordingly. Mirrors `selectedIds` semantics.
  onAnnotationSelectionChange?: (ids: string[]) => void;
  /// Edge selection — single-edge at a time. Fired when the user taps
  /// an edge. Node selection takes precedence; tapping any node or the
  /// background clears the edge selection back to `null`.
  onEdgeSelectionChange?: (edgeId: string | null) => void;
  onPositionChange: (id: string, x: number, y: number) => void;
  /// Fired when the user drag-releases an annotation node; the parent
  /// persists the new position.
  onAnnotationMoved?: (id: string, x: number, y: number) => void;
  /// Right-click on a node. `clientX`/`clientY` are viewport coordinates
  /// suitable for positioning a `position: fixed` floating menu.
  onContextMenu?: (entityId: string, clientX: number, clientY: number) => void;
  /// Right-click on the background (no node). Used to dismiss any open menu.
  onContextMenuBackground?: () => void;
}

/// Imperative ops the parent triggers from the floating canvas controls and
/// keyboard shortcuts.
export interface GraphHandle {
  fit: () => void;
  center: () => void;
  /// Pan the viewport to the average position (centroid) of all visible
  /// nodes, keeping the current zoom. Used by the "center on average"
  /// canvas control and as the initial framing when a case first loads, so
  /// the graph never opens parked on empty space away from the nodes.
  centerAverage: () => void;
  zoomBy: (factor: number) => void;
  resetZoom: () => void;
  /// Re-layout the graph with the named algorithm and persist the resulting
  /// node positions back to the DB via `onPositionChange`. `preset` is a
  /// no-op (positions are already authoritative).
  runLayout: (kind: LayoutKind) => void;
  /// Focus the named entity: select it, center it, and pulse-zoom in. Used
  /// by the find-on-graph overlay.
  focusEntity: (entityId: string) => void;
  /// Render the current graph view to a PNG and return it as a
  /// `data:image/png;base64,...` URL. Caller decides what to do with it
  /// (typically: blob-download). Returns `null` when the graph is empty.
  exportPng: () => string | null;
}

/// ─── Node fill helpers ───────────────────────────────────────────
/// Flat disc nodes read as "low quality"; a soft radial gradient (bright
/// centre → base colour → darker rim) gives them a subtle glossy-orb depth
/// without any extra render cost (Cytoscape rasterises the gradient on the
/// GPU/canvas the same as a solid fill). These helpers derive the rim/centre
/// shades from the node's own colour so every entity type keeps its hue.
function parseHex(hex: string): { r: number; g: number; b: number } | null {
  const m = /^#?([0-9a-f]{3}|[0-9a-f]{6})$/i.exec(hex.trim());
  if (!m) return null;
  let h = m[1];
  if (h.length === 3) h = h.split("").map((c) => c + c).join("");
  const n = parseInt(h, 16);
  return { r: (n >> 16) & 255, g: (n >> 8) & 255, b: n & 255 };
}
function toHex(r: number, g: number, b: number): string {
  const c = (v: number) =>
    Math.max(0, Math.min(255, Math.round(v))).toString(16).padStart(2, "0");
  return `#${c(r)}${c(g)}${c(b)}`;
}
/// Mix `hex` toward white (amt>0) or black (amt<0) by |amt| (0..1).
function shade(hex: string, amt: number): string {
  const c = parseHex(hex);
  if (!c) return hex;
  const t = amt >= 0 ? 255 : 0;
  const a = Math.abs(amt);
  return toHex(c.r + (t - c.r) * a, c.g + (t - c.g) * a, c.b + (t - c.b) * a);
}
/// The CSS for a glossy-orb node fill in `color`. Falls back to a flat fill
/// if `color` isn't a parseable hex (e.g. a future hsl()/named colour).
function orbFill(color: string): Record<string, string> {
  if (!parseHex(color)) return { "background-color": color };
  return {
    "background-fill": "radial-gradient",
    "background-gradient-stop-colors": `${shade(color, 0.34)} ${color} ${shade(color, -0.2)}`,
    "background-gradient-stop-positions": "0% 52% 100%",
  };
}

function buildStylesheet(types: CanonicalType[]): StylesheetCSS[] {
  const base: StylesheetCSS[] = [
    {
      selector: "node",
      css: {
        // Glossy-orb fill (overridden per-type below). The grey default is
        // only hit by typeless nodes.
        ...orbFill("#536179"),
        label: "data(label)",
        // Brighter label + a fully-opaque dark outline AND a soft dark pill
        // behind the text → readable against both the dark canvas and any
        // bright node colour. This is the "restore the labels" fix: they're
        // now crisp instead of faint grey.
        color: "#f3f7fd",
        "text-outline-color": "#04070e",
        "text-outline-width": 2.5,
        "text-outline-opacity": 1,
        "text-background-color": "#04070e",
        "text-background-opacity": 0.45,
        "text-background-shape": "roundrectangle",
        "text-background-padding": "2px",
        "font-size": 11,
        "font-weight": 600,
        "text-valign": "bottom",
        "text-halign": "center",
        "text-margin-y": 7,
        "text-wrap": "ellipsis",
        "text-max-width": "140px",
        // Don't render node labels below 7px screen pixels. Big-graph
        // perf win + UX win (labels turn into pixel noise anyway).
        "min-zoomed-font-size": 7,
        width: 36,
        height: 36,
        // A hair-line rim a touch lighter than the canvas separates the orb
        // cleanly from the background without the heavy dark ring it had.
        "border-width": 1.5,
        "border-color": "#cdd9ec",
        "border-opacity": 0.18,
        "overlay-padding": 6,
        "transition-property":
          "border-color, border-width, width, height, opacity",
        // Snappier select/resize feedback — 240ms read as laggy.
        "transition-duration": 120,
        "transition-timing-function": "ease-out",
      },
    },
    {
      selector: "node:active",
      css: { "overlay-opacity": 0.15, "overlay-color": "#5eead4" },
    },
    // Live mouse-hover "focus" feedback. Pointing at a node grows it, rings
    // it in a bright teal glow, lights up its incident edges + adjacent
    // nodes, and fades everything else right back (the `.faded` rule below).
    // The whole graph reacts to the cursor in real time, which is what reads
    // as "smooth/polished". Everything is a class toggle applied in one
    // batch with a fast transition, so it stays cheap.
    {
      selector: "node.hovered",
      css: {
        width: 46,
        height: 46,
        "border-width": 3,
        "border-color": "#5eead4",
        "border-opacity": 1,
        "overlay-opacity": 0.18,
        "overlay-color": "#5eead4",
        "overlay-padding": 10,
        "z-index": 30,
        "transition-property":
          "border-color, border-width, border-opacity, width, height, overlay-opacity",
        "transition-duration": 70,
      },
    },
    {
      selector: "node.neighbor-hl",
      css: {
        "border-color": "#5eead4",
        "border-opacity": 0.9,
        "border-width": 3,
        "z-index": 25,
      },
    },
    // Everything outside the hovered node's neighbourhood fades back so the
    // focus pops. Quick fade so it tracks the cursor.
    {
      selector: ".faded",
      css: {
        opacity: 0.16,
        "transition-property": "opacity",
        "transition-duration": 70,
      },
    },
    {
      selector: "node:selected",
      css: {
        "border-width": 3,
        "border-color": "#5eead4",
        "border-opacity": 1,
        width: 42,
        height: 42,
      },
    },
    // Free-floating sticky-note annotations. Rendered as a rounded,
    // amber rectangle with the note text as the label. Wider than
    // regular nodes so even a sentence-long annotation reads cleanly.
    {
      selector: "node.annotation",
      css: {
        shape: "round-rectangle",
        width: 180,
        height: 60,
        "background-color": "data(color)",
        "background-opacity": 0.85,
        "border-color": "rgba(0,0,0,0.25)",
        "border-width": 1,
        "border-style": "solid",
        color: "#0b1120",
        "font-weight": 500,
        "font-size": 11,
        "text-valign": "center",
        "text-halign": "center",
        "text-wrap": "wrap",
        "text-max-width": "160px",
        "text-outline-width": 0,
        "text-margin-y": 0,
      },
    },
    {
      selector: "node.annotation:selected",
      css: {
        "border-width": 2,
        "border-color": "#5eead4",
        width: 196,
        height: 68,
      },
    },
    // Bookmarked nodes get a gold ring + slightly bigger size so they
    // stand out at a glance — investigators star the entities they want
    // to come back to.
    {
      selector: "node.bookmarked",
      css: {
        "border-width": 3,
        "border-color": "#fbbf24",
        "border-opacity": 1,
      },
    },
    {
      selector: "node.bookmarked:selected",
      css: {
        "border-width": 4,
        "border-color": "#fde047",
      },
    },
    // Collection nodes are drawn as a stacked-square shape so the user can
    // tell at a glance they're a *bag* of N entities, not one entity.
    {
      selector: "node.collection",
      css: {
        shape: "round-rectangle",
        width: 60,
        height: 40,
        "background-color": "#1a2440",
        "border-color": "#5eead4",
        "border-width": 2,
        "border-style": "dashed",
        "font-weight": 600,
        "font-size": 12,
      },
    },
    {
      selector: "node.collection:selected",
      css: {
        "border-width": 3,
        "border-style": "dashed",
        width: 68,
        height: 46,
      },
    },
    {
      selector: "node.fresh",
      css: { opacity: 0, width: 20, height: 20 },
    },
    {
      selector: "edge",
      css: {
        "curve-style": "bezier",
        "target-arrow-shape": "triangle",
        "target-arrow-fill": "filled",
        "arrow-scale": 0.9,
        "line-color": "#334155",
        "target-arrow-color": "#475569",
        "line-opacity": 0.9,
        width: 1.4,
        label: "data(label)",
        "font-size": 9,
        // Performance hint: don't paint edge labels when they'd be
        // rendered at < 8px on-screen. At low zoom the labels were a
        // dominant per-frame cost on big graphs (>1000 edges).
        "min-zoomed-font-size": 8,
        "font-family":
          "ui-monospace, 'JetBrains Mono', SFMono-Regular, Menlo, monospace",
        color: "#7a8aa3",
        "text-rotation": "autorotate",
        "text-background-color": "#0b1120",
        "text-background-opacity": 0.85,
        "text-background-padding": "3px",
        "text-background-shape": "roundrectangle",
      },
    },
    {
      selector: "edge:selected",
      css: {
        "line-color": "#5eead4",
        "target-arrow-color": "#5eead4",
        width: 2,
      },
    },
    // Per-transform edge colour — each edge carries a `tcolor` derived
    // from a stable hash of its transform_id, so every individual pivot
    // gets its own visually-distinct hue. Edges without a tcolor (the
    // empty-string default) fall through to the neutral base colour
    // defined on `edge` above.
    {
      selector: "edge[tcolor != '']",
      css: {
        "line-color": "data(tcolor)",
        "target-arrow-color": "data(tcolor)",
      },
    },
    // Hovering a node lights up its incident edges. Placed last so it
    // overrides both the neutral base edge colour and the per-transform
    // `tcolor`, and brought to front via `z-index` so the highlighted
    // path reads clearly over the surrounding mesh.
    {
      selector: "edge.hl-edge",
      css: {
        "line-color": "#5eead4",
        "target-arrow-color": "#5eead4",
        "line-opacity": 1,
        width: 2.4,
        "z-index": 20,
        "transition-property": "line-color, width, line-opacity",
        "transition-duration": 70,
      },
    },
  ];
  const perType = types.flatMap<StylesheetCSS>((t) => {
    if (t.id === "Collection") return []; // handled by .collection class
    const rules: StylesheetCSS[] = [
      {
        selector: `node[canonical = '${t.id}']`,
        css: orbFill(t.color),
      },
    ];
    if (t.kind === "threat") {
      rules.push({
        selector: `node[canonical = '${t.id}']`,
        css: { "border-width": 3, "border-color": "#f87171" },
      });
    }
    return rules;
  });
  return [...base, ...perType];
}

/// Tuned per-layout options. Defaults from the Cytoscape ecosystem with
/// small tweaks: rank-direction for dagre, animation on for fcose.
function buildLayoutOptions(kind: LayoutKind): cytoscape.LayoutOptions {
  // Cytoscape's type for layout options is the discriminated union; the
  // dagre/fcose extension options are not in `LayoutOptions` but cytoscape
  // accepts the loose shape at runtime. We cast through `unknown` once.
  const padding = 40;
  switch (kind) {
    case "fcose":
      return {
        name: "fcose",
        animate: true,
        animationDuration: 400,
        fit: true,
        padding,
        // Untangle: a touch of repulsion + decent node-spacing keeps
        // visually-separable clusters apart.
        nodeRepulsion: 5500,
        idealEdgeLength: 110,
        // Quality "default" trades a few ms for noticeably better layouts.
        quality: "default",
        randomize: false,
      } as unknown as cytoscape.LayoutOptions;
    case "dagre":
      return {
        name: "dagre",
        rankDir: "TB",
        nodeSep: 50,
        rankSep: 80,
        animate: true,
        animationDuration: 380,
        padding,
        fit: true,
      } as unknown as cytoscape.LayoutOptions;
    case "concentric":
      return {
        name: "concentric",
        animate: true,
        animationDuration: 360,
        padding,
        minNodeSpacing: 30,
        concentric: (n) => (n.degree(false) ?? 0) + 1,
        levelWidth: () => 1,
      };
    case "circle":
      return {
        name: "circle",
        animate: true,
        animationDuration: 360,
        padding,
        radius: undefined,
      };
    case "grid":
      return {
        name: "grid",
        animate: true,
        animationDuration: 360,
        padding,
        avoidOverlap: true,
      };
    case "preset":
    default:
      return { name: "preset" };
  }
}

/// Cytoscape uses this prefix to distinguish annotation nodes from real
/// entities at the data layer. The frontend strips it before talking to
/// the backend API.
export const ANN_PREFIX = "ann:";

function toElements(
  entities: Entity[],
  edges: Edge[],
  annotations: Annotation[] | undefined,
  transformSources: Record<string, string> | undefined,
  transformColors: Record<string, string> | undefined,
): ElementDefinition[] {
  const nodes: ElementDefinition[] = entities.map((e) => {
    const classes: string[] = [];
    if (e.entity_type === "Collection") classes.push("collection");
    if (e.bookmarked) classes.push("bookmarked");
    return {
      data: {
        id: e.id,
        label: e.label,
        canonical: e.entity_type,
        bookmarked: e.bookmarked ? 1 : 0,
      },
      classes: classes.length > 0 ? classes.join(" ") : undefined,
      position:
        e.pos_x != null && e.pos_y != null
          ? { x: e.pos_x, y: e.pos_y }
          : undefined,
    };
  });
  const edgeEls: ElementDefinition[] = edges.map((edge) => {
    const transformId = edge.label ?? "";
    const source = transformSources?.[transformId] ?? "";
    const tcolor = transformColors?.[transformId] ?? "";
    return {
      data: {
        id: edge.id,
        source: edge.source_id,
        target: edge.target_id,
        label: transformId,
        // `tsource` (transform source) is the *sidecar* id — currently
        // kept on the data but unused by the stylesheet. `tcolor` is the
        // per-transform hex/HSL colour the stylesheet reads to paint
        // each line.
        tsource: source,
        tcolor,
      },
    };
  });
  const annotationNodes: ElementDefinition[] = (annotations ?? []).map((a) => ({
    data: {
      id: `${ANN_PREFIX}${a.id}`,
      label: a.text || "(empty note)",
      color: a.color,
    },
    classes: "annotation",
    position: { x: a.pos_x, y: a.pos_y },
  }));
  return [...nodes, ...annotationNodes, ...edgeEls];
}

/// Pan the viewport so the *centroid* (mean position) of the currently
/// visible nodes sits dead-centre in the canvas, keeping the current zoom.
/// Distinct from `cy.fit`/`cy.center`, which use the bounding-box centre —
/// the centroid follows where the mass of the graph actually is, so a few
/// far-flung outliers don't drag the focus off the main cluster. Returns
/// `false` when there are no visible nodes to centre on.
function panToCentroid(cy: Core, animate: boolean): boolean {
  const nodes = cy.nodes().filter((n) => n.style("display") !== "none");
  if (nodes.length === 0) return false;
  let sx = 0;
  let sy = 0;
  nodes.forEach((n) => {
    const p = n.position();
    sx += p.x;
    sy += p.y;
  });
  const cx = sx / nodes.length;
  const cy_ = sy / nodes.length;
  const z = cy.zoom();
  const pan = { x: cy.width() / 2 - z * cx, y: cy.height() / 2 - z * cy_ };
  if (animate) cy.animate({ pan }, { duration: 220 });
  else cy.pan(pan);
  return true;
}

/// Initial framing when a case first loads: fit to a zoom that shows the
/// whole graph, then pull back so it opens zoomed-out with breathing room
/// rather than tight against the viewport edges, and finally anchor the pan
/// on the centroid. `INITIAL_ZOOM_OUT` < 1 is how far back we pull from the
/// fit-zoom (0.6 ≈ 40% wider view).
const INITIAL_ZOOM_OUT = 0.6;
function frameInitial(cy: Core) {
  if (cy.nodes().length === 0) return;
  cy.fit(undefined, 60);
  cy.zoom(Math.max(cy.minZoom(), cy.zoom() * INITIAL_ZOOM_OUT));
  panToCentroid(cy, false);
}

export const GraphCanvas = forwardRef<GraphHandle, Props>(function GraphCanvas(
  {
    entities,
    edges,
    annotations,
    selectedIds,
    canonicalTypes,
    transformSources,
    transformColors,
    hiddenTypes,
    bookmarkedOnly,
    performanceMode,
    webgl,
    onSelectionChange,
    onAnnotationSelectionChange,
    onEdgeSelectionChange,
    onPositionChange,
    onAnnotationMoved,
    onContextMenu,
    onContextMenuBackground,
  },
  ref,
) {
  // Latest callback refs so the once-only cytoscape effect doesn't re-mount
  // when the parent changes handlers (the parent typically inlines them).
  const onContextMenuRef = useRef(onContextMenu);
  const onContextMenuBackgroundRef = useRef(onContextMenuBackground);
  const onSelectionChangeRef = useRef(onSelectionChange);
  const onAnnotationSelectionChangeRef = useRef(onAnnotationSelectionChange);
  const onEdgeSelectionChangeRef = useRef(onEdgeSelectionChange);
  const onPositionChangeRef = useRef(onPositionChange);
  const onAnnotationMovedRef = useRef(onAnnotationMoved);
  useEffect(() => {
    onEdgeSelectionChangeRef.current = onEdgeSelectionChange;
  }, [onEdgeSelectionChange]);
  useEffect(() => {
    onContextMenuRef.current = onContextMenu;
  }, [onContextMenu]);
  useEffect(() => {
    onContextMenuBackgroundRef.current = onContextMenuBackground;
  }, [onContextMenuBackground]);
  useEffect(() => {
    onSelectionChangeRef.current = onSelectionChange;
  }, [onSelectionChange]);
  useEffect(() => {
    onPositionChangeRef.current = onPositionChange;
  }, [onPositionChange]);
  useEffect(() => {
    onAnnotationSelectionChangeRef.current = onAnnotationSelectionChange;
  }, [onAnnotationSelectionChange]);
  useEffect(() => {
    onAnnotationMovedRef.current = onAnnotationMoved;
  }, [onAnnotationMoved]);
  const container = useRef<HTMLDivElement | null>(null);
  const cyRef = useRef<Core | null>(null);
  /// Guard so syncing React → Cytoscape selection doesn't fire the
  /// `select/unselect` listener that would echo back into React.
  const syncingSelection = useRef(false);
  /// One-shot guard: the very first time nodes appear we frame the graph on
  /// its centroid. Entities load asynchronously *after* the canvas mounts,
  /// so the init effect almost always runs with an empty graph — the framing
  /// has to happen in the entity-sync effect instead, but only once.
  const didInitialCenter = useRef(false);
  const stylesheet = useMemo(
    () => buildStylesheet(canonicalTypes),
    [canonicalTypes],
  );

  useImperativeHandle(
    ref,
    () => ({
      fit: () => {
        const cy = cyRef.current;
        if (!cy) return;
        if (cy.elements().length === 0) return;
        cy.animate({ fit: { eles: cy.elements(), padding: 60 } }, { duration: 220 });
      },
      center: () => {
        const cy = cyRef.current;
        if (!cy) return;
        const sel = cy.$(":selected");
        if (sel.length > 0) {
          cy.animate({ center: { eles: sel } }, { duration: 220 });
        } else if (cy.elements().length > 0) {
          cy.animate({ center: { eles: cy.elements() } }, { duration: 220 });
        }
      },
      centerAverage: () => {
        const cy = cyRef.current;
        if (!cy) return;
        panToCentroid(cy, true);
      },
      zoomBy: (factor: number) => {
        const cy = cyRef.current;
        if (!cy) return;
        const ext = cy.extent();
        const cx = (ext.x1 + ext.x2) / 2;
        const cy_ = (ext.y1 + ext.y2) / 2;
        const next = Math.max(0.2, Math.min(4, cy.zoom() * factor));
        cy.animate(
          { zoom: { level: next, position: { x: cx, y: cy_ } } },
          { duration: 140 },
        );
      },
      resetZoom: () => {
        const cy = cyRef.current;
        if (!cy) return;
        cy.animate({ zoom: 1, center: { eles: cy.elements() } }, { duration: 220 });
      },
      runLayout: (kind: LayoutKind) => {
        const cy = cyRef.current;
        if (!cy || cy.nodes().length === 0) return;
        if (kind === "preset") return;
        const opts = buildLayoutOptions(kind);
        const layout = cy.layout(opts);
        // The layout's own `fit: true` already animates the viewport to
        // frame the result as the layout settles. After it ends, just
        // snapshot positions back to the DB — no second fit animation
        // (which used to produce a visible "settle, then jerk" jump).
        const onStop = () => {
          cy.nodes().forEach((n) => {
            const p = n.position();
            onPositionChangeRef.current(n.id(), p.x, p.y);
          });
        };
        layout.one("layoutstop", onStop);
        layout.run();
      },
      exportPng: () => {
        const cy = cyRef.current;
        if (!cy || cy.elements().length === 0) return null;
        // `full: true` renders the whole graph (not just the viewport),
        // `scale: 2` gives a 2× pixel-density image so it stays sharp on
        // hi-dpi screens and in print. Backgrounds default to transparent
        // — give it the app's canvas tone so a screenshot doesn't look
        // weird on a white doc.
        return cy.png({
          full: true,
          scale: 2,
          bg: "#070b14",
          output: "base64uri",
        });
      },
      focusEntity: (entityId: string) => {
        const cy = cyRef.current;
        if (!cy) return;
        const node = cy.getElementById(entityId);
        if (node.length === 0) return;
        cy.batch(() => {
          cy.elements().unselect();
          node.select();
        });
        cy.animate(
          {
            center: { eles: node },
            zoom: Math.max(cy.zoom(), 1.2),
          },
          { duration: 280 },
        );
      },
    }),
    [],
  );

  useEffect(() => {
    if (!container.current || cyRef.current) return;
    ensureLayoutsRegistered();
    // Auto-enable perf mode past 1000 entities; user-set prop wins.
    const perf =
      performanceMode !== undefined
        ? performanceMode
        : entities.length > 1000;
    const baseOptions: cytoscape.CytoscapeOptions = {
      container: container.current,
      style: stylesheet,
      elements: toElements(entities, edges, annotations, transformSources, transformColors),
      layout: { name: "preset" },
      // Scroll-wheel zoom speed. 0.5 felt sluggish (you had to spin the
      // wheel a lot to zoom); 1.0 is Cytoscape's well-tested default and
      // makes zoom feel immediate.
      wheelSensitivity: 1.0,
      minZoom: 0.2,
      maxZoom: 4,
      // Standard graph-editor selection model: plain click/box-drag
      // replaces the selection; shift+click / shift+box-drag adds.
      // Matches Maltego, Figma, yEd.
      boxSelectionEnabled: true,
      selectionType: "single",
      // Perf-mode wins: bake the canvas to a texture during pan/zoom
      // (huge win for big graphs at the cost of slightly fuzzy edges
      // during motion), and hide edges/labels during pan/zoom.
      // `motionBlur` stays off everywhere — it smears the canvas during
      // interaction on most GPUs (looks "low quality" rather than
      // smooth). `pixelRatio` stays at "auto" so hi-DPI screens render
      // sharply even in perf mode; the texture/edge-hide tricks are
      // plenty on their own.
      //
      // NOTE: `textureOnViewport`/`hideEdgesOnViewport`/`hideLabelsOnViewport`
      // are Canvas2D-renderer tricks; the WebGL renderer ignores them (it's
      // already GPU-fast), so they only matter on the Canvas2D fallback path.
      textureOnViewport: perf,
      motionBlur: false,
      pixelRatio: "auto",
      hideEdgesOnViewport: perf,
      hideLabelsOnViewport: perf,
    };
    // WebGL is the default. The renderer rasterises nodes/edges/labels from
    // GPU texture atlases, so pan/zoom stays at native frame rate on graphs
    // far larger than Canvas2D can drive smoothly. `bgColor` is left unset
    // so it reads the container's (transparent) CSS background and the app's
    // gradient/grid show through. If context creation throws — old GPU,
    // blocklisted driver, headless — we fall back to Canvas2D so the graph
    // always renders.
    const useWebgl = webgl !== false;
    let cy: Core;
    try {
      cy = useWebgl
        ? cytoscape({
            ...baseOptions,
            // Cytoscape's typings don't yet describe the experimental WebGL
            // renderer options, so we widen through `unknown` once.
            renderer: {
              name: "canvas",
              webgl: true,
              // Bigger atlases → more distinct node/label textures per draw
              // call → fewer batches → fewer GPU state changes per frame.
              webglTexSize: 4096,
              webglTexRows: 48,
              webglBatchSize: 4096,
              webglTexPerBatch: 16,
            },
          } as unknown as cytoscape.CytoscapeOptions)
        : cytoscape(baseOptions);
    } catch (err) {
      console.warn(
        "[maltrace] WebGL renderer unavailable — falling back to Canvas2D.",
        err,
      );
      cy = cytoscape(baseOptions);
    }
    // Tag the container with a perf-mode class so the stylesheet can
    // drop edge labels in perf mode (saves a lot of text-render
    // cycles on big graphs).
    if (perf) container.current.classList.add("perf-mode");
    cyRef.current = cy;

    const reportSelection = () => {
      if (syncingSelection.current) return;
      const selected = cy.nodes(":selected").map((n) => n.id() as string);
      // Split: cytoscape-layer ids prefixed with `ann:` are annotations;
      // everything else is an entity. Reporters get the un-prefixed ids.
      const entityIds = selected
        .filter((id) => !id.startsWith(ANN_PREFIX))
        .sort();
      const annotationIds = selected
        .filter((id) => id.startsWith(ANN_PREFIX))
        .map((id) => id.slice(ANN_PREFIX.length))
        .sort();
      onSelectionChangeRef.current(entityIds);
      onAnnotationSelectionChangeRef.current?.(annotationIds);
    };

    cy.on("select unselect", "node", reportSelection);
    cy.on("tap", "edge", (evt) => {
      // Edge selection is single-edge-at-a-time. Tapping a fresh edge
      // clears any node selection so the edge inspector takes over.
      cy.batch(() => {
        cy.nodes().unselect();
        cy.edges().unselect();
        evt.target.select();
      });
      reportSelection();
      onEdgeSelectionChangeRef.current?.(evt.target.id() as string);
    });
    cy.on("tap", "node", () => {
      // Tapping a node clears the edge inspector so the two views don't
      // fight each other.
      onEdgeSelectionChangeRef.current?.(null);
    });
    cy.on("tap", (evt) => {
      if (evt.target === cy) {
        // Background click clears selection. Cytoscape handles the actual
        // unselect; this fires the change reporter.
        reportSelection();
        onEdgeSelectionChangeRef.current?.(null);
      }
    });
    cy.on("dragfree", "node", (evt) => {
      const n = evt.target;
      const p = n.position();
      const id = n.id() as string;
      if (id.startsWith(ANN_PREFIX)) {
        onAnnotationMovedRef.current?.(id.slice(ANN_PREFIX.length), p.x, p.y);
      } else {
        onPositionChangeRef.current(id, p.x, p.y);
      }
    });

    // Right-click on a node → bubble up to the parent so it can open the
    // floating transform menu. `cxttap` fires for both mouse right-click and
    // touch long-press.
    cy.on("cxttap", "node", (evt) => {
      const id = evt.target.id() as string;
      // Right-clicking an annotation doesn't make sense for the transform
      // menu — fall through to the background handler so the parent can
      // route to an annotation-specific context menu (or just dismiss).
      if (id.startsWith(ANN_PREFIX)) {
        onContextMenuBackgroundRef.current?.();
        return;
      }
      const oe = evt.originalEvent as MouseEvent | TouchEvent | undefined;
      let clientX = 0;
      let clientY = 0;
      if (oe instanceof MouseEvent) {
        clientX = oe.clientX;
        clientY = oe.clientY;
      } else if (oe && "touches" in oe && oe.touches.length > 0) {
        clientX = oe.touches[0].clientX;
        clientY = oe.touches[0].clientY;
      }
      onContextMenuRef.current?.(id, clientX, clientY);
    });
    cy.on("cxttap", (evt) => {
      if (evt.target === cy) onContextMenuBackgroundRef.current?.();
    });

    // ─── Live mouse interaction (hover highlight + cursor) ───────
    // Pointer feedback that tracks the cursor in real time: hovering a
    // node highlights it plus its incident edges and adjacent nodes, and
    // swaps the canvas cursor to a pointer. Leaving clears the highlight.
    // Only the hovered node's immediate neighbourhood is touched, so this
    // is cheap regardless of graph size. Skipped in perf mode, where the
    // priority is raw pan/zoom frame rate on huge graphs.
    const clearHover = () => {
      cy.batch(() => {
        cy.elements(".hovered").removeClass("hovered");
        cy.elements(".neighbor-hl").removeClass("neighbor-hl");
        cy.elements(".hl-edge").removeClass("hl-edge");
        cy.elements(".faded").removeClass("faded");
      });
    };
    if (!perf) {
      cy.on("mouseover", "node", (evt) => {
        const node = evt.target;
        if ((node.id() as string).startsWith(ANN_PREFIX)) return;
        container.current?.classList.add("cursor-node");
        // `closedNeighborhood` = the node itself + its incident edges +
        // adjacent nodes. Everything *outside* it fades back so the focus
        // pops; inside it gets the highlight treatment.
        const hood = node.closedNeighborhood();
        cy.batch(() => {
          cy.elements().not(hood).addClass("faded");
          node.addClass("hovered");
          hood.edges().addClass("hl-edge");
          hood.nodes().not(node).addClass("neighbor-hl");
        });
      });
      cy.on("mouseout", "node", () => {
        container.current?.classList.remove("cursor-node");
        clearHover();
      });
      // Hovering an *edge* focuses the relationship: highlight the edge and
      // its two endpoints, fade everything else. Same focus language as the
      // node hover, so the whole canvas feels alive under the cursor.
      cy.on("mouseover", "edge", (evt) => {
        const edge = evt.target;
        container.current?.classList.add("cursor-node");
        const ends = edge.connectedNodes();
        const hood = ends.union(edge);
        cy.batch(() => {
          cy.elements().not(hood).addClass("faded");
          edge.addClass("hl-edge");
          ends.addClass("neighbor-hl");
        });
      });
      cy.on("mouseout", "edge", () => {
        container.current?.classList.remove("cursor-node");
        clearHover();
      });
      // A node leaving via removal/drag shouldn't leave a stale highlight.
      cy.on("grab", "node", clearHover);
    }

    // Rare path: entities already present at mount. Fit to pick a sensible
    // zoom that frames the whole graph, then re-pan onto the centroid so the
    // opening view is anchored on where the nodes actually are. The common
    // path (entities arriving async) is handled in the entity-sync effect.
    if (entities.length > 0) {
      frameInitial(cy);
      didInitialCenter.current = true;
    }

    // ─── Zoom-based label culling (perf) ─────────────────────
    // At low zoom levels, edge + node labels are illegible noise and
    // their text-rendering dominates the frame budget on big graphs.
    // We toggle a class on the container that the stylesheet uses to
    // hide them. Debounced so a pinch-zoom doesn't thrash style.
    const cyContainer = cy.container();
    let zoomRaf = 0;
    const applyZoomClass = () => {
      const z = cy.zoom();
      if (!cyContainer) return;
      cyContainer.classList.toggle("zoom-tiny", z < 0.35);
      cyContainer.classList.toggle("zoom-small", z >= 0.35 && z < 0.65);
    };
    applyZoomClass();
    cy.on("zoom", () => {
      if (zoomRaf) return;
      zoomRaf = requestAnimationFrame(() => {
        zoomRaf = 0;
        applyZoomClass();
      });
    });

    return () => {
      if (zoomRaf) cancelAnimationFrame(zoomRaf);
      cy.destroy();
      cyRef.current = null;
    };
    // Re-create the instance when `webgl` flips — Cytoscape can't hot-swap
    // its renderer, so switching Canvas2D↔WebGL tears down and rebuilds.
    // The other inputs are intentionally excluded (they're synced by the
    // dedicated effects below); see the once-only mount guard above.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [webgl]);

  useEffect(() => {
    const cy = cyRef.current;
    if (!cy) return;
    cy.style(stylesheet);
  }, [stylesheet]);

  // Sync entities/edges. New nodes get a one-shot fade/scale animation via
  // the `.fresh` class.
  useEffect(() => {
    const cy = cyRef.current;
    if (!cy) return;
    const newEls = toElements(entities, edges, annotations, transformSources, transformColors);
    const newIds = new Set(newEls.map((e) => e.data.id as string));
    cy.elements().forEach((el) => {
      if (!newIds.has(el.id())) el.remove();
    });

    // Build a lookup by entity id so we can apply the lock state to
    // freshly-added nodes during this pass (we don't have access to the
    // `entities` array inside the per-element add path otherwise).
    const lockedByEntityId = new Set(
      entities.filter((e) => e.locked).map((e) => e.id),
    );
    const justAdded: cytoscape.NodeSingular[] = [];
    for (const el of newEls) {
      const existing = cy.getElementById(el.data.id as string);
      if (existing.length === 0) {
        const added = cy.add(el);
        if (added.isNode()) {
          added.addClass("fresh");
          const node = added as cytoscape.NodeSingular;
          // Cytoscape's lock semantics: locked nodes are immune to
          // both user drag and layout repositioning. Apply on insert
          // so newly-loaded locked entities behave correctly without
          // waiting for the sync pass below.
          if (lockedByEntityId.has(el.data.id as string)) node.lock();
          justAdded.push(node);
        }
      } else {
        existing.data("label", el.data.label);
        existing.data("canonical", el.data.canonical);
        if (el.data.color !== undefined) {
          existing.data("color", el.data.color);
        }
        if (existing.isEdge()) {
          if (el.data.tsource !== undefined) {
            existing.data("tsource", el.data.tsource);
          }
          if (el.data.tcolor !== undefined) {
            existing.data("tcolor", el.data.tcolor);
          }
        }
        if (existing.isNode()) {
          const isCollection = el.data.canonical === "Collection";
          if (isCollection) existing.addClass("collection");
          else existing.removeClass("collection");
          const bookmarked = el.data.bookmarked === 1;
          existing.data("bookmarked", el.data.bookmarked);
          if (bookmarked) existing.addClass("bookmarked");
          else existing.removeClass("bookmarked");
        }
        if (el.position) existing.position(el.position);
      }
    }
    if (justAdded.length > 0) {
      requestAnimationFrame(() => {
        for (const n of justAdded) n.removeClass("fresh");
      });
    }

    // First time the graph is non-empty (typically the async case load),
    // frame it: fit to a zoom that shows everything, then anchor the pan on
    // the centroid so the case never opens parked on empty canvas.
    if (!didInitialCenter.current && cy.nodes().length > 0) {
      didInitialCenter.current = true;
      frameInitial(cy);
    }
  }, [entities, edges]);

  // Mirror the React selection set into Cytoscape's selection state. The
  // `syncingSelection` flag prevents the resulting select/unselect events
  // from echoing back into React.
  useEffect(() => {
    const cy = cyRef.current;
    if (!cy) return;
    const desired = new Set(selectedIds);
    syncingSelection.current = true;
    cy.batch(() => {
      cy.nodes().forEach((n) => {
        const want = desired.has(n.id());
        const has = n.selected();
        if (want && !has) n.select();
        else if (!want && has) n.unselect();
      });
    });
    syncingSelection.current = false;
  }, [selectedIds]);

  // Mirror the per-entity `locked` flag into Cytoscape's lock state.
  // Re-runs whenever entities change so toggling lock from the inspector
  // takes effect immediately.
  useEffect(() => {
    const cy = cyRef.current;
    if (!cy) return;
    cy.batch(() => {
      for (const e of entities) {
        const n = cy.getElementById(e.id);
        if (n.length === 0) continue;
        if (e.locked && !n.locked()) n.lock();
        else if (!e.locked && n.locked()) n.unlock();
      }
    });
  }, [entities]);

  // Mirror the per-type + bookmarked-only visibility filters. Hiding a
  // node with display:none also hides its incident edges (Cytoscape does
  // this automatically when the endpoint is `display: none`). The state
  // is purely visual — entities stay in the DB and re-appear when the
  // user toggles the filter back off.
  useEffect(() => {
    const cy = cyRef.current;
    if (!cy) return;
    const hidden = new Set(hiddenTypes ?? []);
    cy.batch(() => {
      cy.nodes().forEach((n) => {
        const canonical = n.data("canonical") as string | undefined;
        const typeHidden = canonical ? hidden.has(canonical) : false;
        const bookmarkHidden = bookmarkedOnly
          ? n.data("bookmarked") !== 1
          : false;
        const shouldHide = typeHidden || bookmarkHidden;
        n.style("display", shouldHide ? "none" : "element");
      });
    });
  }, [hiddenTypes, bookmarkedOnly, entities]);

  return (
    <div
      ref={container}
      className="graph-canvas"
      // Block the browser's default context menu — our custom floating menu
      // takes over.
      onContextMenu={(e) => e.preventDefault()}
      style={{
        width: "100%",
        height: "100%",
        background: "transparent",
        position: "relative",
        zIndex: 2,
      }}
    />
  );
});
