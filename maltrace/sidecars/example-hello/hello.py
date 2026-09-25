"""Minimal maltrace plugin.

Demonstrates the smallest possible implementation of the maltrace sidecar
JSON protocol (version 1). One transform; ~40 lines of pure stdlib Python.

Protocol:
  python3 hello.py list-transforms
    → {"transforms": [...], "skipped": []}

  python3 hello.py run --module example_hello --event-type DOMAIN_NAME --data example.com
    → {"ok": true, "events": [...]}

See ../../docs/PLUGIN_SDK.md for the full spec.
"""

from __future__ import annotations

import argparse
import json
import sys
import traceback


TRANSFORMS = [
    {
        "id": "example_hello",
        "name": "Example: hello",
        "description": "Says hello to a domain. Demonstrates the plugin SDK.",
        "categories": ["Example"],
        "group": [],
        "consumes": ["DOMAIN_NAME", "INTERNET_NAME"],
        "produces": ["DESCRIPTION_ABSTRACT"],
    },
]


def cmd_list_transforms(_):
    json.dump({"transforms": TRANSFORMS, "skipped": []}, sys.stdout)
    sys.stdout.write("\n")
    return 0


def cmd_run(args):
    if args.module != "example_hello":
        json.dump(
            {"ok": False, "error": f"unknown module: {args.module}", "events": []},
            sys.stdout,
        )
        return 2

    try:
        events = [
            {
                "event_type": "DESCRIPTION_ABSTRACT",
                "data": f"hello, {args.data}",
                "module": "example_hello",
                "confidence": 100,
                "source_event_type": args.event_type,
                "extra": {"greeting": "hello", "subject": args.data},
            }
        ]
    except Exception as exc:  # noqa: BLE001
        json.dump(
            {
                "ok": False,
                "error": f"{type(exc).__name__}: {exc}",
                "traceback": traceback.format_exc(),
                "events": [],
            },
            sys.stdout,
        )
        return 3

    json.dump({"ok": True, "events": events}, sys.stdout)
    sys.stdout.write("\n")
    return 0


def main():
    parser = argparse.ArgumentParser(prog="example-hello")
    sub = parser.add_subparsers(dest="cmd", required=True)
    sub.add_parser("list-transforms")
    run = sub.add_parser("run")
    run.add_argument("--module", required=True)
    run.add_argument("--event-type", required=True)
    run.add_argument("--data", required=True)
    run.add_argument("--opts", default=None)
    args = parser.parse_args()
    if args.cmd == "list-transforms":
        return cmd_list_transforms(args)
    if args.cmd == "run":
        return cmd_run(args)
    return 2


if __name__ == "__main__":
    sys.exit(main())
