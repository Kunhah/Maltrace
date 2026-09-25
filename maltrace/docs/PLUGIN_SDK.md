# maltrace Plugin SDK

This document is the **public contract** for writing a maltrace plugin.

A plugin is a directory containing a `plugin.toml` manifest and a runnable
entry script (any language — Python, Bash, Node, Go, anything that can read
JSON from argv/stdin and write JSON to stdout). maltrace discovers plugins
at startup by scanning a few well-known directories; once discovered, every
transform a plugin contributes is available in the canvas right-click menu
and inspector palette alongside the built-in ones.

The SDK is intentionally small. Everything you need to ship a transform fits
in two files — a `plugin.toml` and a script — and we keep that promise as
the protocol evolves.

## TL;DR — a 40-line plugin

See [`sidecars/example-hello/`](../sidecars/example-hello/). Copy the
directory, change the `id` in `plugin.toml`, edit the script. That's it.

```toml
# plugin.toml
[plugin]
id = "my-plugin"
name = "My plugin"
version = "0.1.0"
protocol_version = 1
license = "MIT"
description = "Says hello to things."

[runner]
command = "/usr/bin/env"
args = ["python3", "hello.py"]
```

```python
# hello.py — exact contract below
import argparse, json, sys

TRANSFORMS = [{
    "id": "my_hello",
    "name": "Say hello",
    "description": "Says hello to a domain.",
    "categories": ["Example"],
    "group": [],
    "consumes": ["DOMAIN_NAME"],
    "produces": ["DESCRIPTION_ABSTRACT"],
}]

p = argparse.ArgumentParser()
sub = p.add_subparsers(dest="cmd", required=True)
sub.add_parser("list-transforms")
r = sub.add_parser("run")
r.add_argument("--module", required=True)
r.add_argument("--event-type", required=True)
r.add_argument("--data", required=True)
r.add_argument("--opts", default=None)
a = p.parse_args()

if a.cmd == "list-transforms":
    print(json.dumps({"transforms": TRANSFORMS, "skipped": []}))
else:
    print(json.dumps({"ok": True, "events": [{
        "event_type": "DESCRIPTION_ABSTRACT",
        "data": f"hello, {a.data}",
        "source_event_type": a.event_type,
    }]}))
```

Drop the directory in `maltrace/sidecars/`, restart, and the transform
appears in the menu when you right-click a `Domain` entity.

---

## Discovery

At startup, maltrace scans these directories in order. The **first** plugin
to claim a given `id` wins; later duplicates are skipped with a stderr
warning.

| # | Path | Purpose |
|---|------|---------|
| 1 | `<repo>/maltrace/sidecars/*/plugin.toml` | In-tree plugins (development) |
| 2 | `<XDG_DATA_HOME>/dev.maltrace.app/plugins/*/plugin.toml` | User-installed plugins (typically `~/.local/share/dev.maltrace.app/plugins/`) |
| 3 | `$MALTRACE_PLUGIN_PATH` (`:`-separated dirs) | Test / CI override |

To install a third-party plugin for personal use, drop its directory into
the user data dir and restart. Restart is required — there's no hot reload
yet.

To verify what's loaded:

- Backend log: `maltrace: no plugins found. Looked in: [...]` (or no log if
  plugins loaded fine — silence is success).
- Tauri command: `list_plugins` returns the resolved set.
- The topbar status area shows the loaded plugin count once that UI lands.

---

## `plugin.toml` schema

```toml
[plugin]
id               = "string"   # required — unique within an install; [A-Za-z0-9_-]+
name             = "string"   # required — human label in UI
version          = "0.1.0"    # required — SemVer; cosmetic for now
protocol_version = 1          # required — must equal current PROTOCOL_VERSION
license          = "MIT"      # optional — SPDX id
description      = "..."      # optional
homepage         = "https://" # optional
authors          = ["..."]    # optional

[runner]
command = "/usr/bin/env"      # required — relative paths resolve against the plugin dir
args    = ["python3", "x.py"] # optional — fixed args prepended to every invocation

[runner.env]                  # optional — extra env vars set on the subprocess
PYTHONUNBUFFERED = "1"

[requirements]                # optional — human-readable hints; not enforced
python  = ">=3.11"
system  = ["libfoo-dev"]
network = true
```

### Field reference

- **`plugin.id`** — Stable identifier. Used as the `source` field on every
  transform the plugin contributes, and as the dedup key during discovery.
  ASCII alphanumerics + `-` + `_`. Don't change this between versions of
  the same plugin.
- **`plugin.protocol_version`** — Which sidecar JSON protocol this plugin
  implements. **Current version: `1`.** Plugins declaring a different
  version are skipped with a stderr warning so users get an explicit
  "this needs an update" signal instead of mysterious failures.
- **`plugin.version`** — SemVer. We don't enforce any policy yet but please
  bump it on releases.
- **`runner.command`** — Path to the executable. **Relative paths resolve
  against the plugin directory.** That's how `.venv/bin/python` works for
  the bundled plugins. Absolute paths pass through unchanged.
- **`runner.args`** — Fixed args prepended before the maltrace subcommand
  (`list-transforms` or `run …`). Typically just `["adapter.py"]` or
  similar.
- **`runner.env`** — Per-plugin environment variables. The plugin also
  inherits the maltrace process's env; this is for things you want to pin
  per-plugin.
- **`requirements.*`** — Documentation only. Surfaced in the UI's
  "plugin info" tooltip so users know what to install before the runner
  works.

---

## JSON protocol (version 1)

The plugin is invoked as a subprocess. **Stdout must be JSON only.**
Diagnostics go to stderr. Anything that prints to stdout that isn't part of
the JSON response will break parsing.

### `list-transforms`

```
<runner.command> <runner.args>… list-transforms
```

Stdout response:

```json
{
  "transforms": [
    {
      "id": "my_hello",
      "name": "Say hello",
      "description": "Says hello to a domain.",
      "categories": ["Example"],
      "group": [],
      "consumes": ["DOMAIN_NAME"],
      "produces": ["DESCRIPTION_ABSTRACT"]
    }
  ],
  "skipped": [
    { "id": "broken_thing", "reason": "missing dependency: foo" }
  ]
}
```

| Field | Required | Notes |
|---|---|---|
| `transforms[].id` | ✓ | Unique within this plugin |
| `transforms[].name` | ✓ | Human label in the menu |
| `transforms[].description` | optional | Tooltip text |
| `transforms[].categories` | optional | Hint for grouping; not enforced today |
| `transforms[].group` | optional | Subgroup hint |
| `transforms[].consumes` | ✓ | SF event types this transform accepts (e.g. `["DOMAIN_NAME"]`). Use `"*"` to mean "any event". See [ontology.rs](../app/src-tauri/src/ontology.rs) for the canonical list. |
| `transforms[].produces` | ✓ | SF event types this transform emits |
| `skipped[]` | optional | Modules that couldn't load (missing deps, etc.) — surfaced for diagnostics |

Exit code: 0 on success. Non-zero on usage error; maltrace logs the stderr
and drops the plugin from the catalogue for that run.

### `run`

```
<runner.command> <runner.args>… run --module <id> --event-type <SF_EVENT> --data <VALUE> [--opts <JSON>]
```

Stdout response:

```json
{
  "ok": true,
  "events": [
    {
      "event_type": "URL",
      "data": "https://example.com",
      "module": "my_hello",
      "confidence": 80,
      "source_event_type": "DOMAIN_NAME",
      "extra": { "title": "Example homepage" }
    }
  ]
}
```

| Field | Required | Notes |
|---|---|---|
| `ok` | ✓ | `true` = events are valid; `false` = transform raised an exception |
| `events[].event_type` | ✓ | SF event type emitted (becomes the new entity's `source_event_type`; the canonical type is resolved by maltrace) |
| `events[].data` | ✓ | The payload — for `URL` it's the URL, for `DOMAIN_NAME` it's the domain, etc. Also used as the on-graph label (truncated to ~80 chars) |
| `events[].module` | optional | Defaults to the transform id |
| `events[].confidence` | optional | 0–100; surfaced in the inspector |
| `events[].source_event_type` | optional | Echo of the input SF event type |
| `events[].extra` | optional | **Free-form structured metadata.** Lives on `entity.data` as JSON, surfaced in the inspector. Use it for richer context: brazuca puts `{url, title, description, category}` here; face_detect puts `{bbox, det_score, source_image}`. |
| `error` | only if `ok: false` | Human-readable error string |
| `traceback` | optional | Useful for debugging plugin crashes |

Exit codes:

- `0` — `ok: true`. Events on stdout.
- `2` — Usage error (bad args). maltrace surfaces stderr.
- `3` — Transform raised. Stdout is still valid JSON with `ok: false`; this
  is the recommended path for "I tried but it didn't work" failures —
  user-visible error rather than a crash.

---

## Special canvas behaviors

Some `extra` fields trigger first-class UI behavior — using them is
optional but lets your plugin feel native:

- **`extra.url`** on any event — the entity inspector shows an "Open"
  button. Only `http:` / `https:` URLs are honored (security filter).
  Use this to emit results that map to a real web page.
- **Event types that map to existing canonical types** — see
  [ontology.rs](../app/src-tauri/src/ontology.rs). Your event becomes a
  first-class entity with the right color/icon. Unknown event types fall
  back to `Generic` and still work; they're just less pretty.
- **High-volume outputs** — if a single `run` returns more than
  `COLLECTION_THRESHOLD` (currently 10) events of one canonical type, maltrace
  automatically collapses them into a Collection node the user can expand.
  You don't have to do anything; just emit the events.

---

## Examples in the repo

| Plugin | What it shows |
|---|---|
| [`sidecars/example-hello/`](../sidecars/example-hello/) | Minimal plugin — copy-paste starting point. |
| [`sidecars/spiderfoot-adapter/`](../sidecars/spiderfoot-adapter/) | Heavy adapter — wraps 230+ external modules; dynamic transform discovery (catalogues vary based on installed deps); long-running deps. |
| [`sidecars/face-search/`](../sidecars/face-search/) | Multi-backend pattern (Yandex / Bing / Google Lens / FaceCheck) selected per-transform; image input via path or URL; uses `extra` for bounding boxes. |
| [`sidecars/brazuca/`](../sidecars/brazuca/) | Pattern-matching gate (regex on label decides which transforms apply); rich `extra` for inspector display. |

---

## Versioning the protocol

The current protocol version is `1`. We'll bump this only when we make a
breaking change. When that happens:

- Old plugins continue to parse manifest-wise but are skipped at the
  discovery step with a clear warning: *"plugin X declares protocol_version
  N but maltrace implements M"*.
- Migration notes will land here as a section explaining what changed
  between versions.
- The current protocol is **purely additive** as far as `extra` goes —
  adding new fields to `extra` is not a breaking change; the Rust core
  passes it through as a `serde_json::Value`.

---

## Best practices

- **Stdout is JSON only.** Pipe debug prints to stderr (`print(..., file=sys.stderr)`
  in Python) or to a file. Stray stdout output will break the parser.
- **Don't block forever.** Set timeouts on network calls; the user will
  Ctrl-C the app if the UI hangs, and the partial result of a 10-minute
  scrape is rarely worth that.
- **Return `ok: false`, not exit-1, for "expected" failures.** Use exit-3
  with the traceback for true crashes. The user-facing error string is what
  the inspector shows.
- **Be pure-stdlib when possible.** Every dependency you add is one the
  user has to install. The example plugin is intentionally zero-dep.
- **Set up a venv** for anything heavier than stdlib. The convention is
  `.venv/bin/python` next to your `adapter.py`. The Rust core treats
  missing venvs gracefully — the plugin just won't run until you set it up.
- **Use `extra` liberally.** It's free-form JSON, surfaced in the
  inspector, and not parsed by Rust beyond passing it through. Anything
  you'd want a user to see when they click a result entity belongs here.

---

## Future work

These are out of scope for protocol v1 but on the roadmap:

- **Hot reload** — currently you restart the app after dropping a plugin in.
- **Plugin marketplace / signing** — currently all plugins are trusted.
- **Streaming** — currently one-shot subprocess per call. A long-lived
  stdio-JSONL daemon mode would help for transforms that are expensive to
  initialise (e.g. loading large ML models).
- **Per-plugin permissions** — currently every plugin can do anything the
  user can.

If you're building a plugin and run into a gap, please file an issue.
