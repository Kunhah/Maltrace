# maltrace — User Guide

A complete reference for every feature, shortcut, and panel in maltrace.
If you're new, read this top-to-bottom. If you're already running it, skim
the table of contents and jump to whatever you need.

For a guided OSINT investigation walk-through, see
[INVESTIGATION_GUIDE.md](INVESTIGATION_GUIDE.md). For writing your own
transform plugin, see [PLUGIN_SDK.md](PLUGIN_SDK.md). For setting up a dev
environment, see [DEVELOPMENT.md](DEVELOPMENT.md).

---

## Contents

1. [What maltrace is](#1-what-maltrace-is)
2. [Launching](#2-launching)
3. [The interface](#3-the-interface)
4. [Working with entities](#4-working-with-entities)
5. [Running transforms](#5-running-transforms)
6. [Machines (Maltego-style pipelines)](#6-machines-maltego-style-pipelines)
7. [The graph canvas](#7-the-graph-canvas)
8. [Collections](#8-collections)
9. [Cases](#9-cases)
10. [The AI analyst panel](#10-the-ai-analyst-panel)
11. [Secrets (API keys)](#11-secrets-api-keys)
12. [Plugins](#12-plugins)
13. [Transform run history](#13-transform-run-history)
14. [Exporting](#14-exporting)
15. [Keyboard shortcuts](#15-keyboard-shortcuts)
16. [Environment variables](#16-environment-variables)
17. [Where data lives](#17-where-data-lives)
18. [Troubleshooting](#18-troubleshooting)

---

## 1. What maltrace is

maltrace is a desktop app for visual OSINT investigations. You drop an
**entity** on the canvas (a domain, email, IP, image, person…), run a
**transform** that returns related entities, and pivot through the
resulting graph. It's an open-source alternative to Maltego CE.

What makes it useful:

- **230+ transforms out of the box** via the bundled SpiderFoot adapter
  (WHOIS, DNS, breach lookups, ASN intel, dark-web mentions, social-media
  enumeration, and more).
- **Face search** via InsightFace + four reverse-image backends
  (Yandex / Bing / Google Lens / FaceCheck.ID).
- **Brazilian OSINT resource recommender** that surfaces Receita
  Federal, TSE, JusBrasil, Registro.br, Anatel, etc. when an entity
  matches BR patterns (CPF, CNPJ, `.br`).
- **Maltego-style machines** that chain multiple transforms into one
  click.
- **Built-in Claude analyst** that has read access to your graph and the
  Claude-OSINT investigative-tradecraft skills loaded.
- **Plugin SDK** so anyone can ship a new transform set as a single
  directory — no Rust edits.

Architecture: Tauri (Rust + React + Cytoscape.js) talking to Python
sidecars over a JSON subprocess protocol; SQLite for persistence.

---

## 2. Launching

### Desktop (Tauri)

```bash
cd maltrace/app
ANTHROPIC_API_KEY=sk-ant-…  npm run tauri dev      # optional — only needed for the AI panel
```

The `tauri` script bakes in `WEBKIT_DISABLE_DMABUF_RENDERER=1`, which is
required on Linux/Wayland with WebKitGTK 2.42+.

### Headless server (web build)

```bash
cd maltrace/app
npm run build              # produces ../dist
npm run server             # axum on http://127.0.0.1:1421
```

The React frontend auto-detects which transport to use (Tauri IPC vs HTTP
`/api/*`), so the same UI runs in both targets.

---

## 3. The interface

```
┌──────────────────────────────────────────────────────────────────────┐
│ ◆ maltrace OSINT [Desktop]  │ [case tabs] │ [add entity] │  status  │ ← topbar
├──────────────────────────────────────────────────────────────────────┤
│                                                                       │
│                                                          ┌──────────┐│
│                  Graph canvas                            │ Inspector ││
│                  (Cytoscape)                             │   pane    ││
│                                                          │           ││
│                                                          │           ││
│                            ┌──────────┐                  │           ││
│                            │ controls │                  │           ││
│                            └──────────┘                  │           ││
└──────────────────────────────────────────────────────────────────────┘
```

### Topbar (left → right)

| Element | What it does |
|---|---|
| Brand mark + "maltrace" + `OSINT` chip | Decorative |
| `Desktop` / `Web` chip | Shows the runtime — useful when running the headless server in a browser |
| **Case tabs** | One tab per case. Click to switch. `+` adds a new case. Double-click the active tab to rename. Right-click for delete / export. |
| **Add entity form** | Type-grouped dropdown (Identity / Asset / Evidence / Threat / Other) + label field + Add button. |
| **Analyst** button | Toggle the AI panel |
| **Machines** button | Open the machines panel (disabled when no machines are loaded) |
| Status text | Spinner when busy, the active task, transform count, entity / selection count |
| Clickable `N transforms` | Opens the Plugins modal |
| **?** button | Open the keyboard-shortcuts panel |

### Right pane (Inspector)

Empty until you select something. When one entity is selected, shows
type badge, label, original SpiderFoot event type (if known), payload,
position, and the filtered transform palette. When multiple are
selected, shows the multi-select view with intersection-compatible
transforms only.

### Canvas controls

Floating cluster bottom-right of the canvas:

- **Layout** — dropdown: Organic (fcose) / Hierarchical (dagre) /
  Concentric / Circle / Grid. Re-runs the layout and persists the new
  positions.
- **+** / **−** — zoom.
- **1:1** — reset zoom.
- **Fit** — fit all nodes in view.
- **Center** — centre on selection (or all nodes if nothing selected).

---

## 4. Working with entities

### Adding manually

Pick a type from the dropdown, type the value, click **Add**. The new
node lands in the centre with a small random offset. Switching the type
dropdown to **custom** lets you type any SpiderFoot event name verbatim
(e.g. `AFFILIATE_INTERNET_NAME`) — that's a power-user escape hatch you
rarely need.

### Selecting

- **Click a node** — single-select.
- **Ctrl/Cmd + click** — toggle a node in/out of the selection.
- **Drag across empty canvas** — box-select multiple nodes.
- **Esc** — clear selection.
- **Click empty canvas** — clear selection.

Selection drives the inspector and the right-click context menu.

### Dragging

Drag a node to reposition it. The position is persisted to SQLite the
moment you drop. Restart the app and it stays where you left it.

### Deleting

`Del` / `Backspace` deletes the selection (entity ± edges). Or click
the trash icon in the inspector header. Cascade is server-side: deleting
an entity also deletes its edges.

---

## 5. Running transforms

A **transform** takes one entity and returns zero or more new
entities + edges. It's what makes the graph grow.

### From the inspector

1. Select an entity.
2. The inspector's lower half shows compatible transforms, grouped by
   plugin (SpiderFoot / Face search / Brazuca / your custom plugins).
3. Type in the filter box (or press **`/`** to focus it) to narrow.
4. Click a transform to run it. The result merges into the graph and
   the inspector status updates.

### Right-click (context menu)

Right-click any node → floating menu near the cursor with the same
compatible-transforms list. Arrow keys / Enter to run, Esc / click
outside to close, type to filter (auto-focused).

If you right-click while a multi-selection is active, the menu shows
only transforms that accept **every** selected entity — useful for
"run sfp_whois on these 12 domains".

### Multi-select / batch

Same as single, but each transform runs once per entity sequentially.
The merged result becomes part of the graph as it streams in. A 12-way
batch shows progress in the toast.

### Per-transform options

Some transforms accept `--opts` JSON (e.g. `face_search_yandex` honors
`{"match_threshold": 0.6}`). The UI doesn't yet have per-transform
option editors, but plugins can document defaults — see their READMEs.

---

## 6. Machines (Maltego-style pipelines)

A **machine** is a JSON-defined chain of transforms that runs on one
input entity. Machines live in [`maltrace/machines/`](../machines/) and
are loaded at startup.

### Built-in machines

| Machine | Input | What it does |
|---|---|---|
| **Footprint L1** | Domain | WHOIS + DNS resolve + subdomain enumeration. Free, key-free, fast. |
| **IP Profile** | IpAddress | RIPE / IPInfo / BGPView lookups — ownership, geolocation, ASN, reverse-DNS hosts. |
| **Email Breach Check** | EmailAddress | HaveIBeenPwned + Gravatar + cross-platform account enumeration. |

### Running a machine

1. Select the input entity.
2. Click **Machines** in the topbar (or right-click → Machines).
3. Pick one; click **Run**.
4. The panel shows step-by-step results: which transforms ran, how many
   entities each produced, any per-step failures. Per-step failures are
   non-fatal — the machine keeps going.

### Frontier semantics

Each step picks a *frontier* of entities to act on:

- `input` — the original entity (chain multiple transforms on the seed)
- `new` — entities produced by the previous step (depth-first explore)
- `all` — every entity touched so far (broadest fan-out)

The runner only invokes a transform on entities whose source event
type the transform actually consumes — so a step targeting "all" won't
try to run `sfp_dnsresolve` on a WHOIS record, for example.

### Writing your own

See [machines/footprint_l1.json](../machines/footprint_l1.json) for the
schema. Drop a new `.json` file in `maltrace/machines/`, restart, done.

---

## 7. The graph canvas

### Layout engines

Use the layout dropdown in the floating canvas controls. After a layout
runs, every position is persisted, so the graph keeps that shape next
session.

| Layout | Best for |
|---|---|
| **Organic (fcose)** | General-purpose force-directed; the default |
| **Hierarchical (dagre)** | Investigations with strong source → derived chains (one root expanding outward) |
| **Concentric** | Showing relative importance by ring (selected at center) |
| **Circle** | Small graphs where you want every node visible at once |
| **Grid** | Mechanical alignment for screenshots / docs |

### Find on graph (Ctrl/Cmd+F)

Type any substring of a label or entity type. Results are ranked
(label hits beat type hits). **Enter** focuses the picked entity —
the canvas pans/zooms to centre it and selects it.

### Visual encoding

- **Node colour** — canonical entity type (Domain green, IP amber,
  Email purple, Person pink, etc.). The full palette lives in
  [`ontology.rs`](../app/src-tauri/src/ontology.rs).
- **Node shape** — circular by default; dashed rounded-rectangle for
  Collections.
- **Threat halo** — vulnerabilities and threat indicators get a red
  border so they pop visually even when unselected.
- **Edge colour** — by which plugin produced the relationship:
  SpiderFoot (slate), Face-search (pink), Brazuca (green).
- **Selection ring** — selected nodes get a teal border and grow
  slightly.

### Mouse and keys

| Action | Effect |
|---|---|
| Left-click node | Select |
| Ctrl/Cmd + click | Toggle in selection |
| Drag on empty space | Box-select |
| Drag node | Reposition (persisted) |
| Right-click node | Open transform context menu |
| Right-click empty space | Close any open menu |
| Scroll | Zoom |
| Click empty space | Clear selection |

---

## 8. Collections

Some transforms produce **a lot** of children (e.g. a domain that
resolves to 60 IPs, or a netblock with 500 hosts). To keep the graph
readable, maltrace **auto-collapses** them.

If a single transform run produces more than `COLLECTION_THRESHOLD`
(default: **10**) entities of one canonical type, those children
become a single **Collection node** instead.

- A Collection node shows as a dashed rounded rectangle with the count
  and type (e.g. *"14 IpAddress"*).
- Selecting it shows a preview list in the inspector.
- Click **Expand** to materialise the children — the collection node
  disappears and N individual nodes replace it.

A run that produces 14 IPs + 1 WhoisRecord becomes one IP Collection +
the inline WhoisRecord — grouping is per-canonical-type.

---

## 9. Cases

A **case** is an isolated investigation — its own entities, edges, and
history. Switch between them via the case tabs in the topbar.

### What you can do

| Action | How |
|---|---|
| **Create** | Click `+` on the case tab strip. Auto-named *"Investigation N"*. |
| **Switch** | Click any tab. The canvas + inspector reload for that case. |
| **Rename** | Double-click the active tab (or right-click → Rename). |
| **Delete** | Right-click the tab → Delete. Confirmation prompt. **Cascades to entities, edges, and transform-run history.** |
| **Export GraphML** | Right-click → Export. Downloads `{name}.graphml` — importable by Maltego, Gephi, yEd, Cytoscape Desktop, NetworkX. |
| **Export CSV** | Backend command `export_case_entities_csv` (UI affordance forthcoming). |

### What's stored per case

- Every entity + its position + payload (`entity.data` JSON, including
  the `extra` field a sidecar may have populated).
- Every edge + the label (which transform produced it).
- Every transform run — see [§13 Transform run history](#13-transform-run-history).

---

## 10. The AI analyst panel

Open with the **Analyst** button in the topbar. A floating panel
(bottom-right) chat with Claude.

### How it works

- Claude is sent a system prompt that includes the **Claude-OSINT**
  skills: ~235 KB of investigative tradecraft and offensive-OSINT
  primers (loaded once at startup; cached via Anthropic prompt caching
  so repeat turns cost ~10% as much).
- When **"Include graph context"** is checked (default on), every
  message you send is prepended with a compact summary of your case:
  entity count, type mix, selected entity, recent additions.
- Replies show token usage so you can verify caching is working —
  expect `cached N (90%+)` after the first turn.

### Things to ask

- "What pivots should I run on the selected entity?"
- "Summarise what we've found so far."
- "Is this WHOIS data suspicious? What stands out?"
- "What does this IP's ASN tell us about the operator?"
- "Compare these two domains — what patterns are shared?"

### Keys

- **Enter** sends. **Shift+Enter** newline. **Esc** closes.
- **⟲** clears the conversation (no DB persistence — chat history is
  ephemeral per session).

### Setup

Set `ANTHROPIC_API_KEY` in the environment before launching. Without
it, the panel shows a clear "key not set" message and the Send button
is disabled.

Tune via:

- `MALTRACE_AI_MODEL` — default `claude-opus-4-7`. Cheaper options:
  `claude-sonnet-4-6`, `claude-haiku-4-5`.
- `MALTRACE_AI_MAX_TOKENS` — default 4096.
- `MALTRACE_CLAUDE_OSINT_PATH` — override the Claude-OSINT checkout
  location.

---

## 11. Secrets (API keys)

Many SpiderFoot modules need API keys (Shodan, VirusTotal, HudsonRock,
HaveIBeenPwned, AbuseIPDB, etc.). maltrace stores those keys per-user
in `secrets.json` (mode 0600 on Unix) in the app data dir.

### Backend API

Three Tauri commands / HTTP endpoints:

- `list_secret_names()` → array of `{name, set: bool}`. Values are
  never returned, only names — keys are write-only.
- `set_secret(name, value)` — stores or replaces.
- `delete_secret(name)` — removes.

### How sidecars use them

On every transform invocation, the registry passes the full secrets
snapshot to the sidecar via `--opts {"secrets": {"shodan_api_key":
"...", ...}}`. The SpiderFoot adapter reads from this and configures
the matching module options.

### UI

A first-pass secrets panel is wired up backend-side; a frontend
manager is the next item on the polish list. In the meantime you can
set keys by calling the Tauri command directly from a script.

---

## 12. Plugins

Every transform comes from a **plugin**. Plugins are discovered at
startup by scanning `plugin.toml` files in:

1. `maltrace/sidecars/*/plugin.toml` (in-tree)
2. `~/.local/share/dev.maltrace.app/plugins/*/plugin.toml` (user-installed)
3. `$MALTRACE_PLUGIN_PATH` directories (test / CI)

Click "**N transforms**" in the topbar status area to open the
**Plugins** modal — lists every loaded plugin with name, version,
license, transform count, plugin directory, homepage, and a
**not runnable** badge if its runner command isn't on disk (typically
means the venv hasn't been set up).

### Built-in plugins

| Plugin | Transforms | Notes |
|---|---|---|
| `spiderfoot` | ~230 | The big catalogue. Many modules require API keys; set them via [§11 Secrets](#11-secrets-api-keys). |
| `face-search` | 6 | `face_detect` (no network) + `face_search_yandex/_bing/_lens/_facecheck/_all` (reverse image search). Needs InsightFace model on first call (~120 MB download to `~/.insightface/`). |
| `brazuca` | 1 | `brazuca_resources` — produces URL entities pointing at relevant Brazilian OSINT portals when entities match CPF / CNPJ / `+55` phone / `.br` patterns. |
| `example-hello` | 1 | Minimal SDK demo. Copy-paste starting point for your own plugin. |

### Writing your own

See [PLUGIN_SDK.md](PLUGIN_SDK.md) — the public contract for plugin
authors. Minimal viable plugin is ~40 lines.

---

## 13. Transform run history

Every transform invocation is logged in the `transform_runs` table:
case_id, transform_id, source entity id + label, started_at,
duration_ms, status (ok / error), new entity count, error message,
stderr excerpt.

Surface via `list_transform_runs(case_id, limit)` (default 100). The
data is there for performance profiling, debugging failed pivots, and
forensic audit of who-ran-what-when-on-what. A dedicated UI panel is
not yet built; query directly via the SQLite db if you need it now.

---

## 14. Exporting

| Format | Endpoint | Use case |
|---|---|---|
| **GraphML** | Case tab right-click → Export, or `export_case_graphml(case_id)` | Hand-off to Maltego / Gephi / yEd / Cytoscape Desktop / NetworkX. Includes positions. |
| **CSV** | `export_case_entities_csv(case_id)` | Spreadsheet, scripting, archive. Flat list of entities (no edges). |

For shipping the full picture (entities + edges + history) elsewhere
the SQLite database is the source of truth — it's a single file at the
location in [§17 Where data lives](#17-where-data-lives), and you can
open it with `sqlite3` or any SQLite GUI.

---

## 15. Keyboard shortcuts

| Key | Action |
|---|---|
| `?` | Toggle the keyboard shortcuts panel |
| `Esc` | Close any open modal / clear selection / deselect input |
| `/` | Focus the transform filter in the inspector |
| `Ctrl` / `Cmd` + `F` | Open the **Find on graph** finder |
| `Del` / `Backspace` | Delete the selected entity (or batch) |
| `Ctrl` / `Cmd` + click | Toggle multi-select |
| Drag empty canvas | Box-select |
| Scroll | Zoom canvas |
| `Enter` (in AI panel) | Send message |
| `Shift+Enter` (in AI panel) | Newline |
| `↑` / `↓` / `Enter` (in any picker) | Navigate / activate |

---

## 16. Environment variables

| Variable | Default | What it controls |
|---|---|---|
| `ANTHROPIC_API_KEY` | unset | Anthropic key for the AI analyst panel. Panel disabled if unset. |
| `MALTRACE_AI_MODEL` | `claude-opus-4-7` | Claude model used by the analyst. |
| `MALTRACE_AI_MAX_TOKENS` | `4096` | Max tokens per AI reply. |
| `MALTRACE_CLAUDE_OSINT_PATH` | sibling `Claude-OSINT/` | Where the AI panel reads the skills from. |
| `MALTRACE_SPIDERFOOT_PATH` | sibling `spiderfoot/` | Where the SpiderFoot sidecar finds its checkout. |
| `MALTRACE_PLUGIN_PATH` | unset | Colon-separated extra plugin discovery dirs (dev / CI). |
| `WEBKIT_DISABLE_DMABUF_RENDERER` | set by npm script | Workaround for WebKitGTK + Wayland crash. Don't unset on Linux. |

---

## 17. Where data lives

Linux (XDG):

| What | Path |
|---|---|
| SQLite DB (cases, entities, edges, transform runs) | `~/.local/share/dev.maltrace.app/maltrace.sqlite` |
| Secrets store | `~/.local/share/dev.maltrace.app/secrets.json` |
| User-installed plugins | `~/.local/share/dev.maltrace.app/plugins/` |
| InsightFace models | `~/.insightface/models/buffalo_l/` |
| SpiderFoot sidecar venv | `maltrace/sidecars/spiderfoot-adapter/.venv/` |
| Face-search sidecar venv | `maltrace/sidecars/face-search/.venv/` |
| Brazuca sidecar venv | `maltrace/sidecars/brazuca/.venv/` |

On macOS the app data dir is `~/Library/Application Support/dev.maltrace.app/`.
On Windows it's `%APPDATA%\dev.maltrace.app\`.

The SQLite DB uses WAL + foreign keys ON. Schema migrations happen on
launch — bumping schema versions is logged to stderr.

---

## 18. Troubleshooting

### App won't open / blank screen on Linux

If `npm run tauri dev` exits with a misleading `vite:esbuild "service
is no longer running"` EPIPE, the underlying cause is almost certainly
WebKitGTK crashing on Wayland. The `tauri` npm script sets
`WEBKIT_DISABLE_DMABUF_RENDERER=1` for you — make sure you're invoking
through that script, not running the raw `tauri` binary.

### A plugin shows "not runnable" in the Plugins modal

The runner command from its `plugin.toml` doesn't exist on disk.
Typically the venv isn't set up. Check the plugin's README — most need
`python3 -m venv .venv && .venv/bin/pip install -r requirements.txt`
in the plugin directory.

### A transform fails immediately

Open the Plugins modal. If the plugin is **not runnable**, see above.
Otherwise, the transform itself raised — check the toast for the
backend error message; check `transform_runs` for the stderr excerpt
if you need detail.

### A SpiderFoot module wants an API key

Look up which module the transform belongs to and which option is the
key (the module's docstring usually says). Set it via `set_secret`.
The sidecar reads the secrets snapshot on every invocation, so no
restart is needed.

### The AI panel says "API key not set"

Set `ANTHROPIC_API_KEY` before launching maltrace. Environment
variables are read at process start — restart the app after setting
it. On macOS/Linux, run from a terminal that has the variable
exported, not from a launcher.

### `face_search_yandex` returns nothing

Yandex's HTML structure changes regularly; the scraper goes stale.
Either wait for an update or use one of the API-backed backends
(`face_search_bing` with `BING_VISUAL_SEARCH_KEY`, `face_search_facecheck`
with `FACECHECK_API_KEY`).

### Python 3.14 + face-search

InsightFace + onnxruntime + opencv don't yet publish wheels for 3.14.
Use Python 3.12. The fastest path is `uv venv --python 3.12 .venv` —
see [`sidecars/face-search/SETUP.md`](../sidecars/face-search/SETUP.md).

### "schema version" warning at startup

You're running a newer maltrace against an older SQLite. The migration
ran automatically; entities and edges may have been rebuilt while
cases were preserved. Normal during walking-skeleton development.

---

## See also

- **[INVESTIGATION_GUIDE.md](INVESTIGATION_GUIDE.md)** — walk-through of a realistic investigation using these features.
- **[PLUGIN_SDK.md](PLUGIN_SDK.md)** — write your own transforms.
- **[DEVELOPMENT.md](DEVELOPMENT.md)** — building and hacking on maltrace itself.
