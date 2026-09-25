# spiderfoot-adapter

One-shot CLI bridge that lets the maltrace Rust core run individual
SpiderFoot `sfp_*` modules without spinning up SpiderFoot's full scan engine.

## Why a sidecar

SpiderFoot is Python; the maltrace core is Rust. Rather than reimplement 237
modules, we shell out to this script. Each invocation:

1. Imports a single `sfp_*` module from the SpiderFoot checkout.
2. Stubs the module's `notifyListeners` to capture, not dispatch, events.
3. Hands it a synthetic input event.
4. Prints the produced events as JSON on stdout.

Stdout is structured JSON only. Stderr carries human diagnostics.

## Requirements

- Python 3.11+
- A SpiderFoot checkout (any recent version)
- SpiderFoot's runtime deps installed in the same env: `pip install -r ../../../spiderfoot/requirements.txt`

The adapter finds SpiderFoot via:
1. `$MALTRACE_SPIDERFOOT_PATH` if set, else
2. a sibling `spiderfoot/` directory next to `maltrace/`.

## Usage

```bash
# List every loadable transform.
python adapter.py list-transforms

# Run one transform.
python adapter.py run --module sfp_whois --event-type DOMAIN_NAME --data example.com
```

Exit codes: `0` ok, `2` usage error, `3` module raised an exception
(traceback included in JSON output).
