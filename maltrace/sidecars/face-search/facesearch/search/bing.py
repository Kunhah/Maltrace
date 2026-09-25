"""Bing Visual Search backend.

Prefers the official Azure Cognitive Services Visual Search API when
`BING_VISUAL_SEARCH_KEY` is set. Without a key, falls back to scraping
bing.com/images. The scraping path is best-effort and breaks regularly.
"""

from __future__ import annotations

import json
import logging
import re
from typing import Iterable

from bs4 import BeautifulSoup

from ..config import Settings
from ..fetch import Fetcher
from .base import CandidateHit

log = logging.getLogger("facesearch.search.bing")

VISUAL_SEARCH_API = "https://api.bing.microsoft.com/v7.0/images/visualsearch"
SCRAPE_UPLOAD_URL = "https://www.bing.com/images/search?view=detailv2&iss=sbiupload"


class BingBackend:
    name = "bing"

    def __init__(self, settings: Settings) -> None:
        self.settings = settings
        self.api_key = settings.bing_api_key

    def is_available(self) -> bool:
        return True

    async def search(self, image_bytes: bytes, fetcher: Fetcher,
                     *, limit: int) -> list[CandidateHit]:
        if self.api_key:
            try:
                return await self._search_api(image_bytes, fetcher, limit)
            except Exception as exc:  # noqa: BLE001
                log.warning("bing API failed (%s); falling back to scrape", exc)
        try:
            return await self._search_scrape(image_bytes, fetcher, limit)
        except Exception as exc:  # noqa: BLE001
            log.warning("bing scrape failed: %s", exc)
            return []

    # --- API path -------------------------------------------------------

    async def _search_api(self, image_bytes: bytes, fetcher: Fetcher,
                          limit: int) -> list[CandidateHit]:
        headers = {"Ocp-Apim-Subscription-Key": self.api_key or ""}
        files = {"image": ("query.jpg", image_bytes, "application/octet-stream")}
        # The "knowledgeRequest" filters can be added later; default is fine.
        resp = await fetcher.client.post(VISUAL_SEARCH_API, headers=headers, files=files)
        resp.raise_for_status()
        payload = resp.json()
        return list(self._extract_api(payload, limit))

    def _extract_api(self, payload: dict, limit: int) -> Iterable[CandidateHit]:
        emitted = 0
        for tag in payload.get("tags") or []:
            for action in tag.get("actions") or []:
                action_type = action.get("actionType", "")
                values = (action.get("data") or {}).get("value") or []
                for v in values:
                    image_url = v.get("contentUrl") or v.get("thumbnailUrl")
                    if not image_url:
                        continue
                    yield CandidateHit(
                        image_url=image_url,
                        source_url=v.get("hostPageUrl") or v.get("webSearchUrl"),
                        backend=self.name,
                        title=v.get("name"),
                        width=v.get("width"),
                        height=v.get("height"),
                        extra={"action": action_type},
                    )
                    emitted += 1
                    if emitted >= limit:
                        return

    # --- scrape path ----------------------------------------------------

    async def _search_scrape(self, image_bytes: bytes, fetcher: Fetcher,
                             limit: int) -> list[CandidateHit]:
        files = {
            "imgurl": (None, ""),
            "cbir": (None, "sbi"),
            "imageBin": (None, _b64(image_bytes)),
        }
        # Bing's scraping endpoint takes a base64-encoded image in `imageBin`
        # and returns an HTML redirect/landing page.
        resp = await fetcher.client.post(SCRAPE_UPLOAD_URL, data=files)
        resp.raise_for_status()
        html = resp.text
        return list(self._extract_scrape(html, limit))

    def _extract_scrape(self, html: str, limit: int) -> Iterable[CandidateHit]:
        soup = BeautifulSoup(html, "html.parser")
        out_count = 0
        for node in soup.select("a.iusc"):
            raw = node.get("m")
            if not raw:
                continue
            try:
                obj = json.loads(raw)
            except json.JSONDecodeError:
                continue
            image_url = obj.get("murl") or obj.get("turl")
            if not image_url:
                continue
            source_url = obj.get("purl")
            yield CandidateHit(
                image_url=image_url,
                source_url=source_url,
                backend=self.name,
                title=obj.get("t"),
            )
            out_count += 1
            if out_count >= limit:
                return


def _b64(data: bytes) -> str:
    import base64
    return base64.b64encode(data).decode("ascii")
