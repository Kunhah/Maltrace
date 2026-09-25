"""FaceCheck.ID API backend.

FaceCheck.ID exposes a real face-search API. The flow is:
  1. POST /api/upload_pic with the image to receive an `id_search` token.
  2. POST /api/search with that token and poll until results arrive.

API docs: https://facecheck.id/api

Auth: set `FACECHECK_API_KEY` env var or `settings.facecheck_api_key`.
If unset, the backend stays available but uses `demo=true`, which returns
fake hits — useful for end-to-end pipeline testing without burning credits.
"""

from __future__ import annotations

import asyncio
import logging
from typing import Iterable

from ..config import Settings
from ..fetch import Fetcher
from .base import CandidateHit

log = logging.getLogger("facesearch.search.facecheck")

UPLOAD_URL = "https://facecheck.id/api/upload_pic"
SEARCH_URL = "https://facecheck.id/api/search"

POLL_INTERVAL = 2.0
POLL_TIMEOUT = 90.0


class FaceCheckBackend:
    name = "facecheck"

    def __init__(self, settings: Settings) -> None:
        self.settings = settings
        self.api_key = settings.facecheck_api_key
        self.demo = self.api_key is None

    def is_available(self) -> bool:
        return True

    async def search(self, image_bytes: bytes, fetcher: Fetcher,
                     *, limit: int) -> list[CandidateHit]:
        try:
            search_id = await self._upload(image_bytes, fetcher)
        except Exception as exc:  # noqa: BLE001
            log.warning("facecheck upload failed: %s", exc)
            return []
        if not search_id:
            return []
        try:
            items = await self._poll(search_id, fetcher)
        except Exception as exc:  # noqa: BLE001
            log.warning("facecheck search failed: %s", exc)
            return []
        return list(self._to_hits(items, limit))

    async def _upload(self, image_bytes: bytes, fetcher: Fetcher) -> str | None:
        headers = {"accept": "application/json"}
        if self.api_key:
            headers["Authorization"] = self.api_key
        files = {"images": ("query.jpg", image_bytes, "image/jpeg")}
        # The API accepts an `id_search` empty value to start a new search.
        data = {"id_search": "", "demo": "true" if self.demo else "false"}
        resp = await fetcher.client.post(UPLOAD_URL, headers=headers,
                                         files=files, data=data)
        resp.raise_for_status()
        payload = resp.json()
        if payload.get("error"):
            log.warning("facecheck upload error: %s", payload.get("error"))
            return None
        return payload.get("id_search")

    async def _poll(self, id_search: str, fetcher: Fetcher) -> list[dict]:
        headers = {"accept": "application/json", "content-type": "application/json"}
        if self.api_key:
            headers["Authorization"] = self.api_key
        body = {
            "id_search": id_search,
            "with_progress": True,
            "status_only": False,
            "demo": bool(self.demo),
        }
        deadline = asyncio.get_event_loop().time() + POLL_TIMEOUT
        while True:
            resp = await fetcher.client.post(SEARCH_URL, headers=headers, json=body)
            resp.raise_for_status()
            payload = resp.json()
            if payload.get("error"):
                log.warning("facecheck poll error: %s", payload.get("error"))
                return []
            output = payload.get("output") or {}
            items = output.get("items") or []
            if items:
                return items
            if asyncio.get_event_loop().time() > deadline:
                log.warning("facecheck polling timed out")
                return []
            await asyncio.sleep(POLL_INTERVAL)

    def _to_hits(self, items: list[dict], limit: int) -> Iterable[CandidateHit]:
        for item in items[:limit]:
            url = item.get("url")
            if not url:
                continue
            score = item.get("score")
            # FaceCheck scores are 0..100.
            normalized = (score / 100.0) if isinstance(score, (int, float)) else None
            # The `base64` field is a data URL of the thumbnail; we pass it as
            # image_url so the pipeline can embed it without re-fetching.
            image_url = item.get("base64") or url
            yield CandidateHit(
                image_url=image_url,
                source_url=url,
                backend=self.name,
                score=normalized,
                title=item.get("group") and f"group {item['group']}" or None,
                extra={"raw_score": score, "guid": item.get("guid")},
            )
