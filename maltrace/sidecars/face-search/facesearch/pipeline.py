"""Reverse-face-search pipeline.

Inputs: a query image (file path, bytes, or URL).
Outputs: a ranked list of MatchResult objects.

Stages:
  1. Load + embed the query face.
  2. Encode the original query bytes as JPEG so each backend receives the
     same byte payload (some engines reject PNG/WEBP uploads).
  3. Run every enabled backend concurrently. Each yields CandidateHits.
  4. De-duplicate hits by image_url and source_url.
  5. Download each candidate (cached, concurrency-limited).
  6. Detect & embed faces in each candidate; cosine-score against the query.
  7. Filter by `candidate_threshold`, keep top N per source page, rank by
     similarity, and return.
"""

from __future__ import annotations

import asyncio
import io
import logging
import time
from dataclasses import dataclass, field, asdict
from pathlib import Path
from typing import Optional, Sequence

import httpx
from PIL import Image

from .config import Settings
from .embeddings import FaceEmbedder, FaceEncoding
from .fetch import Fetcher
from .search import SearchBackend, CandidateHit
from .utils import (
    confidence_label,
    cosine_similarity,
    load_image_bytes,
    perceptual_hash,
)

log = logging.getLogger("facesearch.pipeline")


@dataclass(slots=True)
class MatchResult:
    image_url: str
    source_url: Optional[str]
    backend: str
    similarity: float
    label: str
    backend_score: Optional[float]
    title: Optional[str]
    det_score: float
    candidate_face_count: int
    bbox: tuple[int, int, int, int]
    thumbnail_path: Optional[str] = None
    error: Optional[str] = None

    def to_dict(self) -> dict:
        return asdict(self)


@dataclass(slots=True)
class QueryInfo:
    source: str
    width: int
    height: int
    embedder_backend: str
    embedding_dim: int
    det_score: float
    bbox: tuple[int, int, int, int]
    face_count: int


@dataclass(slots=True)
class PipelineReport:
    query: QueryInfo
    settings_summary: dict
    elapsed_seconds: float
    candidate_count: int
    embedded_count: int
    matches: list[MatchResult]
    per_backend_counts: dict[str, int] = field(default_factory=dict)

    def to_dict(self) -> dict:
        return {
            "query": asdict(self.query),
            "settings": self.settings_summary,
            "elapsed_seconds": self.elapsed_seconds,
            "candidate_count": self.candidate_count,
            "embedded_count": self.embedded_count,
            "per_backend_counts": self.per_backend_counts,
            "matches": [m.to_dict() for m in self.matches],
        }


class SearchPipeline:
    def __init__(self, settings: Settings, embedder: Optional[FaceEmbedder] = None) -> None:
        self.settings = settings
        self.embedder = embedder or FaceEmbedder(settings)

    async def run(
        self,
        query_image: bytes | str | Path,
        backends: Sequence[SearchBackend],
        *,
        thumbnails_dir: Optional[Path] = None,
    ) -> PipelineReport:
        started = time.monotonic()

        query_bytes, query_pil, query_source = await self._load_query(query_image)
        query_face = self.embedder.encode_primary(query_pil)
        if query_face is None:
            raise ValueError(
                "No face detected in the query image. Provide a clearer photo "
                "with the subject's face occupying a meaningful portion of the frame."
            )

        # Re-encode the cropped + padded face area as JPEG so backends get a
        # consistent payload. We send the full image; some engines weight
        # context (clothing, background) into their reverse-image search.
        upload_bytes = _to_jpeg(query_pil)

        query_info = QueryInfo(
            source=query_source,
            width=query_pil.width,
            height=query_pil.height,
            embedder_backend=self.embedder.backend,
            embedding_dim=self.embedder.embedding_dim,
            det_score=query_face.det_score,
            bbox=query_face.bbox,
            face_count=len(self.embedder.encode(query_pil)),
        )

        async with Fetcher(self.settings) as fetcher:
            backend_hits = await self._collect_candidates(upload_bytes, backends, fetcher)
            per_backend_counts = {b.name: len(h) for b, h in backend_hits}

            hits = self._dedupe([h for _, hs in backend_hits for h in hs])
            hits = hits[: self.settings.total_candidate_limit]

            matches = await self._score_candidates(
                hits, query_face, fetcher, thumbnails_dir=thumbnails_dir
            )

        matches.sort(key=lambda m: m.similarity, reverse=True)

        return PipelineReport(
            query=query_info,
            settings_summary=self._settings_summary(backends),
            elapsed_seconds=round(time.monotonic() - started, 3),
            candidate_count=len(hits),
            embedded_count=sum(1 for m in matches if m.error is None),
            matches=matches,
            per_backend_counts=per_backend_counts,
        )

    # --- stages ---------------------------------------------------------

    async def _load_query(
        self, query: bytes | str | Path
    ) -> tuple[bytes, Image.Image, str]:
        if isinstance(query, bytes):
            return query, load_image_bytes(query), "<bytes>"
        if isinstance(query, Path):
            data = query.read_bytes()
            return data, load_image_bytes(data), str(query)
        if isinstance(query, str):
            if query.startswith(("http://", "https://")):
                async with httpx.AsyncClient(
                    headers={"User-Agent": self.settings.user_agent},
                    timeout=self.settings.http_timeout,
                    follow_redirects=True,
                ) as client:
                    resp = await client.get(query)
                    resp.raise_for_status()
                    data = resp.content
                return data, load_image_bytes(data), query
            path = Path(query)
            data = path.read_bytes()
            return data, load_image_bytes(data), str(path)
        raise TypeError(f"Unsupported query type: {type(query)!r}")

    async def _collect_candidates(
        self,
        upload_bytes: bytes,
        backends: Sequence[SearchBackend],
        fetcher: Fetcher,
    ) -> list[tuple[SearchBackend, list[CandidateHit]]]:
        async def run_one(backend: SearchBackend) -> tuple[SearchBackend, list[CandidateHit]]:
            if not backend.is_available():
                log.info("backend %s unavailable; skipping", backend.name)
                return backend, []
            try:
                hits = await backend.search(
                    upload_bytes, fetcher, limit=self.settings.per_backend_limit
                )
            except Exception as exc:  # noqa: BLE001
                log.warning("backend %s raised: %s", backend.name, exc)
                return backend, []
            log.info("backend %s returned %d hits", backend.name, len(hits))
            return backend, hits

        results = await asyncio.gather(*(run_one(b) for b in backends))
        return list(results)

    def _dedupe(self, hits: list[CandidateHit]) -> list[CandidateHit]:
        seen_images: set[str] = set()
        seen_sources: dict[str, CandidateHit] = {}
        out: list[CandidateHit] = []
        for h in hits:
            if h.image_url in seen_images:
                continue
            seen_images.add(h.image_url)
            # Prefer the higher-scoring hit when two backends found the same page.
            if h.source_url and h.source_url in seen_sources:
                prior = seen_sources[h.source_url]
                if (h.score or 0) <= (prior.score or 0):
                    continue
            if h.source_url:
                seen_sources[h.source_url] = h
            out.append(h)
        return out

    async def _score_candidates(
        self,
        hits: list[CandidateHit],
        query_face: FaceEncoding,
        fetcher: Fetcher,
        *,
        thumbnails_dir: Optional[Path] = None,
    ) -> list[MatchResult]:
        sem = asyncio.Semaphore(self.settings.download_concurrency)
        seen_phashes: set[str] = set()
        results: list[MatchResult] = []
        lock = asyncio.Lock()

        async def process(hit: CandidateHit) -> None:
            async with sem:
                try:
                    if hit.image_url.startswith("data:"):
                        data = _decode_data_url(hit.image_url)
                        if data is None:
                            return
                        pil = load_image_bytes(data)
                    else:
                        dl = await fetcher.download_image(hit.image_url)
                        if dl is None:
                            return
                        try:
                            pil = load_image_bytes(dl.data)
                        except Exception:  # noqa: BLE001
                            return

                    # Cheap dedup on image content (handles re-hosts).
                    try:
                        phash = perceptual_hash(pil)
                    except Exception:  # noqa: BLE001
                        phash = ""
                    async with lock:
                        if phash and phash in seen_phashes:
                            return
                        if phash:
                            seen_phashes.add(phash)

                    faces = await asyncio.to_thread(self.embedder.encode, pil)
                    if not faces:
                        return
                    best_face = max(faces, key=lambda f: cosine_similarity(
                        f.embedding, query_face.embedding))
                    sim = cosine_similarity(best_face.embedding, query_face.embedding)
                    if sim < self.settings.candidate_threshold:
                        return

                    thumb_path: Optional[str] = None
                    if thumbnails_dir is not None:
                        thumb_path = _save_thumbnail(pil, best_face.bbox,
                                                     thumbnails_dir,
                                                     f"{hit.backend}_{abs(hash(hit.image_url)):x}")

                    result = MatchResult(
                        image_url=hit.image_url,
                        source_url=hit.source_url,
                        backend=hit.backend,
                        similarity=round(sim, 4),
                        label=confidence_label(sim, self.settings.match_threshold),
                        backend_score=hit.score,
                        title=hit.title,
                        det_score=round(best_face.det_score, 3),
                        candidate_face_count=len(faces),
                        bbox=best_face.bbox,
                        thumbnail_path=thumb_path,
                    )
                    async with lock:
                        results.append(result)
                except Exception as exc:  # noqa: BLE001
                    log.debug("scoring failed url=%s err=%s", hit.image_url, exc)

        await asyncio.gather(*(process(h) for h in hits))
        return results

    def _settings_summary(self, backends: Sequence[SearchBackend]) -> dict:
        s = self.settings
        return {
            "model": s.model_name,
            "embedder": self.embedder.backend,
            "embedding_dim": self.embedder.embedding_dim,
            "candidate_threshold": s.candidate_threshold,
            "match_threshold": s.match_threshold,
            "per_backend_limit": s.per_backend_limit,
            "total_candidate_limit": s.total_candidate_limit,
            "backends": [b.name for b in backends],
        }


def _to_jpeg(image: Image.Image, max_side: int = 1600, quality: int = 92) -> bytes:
    img = image
    if max(img.size) > max_side:
        ratio = max_side / max(img.size)
        new_size = (int(img.width * ratio), int(img.height * ratio))
        img = img.resize(new_size, Image.Resampling.LANCZOS)
    if img.mode != "RGB":
        img = img.convert("RGB")
    buf = io.BytesIO()
    img.save(buf, format="JPEG", quality=quality, optimize=True)
    return buf.getvalue()


def _save_thumbnail(image: Image.Image, bbox: tuple[int, int, int, int],
                    out_dir: Path, name: str, side: int = 256) -> str:
    out_dir.mkdir(parents=True, exist_ok=True)
    x1, y1, x2, y2 = bbox
    pad = int(0.25 * max(x2 - x1, y2 - y1))
    crop = image.crop((max(0, x1 - pad), max(0, y1 - pad),
                       min(image.width, x2 + pad), min(image.height, y2 + pad)))
    crop.thumbnail((side, side), Image.Resampling.LANCZOS)
    out_path = out_dir / f"{name}.jpg"
    crop.convert("RGB").save(out_path, "JPEG", quality=88)
    return str(out_path)


def _decode_data_url(url: str) -> Optional[bytes]:
    import base64
    try:
        header, b64 = url.split(",", 1)
    except ValueError:
        return None
    if ";base64" not in header:
        return None
    try:
        return base64.b64decode(b64)
    except Exception:  # noqa: BLE001
        return None
