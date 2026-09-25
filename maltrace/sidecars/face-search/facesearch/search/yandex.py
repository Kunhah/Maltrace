"""Yandex Images reverse-image search.

Yandex is the best free reverse-face search engine; it does not require an
API key and returns both pages containing the image and visually similar
images. Their HTML layout drifts every few months, so the extractor tries
several patterns and degrades gracefully.

Flow:
  1. POST the image to https://yandex.com/images/search?rpt=imageview&format=json...
     The response is a JSON envelope whose first block contains the canonical
     CBIR URL with a `cbir_id` and `cbir_page=similar` query.
  2. Follow that URL to get an HTML SERP.
  3. Extract per-result image + source-page URLs from inline JSON
     (`data-bem` / `data-state`) and from anchor tags as a fallback.
"""

from __future__ import annotations

import asyncio
import json
import logging
import re
from typing import Iterable, Optional
from urllib.parse import unquote, urlparse, parse_qs, urljoin

import httpx
from bs4 import BeautifulSoup

from ..fetch import Fetcher
from .base import CandidateHit

log = logging.getLogger("facesearch.search.yandex")


JSON_UPLOAD_URL = (
    "https://yandex.com/images/search"
    "?rpt=imageview&format=json"
    "&request=%7B%22blocks%22%3A%5B%7B%22block%22%3A%22b-page_type_search-by-image__link%22%7D%5D%7D"
)

HTML_HEADERS = {
    "Accept": "text/html,application/xhtml+xml",
    "Accept-Language": "en-US,en;q=0.9",
    "Referer": "https://yandex.com/images/",
}

JSON_HEADERS = {
    "Accept": "application/json, text/plain, */*",
    "Referer": "https://yandex.com/images/",
}


class YandexBackend:
    name = "yandex"

    def __init__(self) -> None:
        self.enabled = True

    def is_available(self) -> bool:
        return True

    async def search(self, image_bytes: bytes, fetcher: Fetcher,
                     *, limit: int) -> list[CandidateHit]:
        client = fetcher.client
        try:
            cbir_url = await self._upload(client, image_bytes)
        except Exception as exc:  # noqa: BLE001
            log.warning("yandex upload failed: %s", exc)
            return []
        if not cbir_url:
            log.warning("yandex returned no cbir url")
            return []

        # Pull both the "sites" and "similar" tabs and merge.
        sites_url = self._with_page(cbir_url, "sites")
        similar_url = self._with_page(cbir_url, "similar")
        try:
            sites_html, similar_html = await asyncio.gather(
                self._fetch_html(client, sites_url),
                self._fetch_html(client, similar_url),
                return_exceptions=True,
            )
        except Exception as exc:  # noqa: BLE001
            log.warning("yandex serp fetch failed: %s", exc)
            return []

        hits: list[CandidateHit] = []
        seen: set[str] = set()
        for html_doc in (sites_html, similar_html):
            if isinstance(html_doc, Exception) or not html_doc:
                continue
            for hit in self._extract(html_doc):
                key = hit.image_url
                if key in seen:
                    continue
                seen.add(key)
                hits.append(hit)
                if len(hits) >= limit:
                    return hits
        return hits

    # --- upload ---------------------------------------------------------

    async def _upload(self, client: httpx.AsyncClient, image_bytes: bytes) -> Optional[str]:
        files = {"upfile": ("query.jpg", image_bytes, "image/jpeg")}
        resp = await client.post(JSON_UPLOAD_URL, headers=JSON_HEADERS, files=files)
        resp.raise_for_status()
        # Response shape: {"blocks":[{"params":{"url":"/images/search?...cbir_id=..."}}]}
        try:
            data = resp.json()
        except json.JSONDecodeError:
            # Sometimes returns the search page directly.
            return self._cbir_from_html(resp.text)
        blocks = data.get("blocks") or []
        if not blocks:
            return None
        params = (blocks[0] or {}).get("params") or {}
        rel = params.get("url")
        if not rel:
            return None
        return urljoin("https://yandex.com/", rel)

    @staticmethod
    def _cbir_from_html(html_doc: str) -> Optional[str]:
        m = re.search(r'cbir_id=([0-9A-Za-z\-_/]+)', html_doc)
        if not m:
            return None
        return f"https://yandex.com/images/search?cbir_id={m.group(1)}&rpt=imageview"

    @staticmethod
    def _with_page(cbir_url: str, page: str) -> str:
        parsed = urlparse(cbir_url)
        qs = parse_qs(parsed.query)
        qs["cbir_page"] = [page]
        qs["rpt"] = ["imageview"]
        new_query = "&".join(f"{k}={v[0]}" for k, v in qs.items())
        return f"{parsed.scheme}://{parsed.netloc}{parsed.path}?{new_query}"

    async def _fetch_html(self, client: httpx.AsyncClient, url: str) -> str:
        resp = await client.get(url, headers=HTML_HEADERS)
        resp.raise_for_status()
        return resp.text

    # --- extraction -----------------------------------------------------

    def _extract(self, html_doc: str) -> Iterable[CandidateHit]:
        # Strategy 1: inline JSON states. The serp embeds a large JSON in
        # `<div class="serp-item" data-bem='{...}'>`. data-bem contains
        # `serp-item.thumb.url`, `serp-item.preview[0].url`, `serp-item.snippet`.
        yield from self._extract_data_bem(html_doc)
        # Strategy 2: anchors. The "sites with this image" tab has links to
        # source pages with thumbnails alongside.
        yield from self._extract_anchors(html_doc)

    def _extract_data_bem(self, html_doc: str) -> Iterable[CandidateHit]:
        soup = BeautifulSoup(html_doc, "html.parser")
        for node in soup.select("[data-bem]"):
            raw = node.get("data-bem")
            if not raw or "serp-item" not in raw:
                continue
            try:
                obj = json.loads(raw)
            except json.JSONDecodeError:
                continue
            item = obj.get("serp-item") or {}
            previews = item.get("preview") or []
            snippet = item.get("snippet") or {}
            img_url = None
            for p in previews:
                if isinstance(p, dict) and p.get("url"):
                    img_url = p["url"]
                    break
            if not img_url:
                img_url = item.get("thumb", {}).get("url") if isinstance(item.get("thumb"), dict) else None
            if not img_url:
                continue
            img_url = self._normalize_url(img_url)
            source_url = item.get("href") or snippet.get("url")
            title = snippet.get("title")
            yield CandidateHit(
                image_url=img_url,
                source_url=source_url,
                backend=self.name,
                title=title,
            )

    def _extract_anchors(self, html_doc: str) -> Iterable[CandidateHit]:
        soup = BeautifulSoup(html_doc, "html.parser")
        # "Sites with this image" cards: a.CbirSites-ItemThumb, a.CbirSites-ItemTitle
        for card in soup.select(".CbirSites-Item, .Sites-Item, .cbir-sites-item"):
            link = card.find("a", href=True)
            img = card.find("img")
            if not link or not img:
                continue
            img_src = img.get("src") or img.get("data-src")
            if not img_src:
                continue
            yield CandidateHit(
                image_url=self._normalize_url(img_src),
                source_url=link["href"],
                backend=self.name,
                title=(link.get_text(strip=True) or None),
            )

    @staticmethod
    def _normalize_url(url: str) -> str:
        url = url.strip()
        if url.startswith("//"):
            return "https:" + url
        if url.startswith("/"):
            return urljoin("https://yandex.com/", url)
        # Yandex sometimes URL-encodes thumbnails as "https%3A//..."
        if url.startswith("https%3A") or url.startswith("http%3A"):
            return unquote(url)
        return url
