"""Command-line entry point for facesearch."""

from __future__ import annotations

import argparse
import asyncio
import json
import logging
import sys
from dataclasses import replace
from pathlib import Path

from .config import load_settings
from .pipeline import SearchPipeline
from .report import write_report
from .search import available_backend_names, build_backends


def _parser() -> argparse.ArgumentParser:
    backend_names = available_backend_names()
    p = argparse.ArgumentParser(
        prog="facesearch",
        description="Reverse face search across multiple engines. "
                    "Provide a query image (path or URL); the tool will "
                    "search, score, and write an HTML report.",
    )
    p.add_argument("query", nargs="?",
                   help="Path or URL to a query image containing a single face.")
    p.add_argument("-o", "--out", default="./facesearch-report",
                   help="Output directory (default: ./facesearch-report).")
    p.add_argument("-b", "--backend", action="append", default=None,
                   choices=backend_names,
                   help=f"Search backend to enable (repeatable). "
                        f"Default: yandex. Options: {', '.join(backend_names)}.")
    p.add_argument("--all-backends", action="store_true",
                   help="Enable every backend that reports as available.")
    p.add_argument("--match-threshold", type=float, default=None,
                   help="Cosine similarity at which to label a match (default 0.55).")
    p.add_argument("--candidate-threshold", type=float, default=None,
                   help="Min cosine similarity to keep a candidate (default 0.35).")
    p.add_argument("--per-backend-limit", type=int, default=None,
                   help="Max candidates fetched per backend (default 40).")
    p.add_argument("--total-candidate-limit", type=int, default=None,
                   help="Max candidates carried through to download (default 200).")
    p.add_argument("--no-thumbnails", action="store_true",
                   help="Skip writing cropped-face thumbnails into the output dir.")
    p.add_argument("--list-backends", action="store_true",
                   help="Print available backends and exit.")
    p.add_argument("-v", "--verbose", action="count", default=0,
                   help="Verbose logging (-v, -vv).")
    p.add_argument("--json", action="store_true",
                   help="Print the JSON report to stdout in addition to writing it.")
    return p


def _configure_logging(verbosity: int) -> None:
    level = logging.WARNING
    if verbosity == 1:
        level = logging.INFO
    elif verbosity >= 2:
        level = logging.DEBUG
    logging.basicConfig(
        level=level,
        format="%(asctime)s %(levelname)-7s %(name)s: %(message)s",
        datefmt="%H:%M:%S",
    )


def main(argv: list[str] | None = None) -> int:
    args = _parser().parse_args(argv)
    _configure_logging(args.verbose)

    if args.list_backends:
        settings = load_settings()
        backends = build_backends(available_backend_names(), settings)
        for b in backends:
            status = "available" if b.is_available() else "unavailable"
            print(f"  {b.name:<14} {status}")
        return 0

    if not args.query:
        _parser().print_help(sys.stderr)
        return 2

    settings = load_settings()
    overrides = {}
    if args.match_threshold is not None:
        overrides["match_threshold"] = args.match_threshold
    if args.candidate_threshold is not None:
        overrides["candidate_threshold"] = args.candidate_threshold
    if args.per_backend_limit is not None:
        overrides["per_backend_limit"] = args.per_backend_limit
    if args.total_candidate_limit is not None:
        overrides["total_candidate_limit"] = args.total_candidate_limit
    if overrides:
        settings = replace(settings, **overrides)

    if args.all_backends:
        backend_names = available_backend_names()
    elif args.backend:
        backend_names = args.backend
    else:
        backend_names = list(settings.default_backends)

    backends = build_backends(backend_names, settings)
    # Drop unavailable ones up front so the user gets a clear log line.
    runnable = []
    for b in backends:
        if b.is_available():
            runnable.append(b)
        else:
            logging.getLogger("facesearch").warning(
                "backend %s is unavailable (missing key or dependency); skipping", b.name
            )
    if not runnable:
        print("No runnable backends. Run with --list-backends to see status.",
              file=sys.stderr)
        return 1

    out_dir = Path(args.out)
    out_dir.mkdir(parents=True, exist_ok=True)
    thumbnails_dir = None if args.no_thumbnails else (out_dir / "thumbnails")

    pipeline = SearchPipeline(settings)
    try:
        report = asyncio.run(pipeline.run(
            args.query, runnable, thumbnails_dir=thumbnails_dir
        ))
    except ValueError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 3
    except KeyboardInterrupt:
        print("interrupted", file=sys.stderr)
        return 130

    paths = write_report(report, out_dir)
    print(f"wrote {paths['json']}")
    print(f"wrote {paths['html']}")
    print(
        f"candidates={report.candidate_count} "
        f"embedded={report.embedded_count} "
        f"matches={len(report.matches)} "
        f"elapsed={report.elapsed_seconds}s"
    )
    if report.matches:
        top = report.matches[0]
        print(f"top: similarity={top.similarity:.3f} label={top.label} "
              f"backend={top.backend} url={top.source_url or top.image_url}")
    if args.json:
        json.dump(report.to_dict(), sys.stdout, indent=2)
        sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
