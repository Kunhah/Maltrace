# face-search sidecar

maltrace's reverse face-search sidecar. Implements the same one-shot
subprocess protocol as `spiderfoot-adapter` — each invocation runs one
transform on one event and prints a JSON document on stdout.

## Architecture

```
adapter.py                  ← maltrace event protocol (this file)
└── facesearch/             ← bundled library (lives entirely in-tree)
    ├── embeddings.py       ← InsightFace ArcFace, 512-d, L2-normalized
    ├── pipeline.py         ← orchestrates search → download → re-embed → score
    ├── fetch.py            ← async HTTP + image cache (~/.cache/facesearch/)
    ├── search/
    │   ├── yandex.py       ← free, no key
    │   ├── bing.py         ← Azure Visual Search API + scrape fallback
    │   ├── google_lens.py  ← Playwright-driven (optional dep)
    │   ├── facecheck.py    ← FaceCheck.ID API + demo mode
    │   └── pimeyes.py      ← stub for private integrations
    └── …
```

The pipeline embeds the query face once, runs every enabled backend
concurrently, then downloads each candidate, re-detects faces in it, and
computes cosine similarity to the query embedding. Only candidates above
`candidate_threshold` (default 0.35) are returned; anything above
`match_threshold` (default 0.55) is labelled a likely match.

## Transforms

| id                       | input   | output                       | network                       |
|--------------------------|---------|------------------------------|-------------------------------|
| `face_detect`            | `IMAGE` | `FACE`                       | none                          |
| `face_search_yandex`     | `IMAGE` | `URL`, `USERNAME`, `HUMAN_NAME` | Yandex                     |
| `face_search_bing`       | `IMAGE` | `URL`, `USERNAME`, `HUMAN_NAME` | Bing (API or scrape)       |
| `face_search_lens`       | `IMAGE` | `URL`, `USERNAME`, `HUMAN_NAME` | Google Lens (Playwright)   |
| `face_search_facecheck`  | `IMAGE` | `URL`, `USERNAME`, `HUMAN_NAME` | FaceCheck.ID API           |
| `face_search_all`        | `IMAGE` | `URL`, `USERNAME`, `HUMAN_NAME` | every available backend    |

Every face-search transform emits one `URL` event per match, plus a
`USERNAME` event when the URL is a recognised social profile and a
`HUMAN_NAME` event when the result title looks like a person name. Each
event's `extra` carries:

```json
{
  "similarity": 0.71,
  "label": "strong",
  "backend": "yandex",
  "image_url": "…",
  "source_url": "…",
  "title": "…",
  "det_score": 0.94,
  "bbox": [x1, y1, x2, y2],
  "candidate_face_count": 2,
  "source_image": "<original input>"
}
```

`confidence` on the maltrace event is `round(similarity * 100)`, so the
inspector can sort high-quality hits to the top without parsing `extra`.

## Why InsightFace

ArcFace (the `buffalo_l` bundle) is state-of-the-art for face embedding,
CPU-capable through ONNX Runtime, well-maintained, and small enough to
ship as a one-time ~120 MB download into `~/.insightface/models/`. The
L2-normalised 512-d embedding lets us turn cosine similarity into a
direct dot product, which keeps scoring cheap when we re-embed every
candidate the search backends return.

## Why per-backend transforms instead of one

Each backend has different latency, cost, and accuracy. Listing them
separately lets the canvas operator pick the right one for the case
(`yandex` for free-text searches, `facecheck` for serious investigation,
`face_search_all` for thorough but slow). The dispatcher in `adapter.py`
is one line per backend; the heavy lifting lives in `facesearch/`.

## Per-transform options

The `--opts` JSON is forwarded to every transform. Currently honoured:

```json
{
  "candidate_threshold":      0.30,
  "match_threshold":          0.55,
  "per_backend_limit":        40,
  "total_candidate_limit":    200
}
```

Lower `candidate_threshold` if you want exploratory recall; raise
`match_threshold` to label fewer matches as "likely".

## Usage

```bash
.venv/bin/python adapter.py list-transforms

# Local detect, no network.
.venv/bin/python adapter.py run --module face_detect \
  --event-type IMAGE --data /path/to/photo.jpg

# Yandex reverse face search with stricter thresholds.
.venv/bin/python adapter.py run --module face_search_yandex \
  --event-type IMAGE --data /path/to/photo.jpg \
  --opts '{"match_threshold":0.6}'

# Run every available backend.
.venv/bin/python adapter.py run --module face_search_all \
  --event-type IMAGE --data /path/to/photo.jpg
```

Exit codes: `0` ok, `2` usage error, `3` transform raised an exception
(traceback included in the JSON).

## See also

- [SETUP.md](SETUP.md) — Python 3.12 venv, API keys, Playwright install.
- [`facesearch/` README](facesearch/) ships as a standalone CLI too: run
  `.venv/bin/python -m facesearch <image>` for a browsable HTML report
  independent of the maltrace protocol.
