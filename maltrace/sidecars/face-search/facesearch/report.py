"""Write the pipeline output as JSON and a browsable HTML report."""

from __future__ import annotations

import html
import json
import os
from pathlib import Path
from typing import Iterable

from .pipeline import MatchResult, PipelineReport


HTML_TEMPLATE = """<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>facesearch report</title>
<style>
  body {{ font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", system-ui, sans-serif;
         margin: 0; background: #0f1115; color: #e6e6e6; }}
  header {{ padding: 20px 28px; background: #161922; border-bottom: 1px solid #262a35; }}
  h1 {{ margin: 0 0 4px; font-size: 18px; font-weight: 600; }}
  .meta {{ color: #9aa0aa; font-size: 12px; }}
  .meta code {{ background: #1f2330; padding: 1px 6px; border-radius: 4px; }}
  main {{ padding: 16px 28px; }}
  .grid {{ display: grid; grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
           gap: 14px; }}
  .card {{ background: #161922; border: 1px solid #262a35; border-radius: 10px;
           overflow: hidden; display: flex; flex-direction: column; }}
  .card img {{ width: 100%; aspect-ratio: 1 / 1; object-fit: cover; background: #000; }}
  .card .body {{ padding: 10px 12px; font-size: 12px; line-height: 1.45; }}
  .score {{ font-size: 14px; font-weight: 600; }}
  .label {{ display: inline-block; padding: 1px 7px; border-radius: 999px; font-size: 11px;
            margin-left: 6px; vertical-align: middle; }}
  .label.strong   {{ background: #16361f; color: #59d977; }}
  .label.likely   {{ background: #2b3614; color: #c7df57; }}
  .label.possible {{ background: #3a2b14; color: #e0b366; }}
  .label.weak     {{ background: #36181a; color: #d97777; }}
  .card a {{ color: #82a8ff; text-decoration: none; word-break: break-all; }}
  .card a:hover {{ text-decoration: underline; }}
  .empty {{ color: #9aa0aa; padding: 40px; text-align: center; }}
</style>
</head>
<body>
<header>
  <h1>facesearch report</h1>
  <div class="meta">
    query: <code>{query_source}</code> &middot;
    embedder: <code>{embedder}</code> ({embedding_dim}-d) &middot;
    candidates: {candidate_count} &middot;
    embedded: {embedded_count} &middot;
    elapsed: {elapsed}s &middot;
    backends: {backends}
  </div>
</header>
<main>
{body}
</main>
</body>
</html>
"""


def write_report(report: PipelineReport, out_dir: Path) -> dict[str, Path]:
    """Write report.json and report.html into out_dir. Returns the paths."""
    out_dir.mkdir(parents=True, exist_ok=True)
    json_path = out_dir / "report.json"
    html_path = out_dir / "report.html"
    json_path.write_text(json.dumps(report.to_dict(), indent=2), encoding="utf-8")
    html_path.write_text(_render_html(report, out_dir), encoding="utf-8")
    return {"json": json_path, "html": html_path}


def _render_html(report: PipelineReport, out_dir: Path) -> str:
    backends = ", ".join(sorted(report.per_backend_counts.keys())) or "(none)"
    body = _render_grid(report.matches, out_dir) if report.matches else \
        '<div class="empty">No matches above the candidate threshold.</div>'
    return HTML_TEMPLATE.format(
        query_source=html.escape(report.query.source),
        embedder=html.escape(report.query.embedder_backend or "?"),
        embedding_dim=report.query.embedding_dim or "?",
        candidate_count=report.candidate_count,
        embedded_count=report.embedded_count,
        elapsed=report.elapsed_seconds,
        backends=html.escape(backends),
        body=body,
    )


def _render_grid(matches: Iterable[MatchResult], out_dir: Path) -> str:
    cards: list[str] = ['<div class="grid">']
    for m in matches:
        thumb_src = _thumbnail_src(m, out_dir)
        source_link = (
            f'<a href="{html.escape(m.source_url)}" target="_blank" rel="noreferrer noopener">source</a>'
            if m.source_url else "<span>(no source)</span>"
        )
        image_link = (
            f'<a href="{html.escape(m.image_url)}" target="_blank" rel="noreferrer noopener">image</a>'
        )
        backend_score = f' backend={m.backend_score:.2f}' if m.backend_score is not None else ''
        title = html.escape(m.title) if m.title else ""
        cards.append(
            "<div class='card'>"
            f"<img src='{html.escape(thumb_src)}' loading='lazy' alt=''>"
            "<div class='body'>"
            f"<div><span class='score'>{m.similarity:.3f}</span>"
            f"<span class='label {html.escape(m.label)}'>{html.escape(m.label)}</span></div>"
            f"<div>backend={html.escape(m.backend)}{html.escape(backend_score)}"
            f" det={m.det_score:.2f} faces={m.candidate_face_count}</div>"
            f"<div>{source_link} &middot; {image_link}</div>"
            + (f"<div>{title}</div>" if title else "")
            + "</div></div>"
        )
    cards.append("</div>")
    return "".join(cards)


def _thumbnail_src(m: MatchResult, out_dir: Path) -> str:
    if m.thumbnail_path:
        try:
            rel = os.path.relpath(m.thumbnail_path, out_dir)
            return rel
        except ValueError:
            return m.thumbnail_path
    # Fall back to the remote image URL.
    return m.image_url
