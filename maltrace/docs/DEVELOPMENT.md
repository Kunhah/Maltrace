# Development

## Prerequisites

- **Rust** (stable or nightly)
- **Node.js** 20+
- **Python** 3.11–3.13 strongly preferred (3.14 works for SpiderFoot deps
  but currently struggles with InsightFace's ML stack — wheels not yet
  published for 3.14)
- **Tauri 2 Linux deps:** webkit2gtk-4.1, libsoup3 (already present on most
  modern distros)
- A **SpiderFoot** checkout sibling to `maltrace/` (or set
  `MALTRACE_SPIDERFOOT_PATH`)

## Layout

```
maltrace/
├── app/                          Tauri 2 app
│   ├── src/                      React + TypeScript frontend (dual-transport)
│   │   ├── api.ts                Tauri invoke OR fetch — auto-detected
│   │   ├── types.ts              Mirrors Rust serde structs
│   │   └── components/
│   ├── src-tauri/                Rust core
│   │   └── src/
│   │       ├── db.rs             SQLite schema + queries
│   │       ├── ontology.rs       SF events → canonical types
│   │       ├── sidecar.rs        SidecarRegistry: many adapters, one API
│   │       ├── core.rs           Transport-agnostic business logic
│   │       ├── commands.rs       Tauri shells over core
│   │       ├── http.rs           axum shells over core (maltrace-server)
│   │       ├── lib.rs            App setup
│   │       └── bin/server.rs     `maltrace-server` standalone binary
│   └── package.json              dev/server/server:release scripts
├── sidecars/
│   ├── spiderfoot-adapter/       SF transforms (231+ modules)
│   │   ├── adapter.py            CLI: list-transforms | run
│   │   └── .venv/                (created by you)
│   └── face-search/              InsightFace + Yandex reverse-image
│       ├── adapter.py            CLI: list-transforms | run
│       ├── requirements.txt
│       └── .venv/                (created by you)
└── docs/
```

## First-time setup

**Easy mode:** `./scripts/setup.sh` from the repo root does everything
below (and clones the sibling `spiderfoot/` checkout if missing). The
rest of this section is the manual breakdown if you want to wire things
up by hand.

```bash
# 1. Frontend deps
cd app && npm install

# 2. SpiderFoot sidecar
cd ../sidecars/spiderfoot-adapter
python3 -m venv .venv
.venv/bin/pip install \
  requests netaddr dnspython publicsuffixlist beautifulsoup4 pyyaml \
  ipwhois python-whois cryptography pyOpenSSL phonenumbers ipaddr pysocks \
  networkx adblockparser ExifRead Mako CherryPy cherrypy-cors PyPDF2 secure

.venv/bin/python adapter.py run --module sfp_whois \
  --event-type DOMAIN_NAME --data example.com
# Expected: JSON with {"ok": true, "events": [...]}.

# 3. Face-search sidecar (heavier — pulls InsightFace, ONNX, OpenCV)
cd ../face-search
python3 -m venv .venv
.venv/bin/pip install -r requirements.txt
# First call to face_detect downloads ~120 MB of model weights to ~/.insightface/

.venv/bin/python adapter.py run --module face_detect \
  --event-type IMAGE --data /path/to/photo.jpg
# Expected: JSON with one FACE event per detected face.
```

> **Python 3.14 caveat:** InsightFace / onnxruntime / opencv don't yet ship
> wheels for 3.14. The pip install above will compile them from source on
> 3.14, which can take 20+ minutes and may fail for some sub-deps. If you
> hit problems, install Python 3.12 via your distro (`pacman -S python312`
> on Arch) or via `pyenv install 3.12` and create the venv with that:
> `python3.12 -m venv .venv`.

## Run the desktop app

```bash
cd maltrace/app
npm run tauri dev
```

`WEBKIT_DISABLE_DMABUF_RENDERER=1` is baked into the `tauri` npm script —
without it, WebKitGTK 2.42+ crashes on Wayland with a misleading "esbuild
EPIPE" downstream symptom.

## Run the headless server (web build)

```bash
cd maltrace/app
npm run build                # produces ../dist
npm run server               # starts axum on http://127.0.0.1:1421
# or: npm run server -- --port 8080 --static ../dist --host 0.0.0.0
```

The React frontend auto-detects which transport to use: Tauri's `invoke`
inside the desktop app, `fetch /api/*` in a browser.

## Smoke test (desktop)

1. App opens with an empty dark canvas.
2. Top bar: pick `Domain`, type `example.com`, click **Add**.
3. Click the new node → inspector shows compatible transforms.
4. Filter for `whois` → click `sfp_whois` → `DOMAIN_WHOIS` + `DOMAIN_REGISTRAR`
   nodes appear, both as canonical `WhoisRecord`.
5. Drag nodes; positions persist. Close and reopen — graph reloads.

## Architecture cheat-sheet

```
React (Cytoscape)                   ← dual-transport
   ↕ Tauri invoke (desktop)  /  fetch /api/* (web)
Rust core (transport-agnostic in core.rs)
   - SQLite (WAL, foreign keys ON)
   - Cases / Entities / Edges
   - Canonical ontology (ontology.rs)
   ↕ subprocess spawn (via SidecarRegistry)
Python adapters (one-shot per call)
   - spiderfoot-adapter: imports sfp_* directly
   - face-search:        InsightFace + Yandex
```

Each transform invocation is `<sidecar>/.venv/bin/python adapter.py run …`.
The Rust `SidecarRegistry` caches a `transform_id → sidecar_id` index after
the first `list-transforms`, so subsequent invocations dispatch in O(1).

## Where data lives

- **App SQLite DB:** Linux: `~/.local/share/dev.maltrace.app/maltrace.sqlite`
- **SpiderFoot sidecar venv:** `sidecars/spiderfoot-adapter/.venv/`
- **Face-search sidecar venv:** `sidecars/face-search/.venv/`
- **InsightFace models:** `~/.insightface/models/buffalo_l/` (downloaded on first use)

## Adding a new sidecar

1. Create `sidecars/<name>/` with `adapter.py` exposing two CLI subcommands:
   - `list-transforms` → `{"transforms": [...], "skipped": []}`
   - `run --module ID --event-type T --data D` → `{"ok": true, "events": [...]}`
2. Create a venv: `python3 -m venv .venv && .venv/bin/pip install -r requirements.txt`
3. Register one line in `app/src-tauri/src/sidecar.rs` `SidecarRegistry::from_repo_layout()`.

No other code changes needed — the registry handles list+dispatch.

## Known walking-skeleton limitations

- Single case (no case picker yet).
- Transforms run synchronously; UI shows a spinner but doesn't queue.
- Initial node positions are a ring around the source.
- No Claude-OSINT panel yet (planned next).
- No brazuca resource hints yet (planned).
- `face_search_yandex` is best-effort scraping — fragile, may break.
