"""
maltrace ↔ SpiderFoot adapter.

One-shot CLI that:
  - Lists available SpiderFoot transforms (sfp_* modules) with their I/O types.
  - Runs a single transform against one input event and prints the produced
    events as JSON.

Invoked by the Rust core via subprocess. Stdout is JSON only; stderr carries
human-readable diagnostics. Exit codes: 0 = ok, 2 = bad usage, 3 = module
error.

The adapter imports `sfp_*` modules directly from a sibling SpiderFoot
checkout. We bypass SpiderFoot's scan engine entirely: a module is
instantiated, given a stubbed `SpiderFoot` context, and its `notifyListeners`
is monkey-patched so emitted events are captured instead of dispatched.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import time
import traceback
from pathlib import Path
from typing import Any


def _resolve_spiderfoot_path() -> Path:
    """Return the path to the SpiderFoot checkout to import from."""
    env = os.environ.get("MALTRACE_SPIDERFOOT_PATH")
    if env:
        p = Path(env).expanduser().resolve()
        if not (p / "sflib.py").exists():
            sys.exit(
                f"MALTRACE_SPIDERFOOT_PATH={p} does not look like a "
                "SpiderFoot checkout (no sflib.py)."
            )
        return p
    here = Path(__file__).resolve()
    # Walk up looking for a sibling `spiderfoot/sflib.py`. Default layout:
    #   <root>/maltrace/sidecars/spiderfoot-adapter/adapter.py
    #   <root>/spiderfoot/sflib.py
    for candidate in [
        here.parent.parent.parent.parent / "spiderfoot",
        here.parent.parent.parent / "spiderfoot",
    ]:
        if (candidate / "sflib.py").exists():
            return candidate.resolve()
    sys.exit(
        "Could not locate SpiderFoot. Set MALTRACE_SPIDERFOOT_PATH or place "
        "the spiderfoot checkout as a sibling to the maltrace directory."
    )


SPIDERFOOT_PATH = _resolve_spiderfoot_path()
sys.path.insert(0, str(SPIDERFOOT_PATH))

from spiderfoot import (  # noqa: E402
    SpiderFootEvent,
    SpiderFootHelpers,
    SpiderFootTarget,
)
from sflib import SpiderFoot  # noqa: E402


def _make_target(data: str, event_type: str) -> SpiderFootTarget | None:
    """Build the scan-seed target a module reaches for via `getTarget()`.

    Modules call `self.getTarget().matches(value)` to decide whether a
    resolved address belongs to the thing under investigation. That requires
    a real `SpiderFootTarget`, not the raw string — otherwise `handleEvent`
    dies with `'str' object has no attribute 'matches'` and emits no events.

    We derive the target type the same way SpiderFoot's scan engine does
    (from the value), falling back to the input event type when the value
    isn't self-describing. Returns None when neither yields a valid target
    type; the caller then leaves the module target-less (fine for modules
    that never consult it).
    """
    target_type = SpiderFootHelpers.targetTypeFromString(data)
    if target_type is None and event_type in SpiderFootTarget._validTypes:
        target_type = event_type
    if target_type is None:
        return None
    return SpiderFootTarget(data, target_type)


# The FULL set of SpiderFoot global options. Modules reach into
# `self.sf.opts[...]` AND their own merged `self.opts[...]` for these without
# guarding — e.g. `self.debug()` reads `opts['_debug']`, `resolveHost` reads
# the `_socks*`/`_dnsserver` keys, and `processHost` reads
# `self.opts['_internettlds']`. Omitting any of them surfaces as a `KeyError`
# mid-`handleEvent`, which the runner reports as a failed transform with zero
# events ("ran the machine, got no data"). Keep this in sync with `sfConfig`
# in the SpiderFoot checkout's `sf.py`.
GLOBAL_OPTS: dict[str, Any] = {
    "_debug": False,
    "_maxthreads": 3,
    "__logging": False,
    "__outputfilter": None,
    "_useragent": "maltrace/0.1 (+https://github.com/)",
    "_dnsserver": "",
    "_fetchtimeout": 5,
    "_internettlds": "https://publicsuffix.org/list/effective_tld_names.dat",
    "_internettlds_cache": 72,
    "_genericusers": "",
    "_socks1type": "",
    "_socks2addr": "",
    "_socks3port": "",
    "_socks4user": "",
    "_socks5pwd": "",
}


def _make_context() -> SpiderFoot:
    return SpiderFoot(dict(GLOBAL_OPTS))


def _resolve_internet_tlds(sfc: SpiderFoot) -> list[str]:
    """Resolve `_internettlds` from its URL into the actual TLD list.

    Modules pass `self.opts['_internettlds']` straight to `hostDomain`, which
    expects a *list* of public-suffix lines, not the source URL. SpiderFoot's
    scan engine resolves+caches this once at scan start (see `sfscan.py`); we
    do the same here, leaning on SpiderFoot's own on-disk cache so we only hit
    the network on a cold cache. Returns `[]` when offline with no cache —
    `hostDomain` treats an empty list as "unknown" and degrades gracefully
    instead of crashing.
    """
    tld_data = sfc.cacheGet("internet_tlds", GLOBAL_OPTS["_internettlds_cache"])
    if tld_data is None:
        try:
            tld_data = sfc.optValueToData(GLOBAL_OPTS["_internettlds"])
        except Exception:  # noqa: BLE001 — network/parse failure is non-fatal
            tld_data = None
        if tld_data:
            sfc.cachePut("internet_tlds", tld_data)
    return tld_data.splitlines() if tld_data else []


def _module_opts(plugin: Any, sfc: SpiderFoot) -> dict[str, Any]:
    """Build the opts dict a module's `setup()` expects.

    Mirrors `sfscan.py`: start from the module's own option defaults, then
    layer every global `_*` option on top, then swap the resolved TLD list in
    for the `_internettlds` URL.
    """
    opts = dict(getattr(plugin, "opts", {}) or {})
    opts.update(GLOBAL_OPTS)
    opts["_internettlds"] = _resolve_internet_tlds(sfc)
    return opts


def cmd_list_transforms(_: argparse.Namespace) -> int:
    """Print a JSON array of every loadable sfp_* module's metadata.

    Modules whose optional Python dependencies are missing are skipped and
    reported in `skipped`, rather than aborting the whole listing.
    """
    import importlib.util

    modules_dir = SPIDERFOOT_PATH / "modules"
    out: list[dict[str, Any]] = []
    skipped: list[dict[str, str]] = []

    for entry in sorted(modules_dir.iterdir()):
        if not entry.name.startswith("sfp_") or not entry.name.endswith(".py"):
            continue
        if entry.name == "sfp_template.py":
            continue
        mod_name = entry.stem

        try:
            spec = importlib.util.spec_from_file_location(mod_name, entry)
            if spec is None or spec.loader is None:
                raise ImportError("no spec")
            mod = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(mod)
            plugin_cls = getattr(mod, mod_name, None)
            if plugin_cls is None:
                raise ImportError(f"no class {mod_name}")
            inst = plugin_cls()
            info = inst.asdict()
        except Exception as exc:  # noqa: BLE001
            skipped.append({"id": mod_name, "reason": f"{type(exc).__name__}: {exc}"})
            continue

        out.append(
            {
                "id": mod_name,
                "name": info.get("name") or mod_name,
                "description": info.get("descr") or "",
                "categories": info.get("cats") or [],
                "group": info.get("group") or [],
                "consumes": info.get("consumes") or [],
                "produces": info.get("provides") or [],
            }
        )

    json.dump({"transforms": out, "skipped": skipped}, sys.stdout)
    sys.stdout.write("\n")
    return 0


def _event_extra(evt: SpiderFootEvent, module_id: str) -> dict[str, Any]:
    """Build a JSON-serializable metadata dict for an emitted SpiderFoot event.

    Populates the inspector-facing `extra` blob the Rust core stores in the
    entity's `data` JSON (and reads `extra.url` from to fill `source_url`).
    Only attributes that actually exist on `SpiderFootEvent` are included
    (see spiderfoot/event.py): module, confidence, visibility, risk, generated
    (epoch float), hash, sourceEventHash. Each is read via getattr with a safe
    default so a quirky module can never crash the run. Keys are snake_case.

    `url` is set ONLY when the event data is itself an http(s) URL — common for
    SpiderFoot URL-type events (e.g. LINKED_URL_*). We never fabricate one.
    """
    extra: dict[str, Any] = {}

    module = getattr(evt, "module", None) or module_id
    if module:
        extra["module"] = module

    confidence = getattr(evt, "confidence", None)
    if confidence is not None:
        extra["confidence"] = confidence

    visibility = getattr(evt, "visibility", None)
    if visibility is not None:
        extra["visibility"] = visibility

    risk = getattr(evt, "risk", None)
    if risk is not None:
        extra["risk"] = risk

    generated = getattr(evt, "generated", None)
    if isinstance(generated, (int, float)):
        try:
            extra["generated"] = time.strftime(
                "%Y-%m-%dT%H:%M:%SZ", time.gmtime(generated)
            )
        except (ValueError, OSError, OverflowError):
            pass

    event_hash = getattr(evt, "hash", None)
    if isinstance(event_hash, str) and event_hash:
        extra["hash"] = event_hash

    source_event_hash = getattr(evt, "sourceEventHash", None)
    if isinstance(source_event_hash, str) and source_event_hash:
        extra["source_event_hash"] = source_event_hash

    data = getattr(evt, "data", None)
    if isinstance(data, str) and (
        data.startswith("http://") or data.startswith("https://")
    ):
        extra["url"] = data

    return extra


def cmd_run(args: argparse.Namespace) -> int:
    """Run a single transform and emit produced events as JSON."""
    module_id: str = args.module
    event_type: str = args.event_type
    data: str = args.data

    modules_dir = SPIDERFOOT_PATH / "modules"
    module_file = modules_dir / f"{module_id}.py"
    if not module_file.exists():
        sys.exit(f"Unknown module: {module_id}")

    # Import the module class. `loadModulesAsDict` already does this, but it
    # also instantiates every module on disk — heavy for a single-shot call.
    import importlib.util

    spec = importlib.util.spec_from_file_location(module_id, module_file)
    if spec is None or spec.loader is None:
        sys.exit(f"Could not load module spec for {module_id}")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    plugin_cls = getattr(mod, module_id, None)
    if plugin_cls is None:
        sys.exit(f"Module {module_id} has no class named {module_id}")

    plugin = plugin_cls()
    plugin.__name__ = module_id

    captured: list[dict[str, Any]] = []

    def capture(evt: SpiderFootEvent) -> None:
        captured.append(
            {
                "event_type": evt.eventType,
                "data": evt.data,
                "module": getattr(evt, "module", module_id),
                "confidence": getattr(evt, "confidence", 100),
                "source_event_type": (
                    getattr(evt.sourceEvent, "eventType", None)
                    if getattr(evt, "sourceEvent", None) is not None
                    else None
                ),
                "extra": _event_extra(evt, module_id),
            }
        )

    plugin.notifyListeners = capture  # type: ignore[method-assign]

    sfc = _make_context()
    # Module opts = module defaults + globals + resolved TLD list, then any
    # caller overrides on top (e.g. API keys passed via --opts).
    module_opts = _module_opts(plugin, sfc)
    if args.opts:
        module_opts.update(json.loads(args.opts))
    plugin.setup(sfc, module_opts)

    target = _make_target(data, event_type)
    if target is not None:
        plugin.setTarget(target)

    root = SpiderFootEvent("ROOT", data, "maltrace", None)
    input_event = SpiderFootEvent(event_type, data, "maltrace", root)

    try:
        plugin.handleEvent(input_event)
    except Exception as exc:  # noqa: BLE001
        json.dump(
            {
                "ok": False,
                "error": f"{type(exc).__name__}: {exc}",
                "traceback": traceback.format_exc(),
                "events": captured,
            },
            sys.stdout,
        )
        sys.stdout.write("\n")
        return 3

    json.dump({"ok": True, "events": captured}, sys.stdout)
    sys.stdout.write("\n")
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="maltrace-spiderfoot-adapter")
    sub = parser.add_subparsers(dest="cmd", required=True)

    sub.add_parser("list-transforms", help="List all sfp_* modules and their I/O types.")

    run = sub.add_parser("run", help="Run one transform against one input event.")
    run.add_argument("--module", required=True, help="e.g. sfp_whois")
    run.add_argument("--event-type", required=True, help="e.g. DOMAIN_NAME")
    run.add_argument("--data", required=True, help="The event payload, e.g. example.com")
    run.add_argument(
        "--opts",
        default=None,
        help="JSON string of module-specific options (optional).",
    )

    args = parser.parse_args(argv)
    if args.cmd == "list-transforms":
        return cmd_list_transforms(args)
    if args.cmd == "run":
        return cmd_run(args)
    parser.print_help(sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main())
