"""
maltrace ↔ wayback adapter.

Mine the Internet Archive's Wayback Machine via the public **CDX** API.

Given a domain (or URL), the CDX index returns every capture the Archive
holds. That set is gold for OSINT: it reveals historic endpoints, old admin
paths, removed pages and subdomains that no longer resolve in live DNS but
were once crawled. This is fully passive — only the Archive is queried.

Transforms:

    wayback_urls        Domain/URL → URL (distinct historic captures)
    wayback_subdomains  Domain     → INTERNET_NAME (hosts seen in captures)

One-shot JSON protocol shared with the other sidecars.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
import traceback
import urllib.error
import urllib.parse
import urllib.request
from typing import Any, Dict, List, Optional


_HEADERS = {
    "User-Agent": "maltrace-wayback/0.1 (+https://github.com/)",
    "Accept": "application/json",
}
_DEFAULT_TIMEOUT = 30
_DEFAULT_LIMIT = 1000

_DOMAIN_RE = re.compile(r"^(?=.{1,253}$)([A-Za-z0-9_-]+\.)+[A-Za-z]{2,}$")


def _get_json(url: str, *, timeout: float) -> tuple[int, Any]:
    req = urllib.request.Request(url, headers=_HEADERS)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            raw = resp.read()
            status = resp.status
    except urllib.error.HTTPError as e:
        raw = e.read() if hasattr(e, "read") else b""
        status = e.code
    except (urllib.error.URLError, TimeoutError, ConnectionError) as e:
        return 0, {"_error": str(e)}
    text = raw.decode("utf-8", errors="replace")
    try:
        return status, json.loads(text)
    except json.JSONDecodeError:
        return status, text


def _normalise_domain(label: str) -> str:
    s = (label or "").strip().lower()
    if "://" in s:
        s = s.split("://", 1)[1]
    s = s.split("/", 1)[0].split("?", 1)[0].split("@")[-1]
    s = s.split(":", 1)[0]
    return s.lstrip("*.").rstrip(".")


def _is_url(label: str) -> bool:
    return "://" in (label or "")


def _query_cdx(target: str, *, match_domain: bool, fields: str,
               limit: int, timeout: float) -> tuple[int, List[list]]:
    """Query the CDX API. Returns (status, rows) — rows[0] is the header."""
    params = {
        "url": target,
        "output": "json",
        "fl": fields,
        "collapse": "urlkey",
        "limit": str(limit),
    }
    if match_domain:
        params["matchType"] = "domain"
    url = "https://web.archive.org/cdx/search/cdx?" + urllib.parse.urlencode(params)
    status, body = _get_json(url, timeout=timeout)
    if status == 200 and isinstance(body, list):
        return status, body
    return status, []


# ──────────────────────────────────────────────────────────────────────
#  Transforms
# ──────────────────────────────────────────────────────────────────────

def _run_urls(event_type: str, data: str, opts: dict) -> List[dict]:
    timeout = float(opts.get("timeout", _DEFAULT_TIMEOUT))
    limit = int(opts.get("limit", _DEFAULT_LIMIT))

    if _is_url(data):
        target = data.strip()
        match_domain = False
    else:
        target = _normalise_domain(data)
        if not _DOMAIN_RE.match(target):
            return [_raw(f"wayback_urls: {data!r} is not a domain/URL",
                         event_type, "wayback_urls", 30, {"input": data})]
        target = target + "/*"
        match_domain = False

    status, rows = _query_cdx(target, match_domain=match_domain,
                              fields="original,timestamp,statuscode,mimetype",
                              limit=limit, timeout=timeout)
    if len(rows) <= 1:
        return [_raw(f"Wayback: no captures / status={status} for {data}",
                     event_type, "wayback_urls", 40,
                     {"target": target, "status": status})]

    header = rows[0]
    idx = {name: i for i, name in enumerate(header)}
    seen: set = set()
    events: List[dict] = []
    for row in rows[1:]:
        try:
            original = row[idx["original"]]
            ts = row[idx.get("timestamp", 1)] if "timestamp" in idx else None
            sc = row[idx.get("statuscode", 2)] if "statuscode" in idx else None
            mime = row[idx.get("mimetype", 3)] if "mimetype" in idx else None
        except (IndexError, KeyError):
            continue
        if original in seen:
            continue
        seen.add(original)
        snapshot = (f"https://web.archive.org/web/{ts}/{original}"
                    if ts else original)
        events.append({
            "event_type": "URL",
            "data": original,
            "module": "wayback_urls",
            "confidence": 80,
            "source_event_type": event_type,
            "extra": {"url": original, "snapshot_url": snapshot,
                      "first_timestamp": ts, "status_code": sc,
                      "mimetype": mime, "source": "web.archive.org"},
        })

    events.append(_raw(
        f"Wayback {data}: {len(seen)} distinct historic URL(s)",
        event_type, "wayback_urls", 75,
        {"target": target, "url_count": len(seen),
         "capped": len(rows) - 1 >= limit, "limit": limit}))
    return events


def _run_subdomains(event_type: str, data: str, opts: dict) -> List[dict]:
    timeout = float(opts.get("timeout", _DEFAULT_TIMEOUT))
    limit = int(opts.get("limit", 5000))
    domain = _normalise_domain(data)
    if not _DOMAIN_RE.match(domain):
        return [_raw(f"wayback_subdomains: {data!r} is not a domain",
                     event_type, "wayback_subdomains", 30, {"input": data})]

    status, rows = _query_cdx(domain, match_domain=True, fields="original",
                              limit=limit, timeout=timeout)
    if len(rows) <= 1:
        return [_raw(f"Wayback: no captures / status={status} for {domain}",
                     event_type, "wayback_subdomains", 40,
                     {"domain": domain, "status": status})]

    seen: Dict[str, int] = {}
    for row in rows[1:]:
        original = row[0] if row else ""
        host = _host_of(original)
        if host and (host == domain or host.endswith("." + domain)):
            seen[host] = seen.get(host, 0) + 1

    events: List[dict] = []
    for host, count in sorted(seen.items(), key=lambda kv: (-kv[1], kv[0])):
        if host == domain:
            continue
        events.append({
            "event_type": "INTERNET_NAME",
            "data": host,
            "module": "wayback_subdomains",
            "confidence": 75,
            "source_event_type": event_type,
            "extra": {"parent_domain": domain, "capture_count": count,
                      "source": "web.archive.org"},
        })
    events.append(_raw(
        f"Wayback {domain}: {len(seen)} distinct host(s) in captures",
        event_type, "wayback_subdomains", 70,
        {"domain": domain, "host_count": len(seen), "hosts": sorted(seen)[:500]}))
    return events


def _host_of(url: str) -> str:
    try:
        netloc = urllib.parse.urlsplit(url if "://" in url else "http://" + url).netloc
    except ValueError:
        return ""
    return netloc.split("@")[-1].split(":", 1)[0].strip().lower().rstrip(".")


def _raw(msg: str, event_type: str, module: str, conf: int, extra: dict) -> dict:
    return {
        "event_type": "RAW_RIR_DATA",
        "data": msg,
        "module": module,
        "confidence": conf,
        "source_event_type": event_type,
        "extra": extra,
    }


# ──────────────────────────────────────────────────────────────────────
#  Manifest + dispatch
# ──────────────────────────────────────────────────────────────────────

TRANSFORMS = [
    {
        "id": "wayback_urls",
        "name": "Wayback Machine → historic URLs",
        "description": (
            "Recover every distinct URL the Internet Archive captured for "
            "this domain (or under this URL prefix). Surfaces old endpoints, "
            "removed pages and leaked paths. Passive, no API key."
        ),
        "categories": ["Web", "Archive"],
        "group": [],
        "consumes": ["DOMAIN_NAME", "INTERNET_NAME", "URL",
                     "LINKED_URL_INTERNAL"],
        "produces": ["URL", "RAW_RIR_DATA"],
    },
    {
        "id": "wayback_subdomains",
        "name": "Wayback Machine → subdomains",
        "description": (
            "Derive distinct subdomains for this domain from the hostnames "
            "seen across all Internet Archive captures — including hosts "
            "that no longer resolve in live DNS."
        ),
        "categories": ["Web", "Archive", "Domain"],
        "group": [],
        "consumes": ["DOMAIN_NAME", "INTERNET_NAME"],
        "produces": ["INTERNET_NAME", "RAW_RIR_DATA"],
    },
]


def _dispatch_run(module: str, event_type: str, data: str, opts: dict) -> List[dict]:
    if module == "wayback_urls":
        return _run_urls(event_type, data, opts)
    if module == "wayback_subdomains":
        return _run_subdomains(event_type, data, opts)
    raise ValueError(f"unknown module: {module}")


def cmd_list_transforms(_: argparse.Namespace) -> int:
    json.dump({"transforms": TRANSFORMS, "skipped": []}, sys.stdout)
    sys.stdout.write("\n")
    return 0


def cmd_run(args: argparse.Namespace) -> int:
    opts: dict = {}
    if args.opts:
        try:
            opts = json.loads(args.opts)
            if not isinstance(opts, dict):
                raise ValueError("--opts must be a JSON object")
        except (json.JSONDecodeError, ValueError) as exc:
            json.dump({"ok": False, "error": f"bad --opts: {exc}", "events": []},
                      sys.stdout)
            sys.stdout.write("\n")
            return 2

    try:
        events = _dispatch_run(args.module, args.event_type, args.data, opts)
    except Exception as exc:  # noqa: BLE001
        json.dump({"ok": False, "error": f"{type(exc).__name__}: {exc}",
                   "traceback": traceback.format_exc(), "events": []}, sys.stdout)
        sys.stdout.write("\n")
        return 3

    json.dump({"ok": True, "events": events}, sys.stdout)
    sys.stdout.write("\n")
    return 0


def main(argv: Optional[List[str]] = None) -> int:
    parser = argparse.ArgumentParser(prog="maltrace-wayback-adapter")
    sub = parser.add_subparsers(dest="cmd", required=True)
    sub.add_parser("list-transforms", help="List wayback transforms.")
    run = sub.add_parser("run", help="Query the Wayback Machine CDX API.")
    run.add_argument("--module", required=True)
    run.add_argument("--event-type", required=True)
    run.add_argument("--data", required=True)
    run.add_argument("--opts", default=None, help='JSON: {"timeout": 30, "limit": 1000}')
    args = parser.parse_args(argv)
    if args.cmd == "list-transforms":
        return cmd_list_transforms(args)
    if args.cmd == "run":
        return cmd_run(args)
    parser.print_help(sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main())
