"""maltrace ↔ face-search adapter.

One-shot CLI bridge that mirrors the SpiderFoot adapter's protocol:

  python adapter.py list-transforms
  python adapter.py run --module <id> --event-type IMAGE --data <path-or-url> [--opts '{...}']

Stdout is JSON only. Stderr carries human-readable diagnostics.
Exit codes: 0 = ok, 2 = bad usage, 3 = transform raised.

Internally this is a thin shell over the bundled `facesearch` package:

  facesearch.SearchPipeline
    ├── InsightFace detection + 512-d ArcFace embedding
    ├── Reverse-image search backends (Yandex / Bing / Google Lens /
    │   FaceCheck.ID / PimEyes)
    └── Cosine-score every candidate against the query face

The pipeline does the heavy lifting; the adapter is responsible for
translating MatchResult records into the maltrace event shape that the
Rust core expects.
"""

from __future__ import annotations

import argparse
import asyncio
import dataclasses
import json
import logging
import os
import re
import sys
import traceback
import urllib.parse
from pathlib import Path
from typing import Any, Iterable, List, Optional

# Make the bundled `facesearch` package importable regardless of cwd.
_HERE = Path(__file__).resolve().parent
if str(_HERE) not in sys.path:
    sys.path.insert(0, str(_HERE))


# ──────────────────────────────────────────────────────────────────────
#  Transform manifest
#
#  Every transform listed here goes through `_dispatch_run` below. Adding
#  a new backend to `facesearch` is just an entry in this list plus the
#  matching backend name in `BACKEND_BY_MODULE`.
# ──────────────────────────────────────────────────────────────────────

TRANSFORMS: List[dict] = [
    {
        "id": "face_detect",
        "name": "Face detect (InsightFace)",
        "description": (
            "Detect faces in an image using InsightFace (ArcFace). Local "
            "only, no network. Produces one FACE entity per detected face "
            "with bounding box and detection score."
        ),
        "categories": ["Face"],
        "group": [],
        "consumes": ["IMAGE"],
        "produces": ["FACE"],
    },
    {
        "id": "face_search_yandex",
        "name": "Face search — Yandex",
        "description": (
            "Reverse image search via Yandex Images, then re-embed every "
            "candidate face and rank by cosine similarity to the query. No "
            "API key required."
        ),
        "categories": ["Face", "Search Engines"],
        "group": [],
        "consumes": ["IMAGE"],
        "produces": ["URL", "USERNAME", "HUMAN_NAME"],
    },
    {
        "id": "face_search_bing",
        "name": "Face search — Bing Visual",
        "description": (
            "Reverse image search via Bing Visual Search. Uses the Azure "
            "Cognitive Services API if BING_VISUAL_SEARCH_KEY is set; "
            "otherwise scrapes (fragile)."
        ),
        "categories": ["Face", "Search Engines"],
        "group": [],
        "consumes": ["IMAGE"],
        "produces": ["URL", "USERNAME", "HUMAN_NAME"],
    },
    {
        "id": "face_search_lens",
        "name": "Face search — Google Lens",
        "description": (
            "Reverse image search via Google Lens. Drives a headless "
            "Chromium browser (requires `playwright install chromium`). "
            "Highest fragility; turn on only when needed."
        ),
        "categories": ["Face", "Search Engines"],
        "group": [],
        "consumes": ["IMAGE"],
        "produces": ["URL", "USERNAME", "HUMAN_NAME"],
    },
    {
        "id": "face_search_facecheck",
        "name": "Face search — FaceCheck.ID",
        "description": (
            "Face-search via the FaceCheck.ID API. Set FACECHECK_API_KEY "
            "for real results; without a key the API returns demo hits "
            "(useful for pipeline testing)."
        ),
        "categories": ["Face", "Search Engines"],
        "group": [],
        "consumes": ["IMAGE"],
        "produces": ["URL", "USERNAME", "HUMAN_NAME"],
    },
    {
        "id": "face_search_all",
        "name": "Face search — all available backends",
        "description": (
            "Run every reverse-face backend that's available, dedupe by "
            "image and source URL, and rank the merged result set by "
            "embedding similarity. Slowest but most thorough."
        ),
        "categories": ["Face", "Search Engines"],
        "group": [],
        "consumes": ["IMAGE"],
        "produces": ["URL", "USERNAME", "HUMAN_NAME"],
    },
]

BACKEND_BY_MODULE = {
    "face_search_yandex":   "yandex",
    "face_search_bing":     "bing",
    "face_search_lens":     "google_lens",
    "face_search_facecheck": "facecheck",
    # face_search_all is special-cased in _dispatch_run.
}


# ──────────────────────────────────────────────────────────────────────
#  Social-handle extraction (used to surface USERNAME entities)
# ──────────────────────────────────────────────────────────────────────

_SOCIAL_HOSTS: dict[str, str] = {
    "instagram.com": r"^/([A-Za-z0-9_.]{2,30})/?$",
    "twitter.com":   r"^/([A-Za-z0-9_]{1,15})/?$",
    "x.com":         r"^/([A-Za-z0-9_]{1,15})/?$",
    "tiktok.com":    r"^/@([A-Za-z0-9_.]+)/?$",
    "facebook.com":  r"^/([A-Za-z0-9_.]+)/?$",
    "linkedin.com":  r"^/in/([A-Za-z0-9_-]+)/?$",
    "github.com":    r"^/([A-Za-z0-9_-]+)/?$",
    "vk.com":        r"^/([A-Za-z0-9_]+)/?$",
    "youtube.com":   r"^/(?:@|user/|c/|channel/)?([A-Za-z0-9_\-]+)/?$",
    "pinterest.com": r"^/([A-Za-z0-9_\-]+)/?$",
}

_SOCIAL_SKIP_PATHS = {"about", "help", "p", "explore", "watch", "tv", "search"}


def _extract_social_username(url: str) -> Optional[str]:
    """Pull a `handle@platform` from a known social URL, or None."""
    try:
        parsed = urllib.parse.urlparse(url)
    except ValueError:
        return None
    host = parsed.netloc.lower().removeprefix("www.")
    for known, pattern in _SOCIAL_HOSTS.items():
        if host == known or host.endswith("." + known):
            m = re.match(pattern, parsed.path)
            if not m:
                continue
            handle = m.group(1)
            if handle.lower() in _SOCIAL_SKIP_PATHS:
                return None
            return f"{handle}@{known}"
    return None


# ──────────────────────────────────────────────────────────────────────
#  face_detect (local, no network)
# ──────────────────────────────────────────────────────────────────────

def _run_face_detect(data: str, opts: dict) -> List[dict]:
    """Detect faces; emit one FACE event per face."""
    from facesearch.config import load_settings
    from facesearch.embeddings import FaceEmbedder
    from facesearch.utils import load_image_bytes
    from facesearch.pipeline import SearchPipeline

    settings = load_settings()
    img_bytes = _load_image_bytes(data)
    img = load_image_bytes(img_bytes)
    embedder = FaceEmbedder(settings)
    faces = embedder.encode(img)

    events: List[dict] = []
    for i, f in enumerate(faces):
        bbox = list(f.bbox)
        summary = (
            f"face {i + 1}/{len(faces)} "
            f"@ ({bbox[0]},{bbox[1]})→({bbox[2]},{bbox[3]}) "
            f"score={f.det_score:.3f}"
        )
        events.append({
            "event_type": "FACE",
            "data": summary,
            "module": "face_detect",
            "confidence": int(round(f.det_score * 100)),
            "source_event_type": "IMAGE",
            "extra": {
                "bbox": bbox,
                "det_score": float(f.det_score),
                "embedder": embedder.backend,
                "embedding_dim": embedder.embedding_dim,
                "source_image": data,
                "face_index": i,
            },
        })
    return events


# ──────────────────────────────────────────────────────────────────────
#  face_search_* (reverse image search + similarity scoring)
# ──────────────────────────────────────────────────────────────────────

async def _run_face_search(module_id: str, data: str, opts: dict) -> List[dict]:
    """Run one or more facesearch backends and translate matches to events."""
    from facesearch.config import load_settings
    from facesearch.pipeline import SearchPipeline
    from facesearch.search import build_backends

    settings = load_settings()
    # Per-transform overrides from --opts JSON.
    settings = _apply_opts(settings, opts)

    if module_id == "face_search_all":
        backend_names = ["yandex", "bing", "google_lens", "facecheck"]
    else:
        name = BACKEND_BY_MODULE.get(module_id)
        if name is None:
            raise ValueError(f"unknown face_search module: {module_id}")
        backend_names = [name]

    backends = build_backends(backend_names, settings)
    runnable = [b for b in backends if b.is_available()]
    if not runnable:
        raise RuntimeError(
            f"no runnable backends for {module_id} "
            f"(check API keys / Playwright install)"
        )

    pipeline = SearchPipeline(settings)
    img_bytes = _load_image_bytes(data)
    report = await pipeline.run(img_bytes, runnable, thumbnails_dir=None)

    return _matches_to_events(report, source_image=data)


def _apply_opts(settings, opts: dict):
    """Apply CLI-supplied per-transform overrides onto Settings."""
    overrides: dict[str, Any] = {}
    if "candidate_threshold" in opts:
        overrides["candidate_threshold"] = float(opts["candidate_threshold"])
    if "match_threshold" in opts:
        overrides["match_threshold"] = float(opts["match_threshold"])
    if "per_backend_limit" in opts:
        overrides["per_backend_limit"] = int(opts["per_backend_limit"])
    if "total_candidate_limit" in opts:
        overrides["total_candidate_limit"] = int(opts["total_candidate_limit"])
    if not overrides:
        return settings
    return dataclasses.replace(settings, **overrides)


def _matches_to_events(report, *, source_image: str) -> List[dict]:
    """Translate a PipelineReport into maltrace events.

    Each MatchResult produces:
      * One URL event for the source page (or the image URL if no page).
      * Optionally one USERNAME event if the URL is a known social profile.

    All events carry an `extra.similarity` so the inspector can sort and
    colour by confidence, and `extra.label` ("strong" / "likely" / …)
    so the canvas can flag high-confidence matches.

    HUMAN_NAME events are emitted for any match `title` that looks like
    a person name (heuristic: 2–5 capitalized tokens, no URLs in it).
    """
    events: List[dict] = []
    seen_usernames: set[str] = set()
    seen_human_names: set[str] = set()
    seen_urls: set[str] = set()

    for m in report.matches:
        primary_url = m.source_url or m.image_url
        if not primary_url or primary_url.startswith("data:"):
            # Pure thumbnail with no source page — skip; not useful as a node.
            continue
        if primary_url in seen_urls:
            continue
        seen_urls.add(primary_url)

        confidence = int(round(max(0.0, min(1.0, m.similarity)) * 100))
        extra = {
            "similarity": m.similarity,
            "label": m.label,
            "backend": m.backend,
            "image_url": m.image_url,
            "source_url": m.source_url,
            "title": m.title,
            "backend_score": m.backend_score,
            "det_score": m.det_score,
            "bbox": list(m.bbox),
            "candidate_face_count": m.candidate_face_count,
            "source_image": source_image,
        }

        # USERNAME first so it's the parent for any downstream profile work.
        username = _extract_social_username(primary_url)
        if username and username not in seen_usernames:
            seen_usernames.add(username)
            events.append({
                "event_type": "USERNAME",
                "data": username,
                "module": f"face_search_{m.backend}",
                "confidence": confidence,
                "source_event_type": "IMAGE",
                "extra": dict(extra, matched_url=primary_url),
            })

        events.append({
            "event_type": "URL",
            "data": primary_url,
            "module": f"face_search_{m.backend}",
            "confidence": confidence,
            "source_event_type": "IMAGE",
            "extra": extra,
        })

        # HUMAN_NAME from titles that look like a person name (e.g. captions
        # from FaceCheck or alt-text from Yandex). Down-weight confidence
        # because title noise is high.
        if m.title:
            name_candidate = _maybe_human_name(m.title)
            if name_candidate and name_candidate not in seen_human_names:
                seen_human_names.add(name_candidate)
                events.append({
                    "event_type": "HUMAN_NAME",
                    "data": name_candidate,
                    "module": f"face_search_{m.backend}",
                    "confidence": max(30, confidence - 20),
                    "source_event_type": "IMAGE",
                    "extra": dict(extra, source_field="title"),
                })

    return events


_NAME_TOKEN_RE = re.compile(r"^[A-Z][a-zà-öø-ÿ'\-]{1,}$")


def _maybe_human_name(title: str) -> Optional[str]:
    """Best-effort: pluck a 'First Last' (up to 4 tokens) from `title`."""
    cleaned = re.sub(r"\s+", " ", title).strip()
    if any(t in cleaned for t in ("http://", "https://", "www.")):
        return None
    # Trim trailing punctuation / hyphen-suffixed site name.
    cleaned = re.split(r"\s+[\-|·•]\s+", cleaned, maxsplit=1)[0]
    tokens = cleaned.split(" ")[:5]
    if len(tokens) < 2:
        return None
    if all(_NAME_TOKEN_RE.match(t) for t in tokens):
        return " ".join(tokens)
    return None


# ──────────────────────────────────────────────────────────────────────
#  Image loading helper
# ──────────────────────────────────────────────────────────────────────

def _load_image_bytes(src: str) -> bytes:
    """Resolve `src` (URL or local path) to raw image bytes.

    URL fetches use `requests` (sync, fine because each adapter invocation
    is a one-shot subprocess). For larger images we cap at 25 MB to keep
    accidental misuse cheap.
    """
    if src.startswith(("http://", "https://")):
        import requests
        r = requests.get(
            src,
            timeout=20,
            headers={"User-Agent": "maltrace/0.1"},
            stream=True,
        )
        r.raise_for_status()
        chunks: list[bytes] = []
        total = 0
        for chunk in r.iter_content(chunk_size=65536):
            chunks.append(chunk)
            total += len(chunk)
            if total > 25 * 1024 * 1024:
                raise ValueError(f"query image at {src} exceeds 25 MB")
        return b"".join(chunks)
    p = Path(src).expanduser().resolve()
    if not p.is_file():
        raise FileNotFoundError(f"image not found: {src}")
    return p.read_bytes()


# ──────────────────────────────────────────────────────────────────────
#  Dispatch
# ──────────────────────────────────────────────────────────────────────

def _dispatch_run(module: str, data: str, opts: dict) -> List[dict]:
    if module == "face_detect":
        return _run_face_detect(data, opts)
    if module in BACKEND_BY_MODULE or module == "face_search_all":
        return asyncio.run(_run_face_search(module, data, opts))
    raise ValueError(f"unknown module: {module}")


# ──────────────────────────────────────────────────────────────────────
#  Subcommand entry points
# ──────────────────────────────────────────────────────────────────────

def cmd_list_transforms(_: argparse.Namespace) -> int:
    json.dump({"transforms": TRANSFORMS, "skipped": []}, sys.stdout)
    sys.stdout.write("\n")
    return 0


def cmd_run(args: argparse.Namespace) -> int:
    module: str = args.module
    data: str = args.data
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
        events = _dispatch_run(module, data, opts)
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
    # Quiet by default; the adapter must produce clean JSON on stdout. The
    # `facesearch` logger writes only to stderr.
    logging.basicConfig(
        level=os.environ.get("FACESEARCH_LOGLEVEL", "WARNING"),
        stream=sys.stderr,
        format="%(levelname)s %(name)s: %(message)s",
    )

    parser = argparse.ArgumentParser(prog="maltrace-face-search-adapter")
    sub = parser.add_subparsers(dest="cmd", required=True)
    sub.add_parser("list-transforms", help="List face transforms.")

    run = sub.add_parser("run", help="Run one face transform.")
    run.add_argument("--module", required=True)
    run.add_argument("--event-type", required=True, help="IMAGE")
    run.add_argument("--data", required=True, help="Image URL or local file path.")
    run.add_argument("--opts", default=None,
                     help="JSON object of per-transform options "
                          "(e.g. {\"match_threshold\":0.5})")

    args = parser.parse_args(argv)
    if args.cmd == "list-transforms":
        return cmd_list_transforms(args)
    if args.cmd == "run":
        return cmd_run(args)
    parser.print_help(sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main())
