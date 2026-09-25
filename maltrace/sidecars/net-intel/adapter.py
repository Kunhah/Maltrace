"""
maltrace ↔ net-intel adapter.

Enrich an IP address with geolocation, hosting provider, ASN and reverse-DNS
hostnames using free, key-less sources:

  * ip-api.com  — geolocation + ISP/org + ASN. Free tier: 45 req/min, HTTP
                  only on the free endpoint. We request a compact field set.
  * system DNS  — reverse-DNS (PTR) lookups via socket.gethostbyaddr.

The geolocation transform emits a PHYSICAL_COORDINATES entity whose label is
`"LAT,LNG"` and whose `extra` carries `{lat, lng}` — exactly the shape the
maltrace Map view extracts, so a geolocated IP drops a pin with no extra work.

Transforms:

    ip_geolocate   IpAddress → GEOINFO + COUNTRY_NAME + PHYSICAL_COORDINATES
                               + PROVIDER_HOSTING + BGP_AS_OWNER
    ip_reverse_dns IpAddress → INTERNET_NAME (PTR hostnames)   [active]

One-shot JSON protocol shared with the other sidecars.
"""

from __future__ import annotations

import argparse
import ipaddress
import json
import socket
import sys
import traceback
import urllib.error
import urllib.request
from typing import Any, List, Optional


# ──────────────────────────────────────────────────────────────────────
#  HTTP helper
# ──────────────────────────────────────────────────────────────────────

_HEADERS = {
    "User-Agent": "maltrace-net-intel/0.1 (+https://github.com/)",
    "Accept": "application/json",
}
_DEFAULT_TIMEOUT = 12


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


def _normalise_ip(label: str) -> Optional[str]:
    """Extract a bare IP from the label; return None if not an IP."""
    s = (label or "").strip()
    # Tolerate "1.2.3.4 (something)" or "ip: 1.2.3.4" style labels.
    for tok in s.replace("(", " ").replace(")", " ").split():
        try:
            return str(ipaddress.ip_address(tok))
        except ValueError:
            continue
    try:
        return str(ipaddress.ip_address(s))
    except ValueError:
        return None


# ──────────────────────────────────────────────────────────────────────
#  Geolocation (ip-api.com)
# ──────────────────────────────────────────────────────────────────────

# Bitmask field selection keeps the response tight and documents intent.
_IPAPI_FIELDS = (
    "status,message,country,countryCode,region,regionName,city,zip,"
    "lat,lon,timezone,isp,org,as,asname,reverse,query"
)


def _run_geolocate(event_type: str, data: str, opts: dict) -> List[dict]:
    ip = _normalise_ip(data)
    timeout = float(opts.get("timeout", _DEFAULT_TIMEOUT))
    if ip is None:
        return [_raw(f"ip_geolocate: {data!r} is not an IP address",
                     event_type, "ip_geolocate", 30, {"input": data})]

    if ipaddress.ip_address(ip).is_private:
        return [_raw(f"ip_geolocate: {ip} is private / non-routable",
                     event_type, "ip_geolocate", 40, {"ip": ip})]

    status, body = _get_json(
        f"http://ip-api.com/json/{ip}?fields={_IPAPI_FIELDS}", timeout=timeout)
    if status != 200 or not isinstance(body, dict) or body.get("status") != "success":
        msg = body.get("message") if isinstance(body, dict) else body
        return [_raw(f"ip-api.com: {msg or status} for {ip}",
                     event_type, "ip_geolocate", 40,
                     {"ip": ip, "status": status, "body": body})]

    country = body.get("country")
    cc = body.get("countryCode")
    city = body.get("city")
    region = body.get("regionName")
    lat = body.get("lat")
    lon = body.get("lon")
    isp = body.get("isp")
    org = body.get("org")
    as_field = body.get("as") or ""   # e.g. "AS15169 Google LLC"
    asname = body.get("asname")

    events: List[dict] = []

    # GEOINFO — a human-readable place string.
    place = ", ".join(p for p in (city, region, country) if p)
    if place:
        events.append({
            "event_type": "GEOINFO",
            "data": place,
            "module": "ip_geolocate",
            "confidence": 80,
            "source_event_type": event_type,
            "extra": {"ip": ip, "city": city, "region": region,
                      "country": country, "timezone": body.get("timezone"),
                      "zip": body.get("zip"), "source": "ip-api.com"},
        })

    if country:
        events.append({
            "event_type": "COUNTRY_NAME",
            "data": country,
            "module": "ip_geolocate",
            "confidence": 85,
            "source_event_type": event_type,
            "extra": {"ip": ip, "country_code": cc, "source": "ip-api.com"},
        })

    # PHYSICAL_COORDINATES — "LAT,LNG" label + extra.{lat,lng} → Map pin.
    if isinstance(lat, (int, float)) and isinstance(lon, (int, float)):
        events.append({
            "event_type": "PHYSICAL_COORDINATES",
            "data": f"{lat},{lon}",
            "module": "ip_geolocate",
            "confidence": 75,
            "source_event_type": event_type,
            "extra": {"lat": lat, "lng": lon, "latitude": lat, "longitude": lon,
                      "ip": ip, "label": place or ip, "source": "ip-api.com"},
        })

    # PROVIDER_HOSTING — the ISP / hosting org.
    host_org = org or isp
    if host_org:
        events.append({
            "event_type": "PROVIDER_HOSTING",
            "data": host_org,
            "module": "ip_geolocate",
            "confidence": 75,
            "source_event_type": event_type,
            "extra": {"ip": ip, "isp": isp, "org": org, "source": "ip-api.com"},
        })

    # BGP_AS_OWNER — the autonomous system.
    if as_field:
        events.append({
            "event_type": "BGP_AS_OWNER",
            "data": as_field,
            "module": "ip_geolocate",
            "confidence": 80,
            "source_event_type": event_type,
            "extra": {"ip": ip, "as": as_field, "asname": asname,
                      "source": "ip-api.com"},
        })

    events.append(_raw(
        f"ip-api.com {ip}: {place or '?'} · {host_org or '?'} · {as_field or '?'}",
        event_type, "ip_geolocate", 70, {"ip": ip, "body": body}))
    return events


# ──────────────────────────────────────────────────────────────────────
#  Reverse DNS (PTR) — active: hits the resolver / authoritative servers.
# ──────────────────────────────────────────────────────────────────────

def _run_reverse_dns(event_type: str, data: str, opts: dict) -> List[dict]:
    ip = _normalise_ip(data)
    timeout = float(opts.get("timeout", 8))
    if ip is None:
        return [_raw(f"ip_reverse_dns: {data!r} is not an IP address",
                     event_type, "ip_reverse_dns", 30, {"input": data})]

    socket.setdefaulttimeout(timeout)
    try:
        primary, aliases, _addrs = socket.gethostbyaddr(ip)
    except (socket.herror, socket.gaierror, OSError) as e:
        return [_raw(f"ip_reverse_dns: no PTR for {ip} ({e})",
                     event_type, "ip_reverse_dns", 40, {"ip": ip})]

    hosts = []
    for h in [primary, *aliases]:
        h = (h or "").strip().rstrip(".").lower()
        if h and h not in hosts:
            hosts.append(h)

    events: List[dict] = []
    for h in hosts:
        events.append({
            "event_type": "INTERNET_NAME",
            "data": h,
            "module": "ip_reverse_dns",
            "confidence": 85,
            "source_event_type": event_type,
            "extra": {"ip": ip, "record": "PTR", "source": "system-resolver"},
        })
    events.append(_raw(f"reverse DNS {ip}: {len(hosts)} hostname(s)",
                       event_type, "ip_reverse_dns", 70,
                       {"ip": ip, "hosts": hosts}))
    return events


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
        "id": "ip_geolocate",
        "name": "IP geolocation + ASN (ip-api.com)",
        "description": (
            "Geolocate an IP via ip-api.com and emit a Geolocation pin "
            "(lat/lng — shows on the Map view), the country, the hosting "
            "ISP/org and the autonomous system. Passive, no API key."
        ),
        "categories": ["Infrastructure", "Geolocation"],
        "group": [],
        "consumes": ["IP_ADDRESS", "IPV6_ADDRESS", "AFFILIATE_IPADDR"],
        "produces": ["GEOINFO", "COUNTRY_NAME", "PHYSICAL_COORDINATES",
                     "PROVIDER_HOSTING", "BGP_AS_OWNER", "RAW_RIR_DATA"],
    },
    {
        "id": "ip_reverse_dns",
        "name": "IP → reverse DNS hostnames (PTR)",
        "description": (
            "Resolve the PTR record(s) for an IP via the system resolver, "
            "emitting the hostname(s) as Domain entities. Active — the "
            "lookup hits DNS infrastructure."
        ),
        "categories": ["Infrastructure", "DNS"],
        "group": [],
        "consumes": ["IP_ADDRESS", "IPV6_ADDRESS", "AFFILIATE_IPADDR"],
        "produces": ["INTERNET_NAME", "RAW_RIR_DATA"],
    },
]


def _dispatch_run(module: str, event_type: str, data: str, opts: dict) -> List[dict]:
    if module == "ip_geolocate":
        return _run_geolocate(event_type, data, opts)
    if module == "ip_reverse_dns":
        return _run_reverse_dns(event_type, data, opts)
    raise ValueError(f"unknown module: {module}")


# ──────────────────────────────────────────────────────────────────────
#  Subcommand entry points
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
    parser = argparse.ArgumentParser(prog="maltrace-net-intel-adapter")
    sub = parser.add_subparsers(dest="cmd", required=True)
    sub.add_parser("list-transforms", help="List net-intel transforms.")
    run = sub.add_parser("run", help="Enrich an IP address.")
    run.add_argument("--module", required=True)
    run.add_argument("--event-type", required=True)
    run.add_argument("--data", required=True)
    run.add_argument("--opts", default=None, help='JSON: {"timeout": 12}')
    args = parser.parse_args(argv)
    if args.cmd == "list-transforms":
        return cmd_list_transforms(args)
    if args.cmd == "run":
        return cmd_run(args)
    parser.print_help(sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main())
