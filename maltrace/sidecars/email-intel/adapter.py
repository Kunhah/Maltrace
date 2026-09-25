"""
maltrace ↔ email-intel adapter.

Enrich an email address with key-less OSINT:

  * USERNAME    the local part (before @) → pivot into username enumeration.
  * Gravatar    md5(lowercased email) → public profile JSON + avatar URL.
                Globally-recognised profile that many people forget exists.
  * provider    mapped from the domain (gmail/outlook/proton/…); plus a
                disposable-address flag from a curated throwaway-domain list.
  * pivots      curated openable reputation/breach lookup URLs (HIBP,
                EmailRep, Epieos, Hunter, IntelX) the analyst clicks through.

Transforms:
    email_enrich    EmailAddress → USERNAME + Gravatar + provider + pivots

One-shot JSON protocol shared with the other sidecars.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
import traceback
import urllib.error
import urllib.parse
import urllib.request
from typing import Any, List, Optional


_HEADERS = {
    "User-Agent": "maltrace-email-intel/0.1 (+https://github.com/)",
    "Accept": "application/json",
}
_DEFAULT_TIMEOUT = 12

_EMAIL_RE = re.compile(r"^[^@\s]+@[^@\s]+\.[^@\s]+$")

# Well-known free / consumer mail providers → friendly name.
_PROVIDERS = {
    "gmail.com": "Google Gmail",
    "googlemail.com": "Google Gmail",
    "outlook.com": "Microsoft Outlook",
    "hotmail.com": "Microsoft Outlook (Hotmail)",
    "live.com": "Microsoft Outlook (Live)",
    "msn.com": "Microsoft Outlook (MSN)",
    "yahoo.com": "Yahoo Mail",
    "ymail.com": "Yahoo Mail",
    "proton.me": "Proton Mail",
    "protonmail.com": "Proton Mail",
    "pm.me": "Proton Mail",
    "icloud.com": "Apple iCloud",
    "me.com": "Apple iCloud",
    "mac.com": "Apple iCloud",
    "aol.com": "AOL Mail",
    "gmx.com": "GMX",
    "gmx.net": "GMX",
    "zoho.com": "Zoho Mail",
    "yandex.ru": "Yandex Mail",
    "yandex.com": "Yandex Mail",
    "mail.com": "mail.com",
    "tutanota.com": "Tutanota",
    "tuta.io": "Tutanota",
}

# A compact curated disposable / throwaway domain set. Not exhaustive — just
# the high-frequency offenders — so a hit is meaningful and a miss is honest.
_DISPOSABLE = {
    "mailinator.com", "guerrillamail.com", "10minutemail.com", "tempmail.com",
    "temp-mail.org", "throwawaymail.com", "yopmail.com", "getnada.com",
    "trashmail.com", "sharklasers.com", "guerrillamailblock.com", "dispostable.com",
    "fakeinbox.com", "maildrop.cc", "mailnesia.com", "mohmal.com", "moakt.com",
    "tempr.email", "33mail.com", "spamgourmet.com", "mintemail.com",
}


def _get(url: str, *, timeout: float) -> tuple[int, bytes]:
    req = urllib.request.Request(url, headers=_HEADERS)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            return resp.status, resp.read()
    except urllib.error.HTTPError as e:
        return e.code, (e.read() if hasattr(e, "read") else b"")
    except (urllib.error.URLError, TimeoutError, ConnectionError):
        return 0, b""


def _normalise_email(label: str) -> Optional[str]:
    s = (label or "").strip().strip("<>").lower()
    # Tolerate "Name <a@b.com>" and "mailto:a@b.com".
    if s.startswith("mailto:"):
        s = s[len("mailto:"):]
    m = re.search(r"[^@\s<>]+@[^@\s<>]+\.[^@\s<>]+", s)
    if m:
        s = m.group(0)
    return s if _EMAIL_RE.match(s) else None


# ──────────────────────────────────────────────────────────────────────
#  Gravatar
# ──────────────────────────────────────────────────────────────────────

def _gravatar(email: str, *, timeout: float) -> Optional[dict]:
    h = hashlib.md5(email.encode("utf-8")).hexdigest()
    profile_url = f"https://www.gravatar.com/{h}.json"
    status, raw = _get(profile_url, timeout=timeout)
    avatar = f"https://www.gravatar.com/avatar/{h}"
    if status != 200 or not raw:
        return {"hash": h, "exists": False, "avatar": avatar}
    try:
        body = json.loads(raw.decode("utf-8", errors="replace"))
    except json.JSONDecodeError:
        return {"hash": h, "exists": False, "avatar": avatar}
    entries = body.get("entry") if isinstance(body, dict) else None
    entry = entries[0] if isinstance(entries, list) and entries else {}
    return {
        "hash": h,
        "exists": True,
        "avatar": avatar,
        "profile_url": entry.get("profileUrl") or f"https://gravatar.com/{h}",
        "display_name": entry.get("displayName"),
        "preferred_username": entry.get("preferredUsername"),
        "name": entry.get("name"),
        "location": entry.get("currentLocation"),
        "accounts": entry.get("accounts") or [],
        "urls": entry.get("urls") or [],
    }


# ──────────────────────────────────────────────────────────────────────
#  Pivot links (open in browser)
# ──────────────────────────────────────────────────────────────────────

def _pivot_links(email: str) -> List[dict]:
    e = urllib.parse.quote(email)
    return [
        {"title": "Have I Been Pwned", "url": f"https://haveibeenpwned.com/account/{e}",
         "description": "Breach exposure for this address (web UI)."},
        {"title": "EmailRep.io", "url": f"https://emailrep.io/{e}",
         "description": "Reputation / risk profile for the address."},
        {"title": "Epieos", "url": f"https://epieos.com/?q={e}",
         "description": "Email → linked Google/other account discovery."},
        {"title": "Hunter.io verify", "url": f"https://hunter.io/verify/{e}",
         "description": "Deliverability + format verification."},
        {"title": "Intelligence X", "url": f"https://intelx.io/?s={e}",
         "description": "Leaks / pastes / darkweb mentions index."},
    ]


# ──────────────────────────────────────────────────────────────────────
#  Main transform
# ──────────────────────────────────────────────────────────────────────

def _run_enrich(event_type: str, data: str, opts: dict) -> List[dict]:
    email = _normalise_email(data)
    timeout = float(opts.get("timeout", _DEFAULT_TIMEOUT))
    if email is None:
        return [_raw(f"email_enrich: {data!r} is not an email address",
                     event_type, "email_enrich", 30, {"input": data})]

    local, domain = email.split("@", 1)
    events: List[dict] = []

    # ── username pivot ───────────────────────────────────────────────
    if re.match(r"^[A-Za-z0-9_.\-]{1,64}$", local):
        events.append({
            "event_type": "USERNAME",
            "data": local,
            "module": "email_enrich",
            "confidence": 70,
            "source_event_type": event_type,
            "extra": {"derived_from": email, "source": "local-part"},
        })

    # ── provider / disposable ────────────────────────────────────────
    provider = _PROVIDERS.get(domain)
    disposable = domain in _DISPOSABLE
    if provider:
        events.append({
            "event_type": "PROVIDER_MAIL",
            "data": provider,
            "module": "email_enrich",
            "confidence": 85,
            "source_event_type": event_type,
            "extra": {"domain": domain, "email": email},
        })
    if disposable:
        events.append({
            "event_type": "EMAILADDR_DISPOSABLE",
            "data": email,
            "module": "email_enrich",
            "confidence": 90,
            "source_event_type": event_type,
            "extra": {"domain": domain, "reason": "known disposable provider"},
        })

    # ── Gravatar ─────────────────────────────────────────────────────
    grav = _gravatar(email, timeout=timeout)
    if grav and grav.get("exists"):
        prof = grav.get("profile_url")
        events.append({
            "event_type": "URL",
            "data": prof,
            "module": "email_enrich",
            "confidence": 88,
            "source_event_type": event_type,
            "extra": {"url": prof, "title": "Gravatar profile",
                      "avatar": grav.get("avatar"),
                      "display_name": grav.get("display_name"),
                      "preferred_username": grav.get("preferred_username"),
                      "location": grav.get("location"),
                      "linked_accounts": grav.get("accounts"),
                      "source": "gravatar"},
        })
        # A confirmed display name is a HUMAN_NAME pivot.
        if grav.get("display_name"):
            events.append({
                "event_type": "HUMAN_NAME",
                "data": grav["display_name"],
                "module": "email_enrich",
                "confidence": 70,
                "source_event_type": event_type,
                "extra": {"source": "gravatar", "email": email},
            })
        # Linked accounts on the Gravatar profile → username pivots.
        for acct in (grav.get("accounts") or [])[:20]:
            uname = acct.get("username") or acct.get("display")
            if uname:
                events.append({
                    "event_type": "ACCOUNT_EXTERNAL_OWNED",
                    "data": f"{uname}@{acct.get('shortname') or acct.get('domain') or 'web'}",
                    "module": "email_enrich",
                    "confidence": 75,
                    "source_event_type": event_type,
                    "extra": {"url": acct.get("url"),
                              "service": acct.get("shortname") or acct.get("domain"),
                              "source": "gravatar"},
                })

    # ── pivot links ──────────────────────────────────────────────────
    for link in _pivot_links(email):
        events.append({
            "event_type": "URL",
            "data": link["title"],
            "module": "email_enrich",
            "confidence": 60,
            "source_event_type": event_type,
            "extra": link,
        })

    # ── summary ──────────────────────────────────────────────────────
    events.append(_raw(
        f"email_enrich {email}: provider={provider or '?'}"
        f"{' · disposable' if disposable else ''}"
        f"{' · gravatar✓' if grav and grav.get('exists') else ''}",
        event_type, "email_enrich", 70,
        {"email": email, "local": local, "domain": domain,
         "provider": provider, "disposable": disposable,
         "gravatar": grav}))
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
        "id": "email_enrich",
        "name": "Email enrichment (Gravatar + provider + pivots)",
        "description": (
            "Enrich an email address: derive the username, look up its "
            "Gravatar profile (avatar, display name, linked accounts), "
            "identify the mail provider, flag disposable addresses, and "
            "emit curated breach/reputation lookup links. No API key."
        ),
        "categories": ["People", "Email", "Enrichment"],
        "group": [],
        "consumes": ["EMAILADDR", "EMAILADDR_GENERIC", "AFFILIATE_EMAILADDR",
                     "EMAILADDR_COMPROMISED"],
        "produces": ["USERNAME", "ACCOUNT_EXTERNAL_OWNED", "HUMAN_NAME",
                     "PROVIDER_MAIL", "EMAILADDR_DISPOSABLE", "URL",
                     "RAW_RIR_DATA"],
    },
]


def _dispatch_run(module: str, event_type: str, data: str, opts: dict) -> List[dict]:
    if module == "email_enrich":
        return _run_enrich(event_type, data, opts)
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
    parser = argparse.ArgumentParser(prog="maltrace-email-intel-adapter")
    sub = parser.add_subparsers(dest="cmd", required=True)
    sub.add_parser("list-transforms", help="List email-intel transforms.")
    run = sub.add_parser("run", help="Enrich an email address.")
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
