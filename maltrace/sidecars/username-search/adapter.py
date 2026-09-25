"""
maltrace ↔ username-search adapter.

Given a USERNAME entity, probe a curated list of ~50 social, dev and
community sites and emit one URL event per site where a profile is found.

Sites are detected with **stable** rules — either an HTTP status code
distinction (200 = exists, 404 = missing) or an explicit body marker for
the few sites that always return 200. Anything flakier than that gets
dropped, because false positives erode trust in the canvas faster than
they help.

One-shot JSON protocol shared with other sidecars:

    python adapter.py list-transforms
    python adapter.py run --module username_lookup --event-type USERNAME --data <HANDLE>

Stdout: JSON. Stderr: diagnostics. Pure stdlib (urllib + thread pool).
"""

from __future__ import annotations

import argparse
import json
import re
import sys
import traceback
import urllib.error
import urllib.request
from concurrent.futures import ThreadPoolExecutor, as_completed
from dataclasses import dataclass, field
from typing import List, Optional


# ──────────────────────────────────────────────────────────────────────
#  Detection rule data model
# ──────────────────────────────────────────────────────────────────────

@dataclass
class Site:
    """One social-site probe rule.

    `url`        — URL template; `{u}` is replaced with the username.
    `category`   — coarse grouping (`Social`, `Dev`, `Gaming`, …).
    `present_status` / `absent_status` — HTTP status codes that
                     unambiguously mean *exists* or *missing*. Either may
                     be `None`; we require at least one to differ for the
                     site to be useful.
    `absent_body`   — substring whose presence in the body means *missing*
                     (used for sites that always return 200).
    `present_body`  — substring whose presence means *exists* (rarely
                     needed; useful when both states return 200).
    `method`        — HTTP method (default GET).
    `read_limit`    — cap on body bytes read for marker checks. Keeps
                     enumeration cheap; full pages are unnecessary.
    """
    name: str
    url: str
    category: str = "Social"
    present_status: Optional[int] = 200
    absent_status: Optional[int] = 404
    absent_body: Optional[str] = None
    present_body: Optional[str] = None
    method: str = "GET"
    read_limit: int = 65536
    # Per-site allow/forbid character set (default: standard handle chars).
    valid_chars: re.Pattern = field(
        default_factory=lambda: re.compile(r"^[A-Za-z0-9_.\-]{1,64}$")
    )


# ──────────────────────────────────────────────────────────────────────
#  Curated site catalogue
# ──────────────────────────────────────────────────────────────────────

SITES: List[Site] = [
    # ─── Dev / Tech ──────────────────────────────────────────────
    #
    # Every site here returns a clean 200/404 distinction from a vanilla
    # `urllib` GET, OR has a body marker reliably within the first 64 KB.
    # Sites whose front page is a JS-only SPA that always returns 200
    # with an identical shell (PyPI, Hashnode, Replit, eBay, PayPal-me,
    # Twitch, Behance, Producthunt, Instagram, TikTok, Pinterest, Spotify,
    # AniList, 500px, Internet Archive, Reddit-with-CF-block, Wikipedia
    # user-pages-always-200, etc.) are deliberately omitted — false
    # positives erode trust faster than missing sites help.
    Site("GitHub",        "https://github.com/{u}",            category="Dev"),
    Site("GitLab",        "https://gitlab.com/{u}",            category="Dev"),
    Site("Bitbucket",     "https://bitbucket.org/{u}/",        category="Dev"),
    Site("Codeberg",      "https://codeberg.org/{u}",          category="Dev"),
    Site("CodePen",       "https://codepen.io/{u}",            category="Dev"),
    Site("Dev.to",        "https://dev.to/{u}",                category="Dev"),
    Site("RubyGems",      "https://rubygems.org/profiles/{u}", category="Dev"),
    Site("DockerHub",     "https://hub.docker.com/u/{u}",      category="Dev"),
    Site("HackerOne",     "https://hackerone.com/{u}",         category="Dev"),
    Site("Keybase",       "https://keybase.io/{u}",            category="Dev"),

    # ─── Social ──────────────────────────────────────────────────
    Site("About.me",      "https://about.me/{u}",              category="Social"),
    Site("Linktree",      "https://linktr.ee/{u}",             category="Social"),
    Site("Patreon",       "https://www.patreon.com/{u}",       category="Social"),
    Site("BuyMeACoffee",  "https://www.buymeacoffee.com/{u}",  category="Social"),
    Site("Mastodon.social", "https://mastodon.social/@{u}",    category="Social"),
    Site("Fosstodon",     "https://fosstodon.org/@{u}",        category="Social"),
    Site("Lemmy.world",   "https://lemmy.world/u/{u}",         category="Social"),
    Site("Wattpad",       "https://www.wattpad.com/user/{u}",  category="Social"),

    # ─── Media / Creators ───────────────────────────────────────
    Site("YouTube (@)",   "https://www.youtube.com/@{u}",      category="Media"),
    Site("SoundCloud",    "https://soundcloud.com/{u}",        category="Media"),
    Site("Last.fm",       "https://www.last.fm/user/{u}",      category="Media"),
    Site("Mixcloud",      "https://www.mixcloud.com/{u}/",     category="Media"),
    Site("Vimeo",         "https://vimeo.com/{u}",             category="Media"),
    Site("Dribbble",      "https://dribbble.com/{u}",          category="Media"),
    Site("Flickr",        "https://www.flickr.com/people/{u}/", category="Media"),
    Site("DeviantArt",    "https://www.deviantart.com/{u}",    category="Media"),

    # ─── Commerce ────────────────────────────────────────────────
    Site("Etsy",          "https://www.etsy.com/people/{u}",   category="Commerce"),
    Site("Gumroad",       "https://gumroad.com/{u}",           category="Commerce"),
    Site("Fiverr",        "https://www.fiverr.com/{u}",        category="Commerce"),
    Site("Cash App",      "https://cash.app/${u}",             category="Commerce"),
    Site("Venmo",         "https://account.venmo.com/u/{u}",   category="Commerce"),

    # ─── Gaming ──────────────────────────────────────────────────
    Site(
        "Steam",
        "https://steamcommunity.com/id/{u}",
        category="Gaming",
        absent_status=None,
        # Marker appears around byte 25 K — well within `read_limit`.
        absent_body="The specified profile could not be found",
    ),
    Site("Chess.com",     "https://www.chess.com/member/{u}",     category="Gaming"),
    Site("Lichess",       "https://lichess.org/@/{u}",            category="Gaming"),
    Site("MyAnimeList",   "https://myanimelist.net/profile/{u}",  category="Gaming"),
    Site("BoardGameGeek", "https://boardgamegeek.com/user/{u}",   category="Gaming"),
    Site("Itch.io",       "https://itch.io/profile/{u}",          category="Gaming"),

    # ─── Reference / Misc ────────────────────────────────────────
    Site("TradingView",   "https://www.tradingview.com/u/{u}/",   category="Reference"),
]


# ──────────────────────────────────────────────────────────────────────
#  HTTP probe
# ──────────────────────────────────────────────────────────────────────

_UA = (
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) "
    "Chrome/124.0 Safari/537.36 maltrace-username/0.1"
)
_HEADERS = {
    "User-Agent": _UA,
    "Accept": "text/html,application/xhtml+xml",
    "Accept-Language": "en-US,en;q=0.7",
}


def _probe(site: Site, username: str, *, timeout: float) -> Optional[dict]:
    """Return a match dict if the profile is found on `site`, else None.

    Returns None on transient errors as well — false positives are worse
    than missing a site.
    """
    if not site.valid_chars.match(username):
        return None

    url = site.url.format(u=username)
    req = urllib.request.Request(url, headers=_HEADERS, method=site.method)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            status = resp.status
            body = (
                resp.read(site.read_limit).decode("utf-8", errors="replace")
                if (site.absent_body or site.present_body)
                else ""
            )
    except urllib.error.HTTPError as e:
        status = e.code
        body = ""
        if site.absent_body or site.present_body:
            try:
                body = e.read(site.read_limit).decode("utf-8", errors="replace")
            except Exception:  # noqa: BLE001
                body = ""
    except (urllib.error.URLError, TimeoutError, ConnectionError):
        return None
    except Exception:  # noqa: BLE001
        return None

    # 1) Definite-absence via body marker.
    if site.absent_body and site.absent_body in body:
        return None
    # 2) Definite-absence via status.
    if site.absent_status is not None and status == site.absent_status:
        return None
    # 3) Definite-presence via status.
    is_present = site.present_status is not None and status == site.present_status
    # 4) Or via body marker.
    if site.present_body and site.present_body in body:
        is_present = True
    if not is_present:
        return None

    return {
        "site": site.name,
        "category": site.category,
        "url": url,
        "status": status,
    }


# ──────────────────────────────────────────────────────────────────────
#  Transform manifest
# ──────────────────────────────────────────────────────────────────────

TRANSFORMS = [
    {
        "id": "username_lookup",
        "name": "Username enumeration (≈40 sites)",
        "description": (
            "Probe a curated list of social, dev, media and gaming sites "
            "to find which ones have a public profile under the given "
            "username. Uses HTTP status and stable body markers — no API "
            "keys required. Returns one URL per hit."
        ),
        "categories": ["People", "Social"],
        "group": [],
        "consumes": ["USERNAME", "ACCOUNT_EXTERNAL_OWNED", "SOCIAL_MEDIA"],
        "produces": ["URL", "ACCOUNT_EXTERNAL_OWNED", "SOCIAL_MEDIA"],
    },
    {
        "id": "username_lookup_dev_only",
        "name": "Username enumeration — dev sites only",
        "description": (
            "Faster variant of username_lookup that only probes "
            "developer-focused sites (GitHub, GitLab, npm, PyPI, "
            "DockerHub, HackerOne, Keybase…)."
        ),
        "categories": ["People", "Dev"],
        "group": [],
        "consumes": ["USERNAME", "ACCOUNT_EXTERNAL_OWNED"],
        "produces": ["URL", "ACCOUNT_EXTERNAL_OWNED"],
    },
]


# ──────────────────────────────────────────────────────────────────────
#  Username normalisation
# ──────────────────────────────────────────────────────────────────────

def _normalise(label: str) -> str:
    """Pull a bare handle out of the input label.

    Accepts `handle`, `@handle`, `handle@platform`, or a URL with a
    final path segment. We only return the handle string — no platform.
    """
    s = label.strip()
    # `handle@platform` (our face-search format) → handle.
    if "@" in s and not s.startswith("@") and "/" not in s and " " not in s:
        s = s.split("@", 1)[0]
    s = s.lstrip("@")
    # URL → last non-empty path segment.
    if "://" in s:
        s = s.split("?", 1)[0].rstrip("/").split("/")[-1]
    return s


# ──────────────────────────────────────────────────────────────────────
#  Dispatch
# ──────────────────────────────────────────────────────────────────────

def _run_lookup(event_type: str, data: str, *, dev_only: bool, opts: dict) -> List[dict]:
    username = _normalise(data)
    if not username or not re.match(r"^[A-Za-z0-9_.\-]{1,64}$", username):
        return [{
            "event_type": "RAW_RIR_DATA",
            "data": f"username_lookup: invalid handle {data!r}",
            "module": "username_lookup",
            "confidence": 30,
            "source_event_type": event_type,
            "extra": {"input": data},
        }]

    sites = [s for s in SITES if not dev_only or s.category == "Dev"]
    timeout = float(opts.get("timeout", 10))
    workers = max(1, int(opts.get("workers", 16)))

    hits: List[dict] = []
    with ThreadPoolExecutor(max_workers=workers) as pool:
        futs = {pool.submit(_probe, s, username, timeout=timeout): s for s in sites}
        for fut in as_completed(futs):
            try:
                res = fut.result()
            except Exception:  # noqa: BLE001
                res = None
            if res:
                hits.append(res)

    events: List[dict] = []
    for h in hits:
        events.append({
            "event_type": "URL",
            "data": h["url"],
            "module": "username_lookup_dev_only" if dev_only else "username_lookup",
            "confidence": 85,
            "source_event_type": event_type,
            "extra": {
                "site": h["site"],
                "category": h["category"],
                "status": h["status"],
                "username": username,
            },
        })
        events.append({
            "event_type": "ACCOUNT_EXTERNAL_OWNED",
            "data": f"{username}@{h['site']}",
            "module": "username_lookup_dev_only" if dev_only else "username_lookup",
            "confidence": 85,
            "source_event_type": event_type,
            "extra": {"site": h["site"], "url": h["url"], "category": h["category"]},
        })

    # Summary event makes the run visible even when nothing was found.
    events.append({
        "event_type": "RAW_RIR_DATA",
        "data": (
            f"username_lookup: {len(hits)} hit(s) for {username} "
            f"across {len(sites)} sites"
        ),
        "module": "username_lookup_dev_only" if dev_only else "username_lookup",
        "confidence": 70,
        "source_event_type": event_type,
        "extra": {
            "username": username,
            "sites_probed": len(sites),
            "hit_count": len(hits),
            "hits": hits,
        },
    })
    return events


def _dispatch_run(module: str, event_type: str, data: str, opts: dict) -> List[dict]:
    if module == "username_lookup":
        return _run_lookup(event_type, data, dev_only=False, opts=opts)
    if module == "username_lookup_dev_only":
        return _run_lookup(event_type, data, dev_only=True, opts=opts)
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
            json.dump({
                "ok": False,
                "error": f"bad --opts: {exc}",
                "events": [],
            }, sys.stdout)
            sys.stdout.write("\n")
            return 2

    try:
        events = _dispatch_run(args.module, args.event_type, args.data, opts)
    except Exception as exc:  # noqa: BLE001
        json.dump({
            "ok": False,
            "error": f"{type(exc).__name__}: {exc}",
            "traceback": traceback.format_exc(),
            "events": [],
        }, sys.stdout)
        sys.stdout.write("\n")
        return 3

    json.dump({"ok": True, "events": events}, sys.stdout)
    sys.stdout.write("\n")
    return 0


def main(argv: Optional[List[str]] = None) -> int:
    parser = argparse.ArgumentParser(prog="maltrace-username-search-adapter")
    sub = parser.add_subparsers(dest="cmd", required=True)
    sub.add_parser("list-transforms", help="List username-search transforms.")
    run = sub.add_parser("run", help="Probe sites for a username.")
    run.add_argument("--module", required=True)
    run.add_argument("--event-type", required=True)
    run.add_argument("--data", required=True)
    run.add_argument("--opts", default=None,
                     help='JSON: {"timeout": 10, "workers": 16}')
    args = parser.parse_args(argv)
    if args.cmd == "list-transforms":
        return cmd_list_transforms(args)
    if args.cmd == "run":
        return cmd_run(args)
    parser.print_help(sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main())
