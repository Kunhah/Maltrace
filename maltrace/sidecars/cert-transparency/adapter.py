"""
maltrace ↔ cert-transparency adapter.

Mine the public Certificate Transparency logs via **crt.sh** for a domain.

CT logs are an append-only public record of every TLS certificate issued by
participating CAs. Querying them for `%.example.com` surfaces subdomains that
DNS brute-force will never find (because the operator quietly requested a cert
for them), plus the issuing CA and any organisation name baked into the cert
subject. This is a passive query — crt.sh is hit, never the target.

Transforms:

    crtsh_subdomains  Domain → INTERNET_NAME (every distinct host seen in CT)
    crtsh_certs       Domain → SSL_CERTIFICATE_ISSUED + SSL_CERTIFICATE_ISSUER

One-shot JSON protocol shared with the other sidecars:

    python adapter.py list-transforms
    python adapter.py run --module crtsh_subdomains --event-type DOMAIN_NAME --data example.com

Stdout: JSON only. Stderr: diagnostics. Pure stdlib (urllib + json).
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


# ──────────────────────────────────────────────────────────────────────
#  HTTP helper
# ──────────────────────────────────────────────────────────────────────

_HEADERS = {
    "User-Agent": "maltrace-cert-transparency/0.1 (+https://github.com/)",
    "Accept": "application/json",
}
_DEFAULT_TIMEOUT = 30  # crt.sh can be slow for popular domains.


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


# ──────────────────────────────────────────────────────────────────────
#  Domain normalisation
# ──────────────────────────────────────────────────────────────────────

_DOMAIN_RE = re.compile(r"^(?=.{1,253}$)([A-Za-z0-9_*-]+\.)+[A-Za-z]{2,}$")


def _normalise_domain(label: str) -> str:
    """Strip a URL / scheme / leading wildcard down to a bare apex-ish host."""
    s = (label or "").strip().lower()
    if "://" in s:
        s = s.split("://", 1)[1]
    s = s.split("/", 1)[0].split("?", 1)[0].split("@")[-1]
    s = s.split(":", 1)[0]  # drop port
    s = s.lstrip("*.").rstrip(".")
    return s


def _looks_like_host(s: str) -> bool:
    return bool(_DOMAIN_RE.match(s)) and ".." not in s


# ──────────────────────────────────────────────────────────────────────
#  crt.sh query
# ──────────────────────────────────────────────────────────────────────

def _query_crtsh(domain: str, *, timeout: float) -> tuple[int, List[dict]]:
    """Return (status, rows). Each row is a crt.sh certificate record."""
    q = urllib.parse.quote(f"%.{domain}")
    url = f"https://crt.sh/?q={q}&output=json"
    status, body = _get_json(url, timeout=timeout)
    if status == 200 and isinstance(body, list):
        return status, body
    return status, []


def _split_names(name_value: str) -> List[str]:
    """crt.sh packs multiple SANs newline-separated in `name_value`."""
    out: List[str] = []
    for raw in (name_value or "").split("\n"):
        host = raw.strip().lower().lstrip("*.").rstrip(".")
        if host and _looks_like_host(host):
            out.append(host)
    return out


# ──────────────────────────────────────────────────────────────────────
#  Transforms
# ──────────────────────────────────────────────────────────────────────

def _run_subdomains(event_type: str, data: str, opts: dict) -> List[dict]:
    domain = _normalise_domain(data)
    timeout = float(opts.get("timeout", _DEFAULT_TIMEOUT))
    if not _looks_like_host(domain):
        return [_raw(f"crtsh_subdomains: {data!r} is not a domain", event_type,
                    "crtsh_subdomains", 30, {"input": data})]

    status, rows = _query_crtsh(domain, timeout=timeout)
    if not rows:
        return [_raw(f"crt.sh: no certificates / status={status} for {domain}",
                     event_type, "crtsh_subdomains", 40,
                     {"domain": domain, "status": status})]

    # Collect distinct hosts, tracking how many certs mentioned each so the
    # inspector can show the most-frequently-seen subdomains first.
    seen: Dict[str, int] = {}
    for row in rows:
        for host in _split_names(row.get("name_value", "")):
            if host == domain or host.endswith("." + domain):
                seen[host] = seen.get(host, 0) + 1

    events: List[dict] = []
    for host, count in sorted(seen.items(), key=lambda kv: (-kv[1], kv[0])):
        # Don't re-emit the apex as its own subdomain node.
        if host == domain:
            continue
        events.append({
            "event_type": "INTERNET_NAME",
            "data": host,
            "module": "crtsh_subdomains",
            "confidence": 90,
            "source_event_type": event_type,
            "extra": {"parent_domain": domain, "cert_count": count,
                      "source": "crt.sh"},
        })

    events.append(_raw(
        f"crt.sh {domain}: {len(seen)} distinct hosts across {len(rows)} certs",
        event_type, "crtsh_subdomains", 80,
        {"domain": domain, "host_count": len(seen), "cert_count": len(rows),
         "hosts": sorted(seen)[:500]}))
    return events


def _run_certs(event_type: str, data: str, opts: dict) -> List[dict]:
    domain = _normalise_domain(data)
    timeout = float(opts.get("timeout", _DEFAULT_TIMEOUT))
    if not _looks_like_host(domain):
        return [_raw(f"crtsh_certs: {data!r} is not a domain", event_type,
                    "crtsh_certs", 30, {"input": data})]

    status, rows = _query_crtsh(domain, timeout=timeout)
    if not rows:
        return [_raw(f"crt.sh: no certificates / status={status} for {domain}",
                     event_type, "crtsh_certs", 40,
                     {"domain": domain, "status": status})]

    events: List[dict] = []
    seen_certs: set = set()
    seen_issuers: Dict[str, int] = {}
    for row in rows:
        cn = (row.get("common_name") or "").strip().lower()
        not_before = row.get("not_before")
        not_after = row.get("not_after")
        serial = row.get("serial_number")
        issuer = (row.get("issuer_name") or "").strip()
        crt_id = row.get("id")

        key = (cn, serial, not_after)
        if cn and key not in seen_certs:
            seen_certs.add(key)
            events.append({
                "event_type": "SSL_CERTIFICATE_ISSUED",
                "data": f"{cn} (exp {not_after})",
                "module": "crtsh_certs",
                "confidence": 90,
                "source_event_type": event_type,
                "extra": {
                    "common_name": cn,
                    "issuer": issuer,
                    "not_before": not_before,
                    "not_after": not_after,
                    "serial_number": serial,
                    "url": f"https://crt.sh/?id={crt_id}" if crt_id else None,
                    "source": "crt.sh",
                },
            })

        if issuer:
            seen_issuers[issuer] = seen_issuers.get(issuer, 0) + 1

    # One issuer entity per distinct issuing CA — extract the org (O=...).
    for issuer, count in sorted(seen_issuers.items(), key=lambda kv: -kv[1]):
        org = _extract_org(issuer) or issuer
        events.append({
            "event_type": "SSL_CERTIFICATE_ISSUER",
            "data": org,
            "module": "crtsh_certs",
            "confidence": 85,
            "source_event_type": event_type,
            "extra": {"issuer_dn": issuer, "cert_count": count,
                      "source": "crt.sh"},
        })

    events.append(_raw(
        f"crt.sh {domain}: {len(seen_certs)} certs from {len(seen_issuers)} CAs",
        event_type, "crtsh_certs", 80,
        {"domain": domain, "cert_count": len(seen_certs),
         "issuer_count": len(seen_issuers)}))
    return events


def _extract_org(dn: str) -> Optional[str]:
    """Pull the O= field out of an X.509 issuer distinguished name."""
    m = re.search(r"\bO\s*=\s*([^,]+)", dn or "")
    return m.group(1).strip() if m else None


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
        "id": "crtsh_subdomains",
        "name": "CT logs → subdomains (crt.sh)",
        "description": (
            "Query Certificate Transparency logs (crt.sh) for every host "
            "ever named on a certificate under this domain. Surfaces "
            "subdomains that DNS brute-force misses. Passive, no API key."
        ),
        "categories": ["Infrastructure", "Domain"],
        "group": [],
        "consumes": ["DOMAIN_NAME", "INTERNET_NAME", "AFFILIATE_DOMAIN_NAME"],
        "produces": ["INTERNET_NAME", "RAW_RIR_DATA"],
    },
    {
        "id": "crtsh_certs",
        "name": "CT logs → certificates + issuers (crt.sh)",
        "description": (
            "List the TLS certificates issued for this domain along with "
            "the issuing CA / organisation. Useful for spotting wildcard "
            "certs, expired certs and the providers an org trusts."
        ),
        "categories": ["Infrastructure", "Certificates"],
        "group": [],
        "consumes": ["DOMAIN_NAME", "INTERNET_NAME"],
        "produces": ["SSL_CERTIFICATE_ISSUED", "SSL_CERTIFICATE_ISSUER",
                     "RAW_RIR_DATA"],
    },
]


def _dispatch_run(module: str, event_type: str, data: str, opts: dict) -> List[dict]:
    if module == "crtsh_subdomains":
        return _run_subdomains(event_type, data, opts)
    if module == "crtsh_certs":
        return _run_certs(event_type, data, opts)
    raise ValueError(f"unknown module: {module}")


# ──────────────────────────────────────────────────────────────────────
#  Subcommand entry points (identical contract across sidecars)
# ──────────────────────────────────────────────────────────────────────

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
    parser = argparse.ArgumentParser(prog="maltrace-cert-transparency-adapter")
    sub = parser.add_subparsers(dest="cmd", required=True)
    sub.add_parser("list-transforms", help="List cert-transparency transforms.")
    run = sub.add_parser("run", help="Query crt.sh for a domain.")
    run.add_argument("--module", required=True)
    run.add_argument("--event-type", required=True)
    run.add_argument("--data", required=True)
    run.add_argument("--opts", default=None, help='JSON: {"timeout": 30}')
    args = parser.parse_args(argv)
    if args.cmd == "list-transforms":
        return cmd_list_transforms(args)
    if args.cmd == "run":
        return cmd_run(args)
    parser.print_help(sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main())
