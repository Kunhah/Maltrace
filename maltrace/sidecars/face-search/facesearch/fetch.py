"""HTTP fetching with size limits, retries, and on-disk caching."""

from __future__ import annotations

import asyncio
import logging
import mimetypes
from pathlib import Path
from typing import Optional

import httpx

from .config import Settings
from .utils import ensure_dir, sha1_bytes

log = logging.getLogger("facesearch.fetch")


IMAGE_CONTENT_TYPES = {
    "image/jpeg", "image/jpg", "image/png", "image/webp",
    "image/bmp", "image/gif", "image/tiff", "image/avif",
}


class Fetcher:
    """A reusable async HTTP client + content-addressable cache.

    Use as `async with Fetcher(settings) as f: ...`. Closing the context
    closes the underlying HTTPX client.
    """

    def __init__(self, settings: Settings) -> None:
        self.settings = settings
        self.cache_dir = ensure_dir(settings.cache_dir / "downloads")
        headers = {
            "User-Agent": settings.user_agent,
            "Accept-Language": "en-US,en;q=0.9",
        }
        transport = httpx.AsyncHTTPTransport(retries=2, http2=True)
        proxy = settings.http_proxy
        self.client = httpx.AsyncClient(
            headers=headers,
            follow_redirects=True,
            timeout=settings.http_timeout,
            transport=transport,
            proxy=proxy,
        )
        self._download_sem = asyncio.Semaphore(settings.download_concurrency)

    async def __aenter__(self) -> "Fetcher":
        return self

    async def __aexit__(self, exc_type, exc, tb) -> None:
        await self.client.aclose()

    async def get_bytes(self, url: str, *, max_bytes: Optional[int] = None) -> bytes:
        """Stream a URL, abort if it exceeds max_bytes."""
        limit = max_bytes or self.settings.max_download_bytes
        chunks: list[bytes] = []
        total = 0
        async with self.client.stream("GET", url) as resp:
            resp.raise_for_status()
            async for chunk in resp.aiter_bytes():
                total += len(chunk)
                if total > limit:
                    raise httpx.ReadError(f"response exceeded {limit} bytes")
                chunks.append(chunk)
        return b"".join(chunks)

    async def download_image(self, url: str) -> Optional["DownloadedImage"]:
        """Download an image with caching. Returns None on failure."""
        async with self._download_sem:
            cache_key = sha1_bytes(url.encode("utf-8"))
            cache_path = self.cache_dir / f"{cache_key}.bin"
            meta_path = self.cache_dir / f"{cache_key}.meta"
            if cache_path.exists() and meta_path.exists():
                data = cache_path.read_bytes()
                content_type = meta_path.read_text(encoding="utf-8").strip() or None
                return DownloadedImage(url=url, data=data, content_type=content_type,
                                       path=cache_path, from_cache=True)
            try:
                resp = await self.client.get(url)
                resp.raise_for_status()
            except Exception as exc:  # noqa: BLE001
                log.debug("download failed url=%s err=%s", url, exc)
                return None
            content_type = (resp.headers.get("content-type") or "").split(";")[0].strip().lower()
            # Some CDNs send octet-stream; let PIL decide later if extension hints image.
            if content_type and content_type not in IMAGE_CONTENT_TYPES:
                guessed = mimetypes.guess_type(url)[0]
                if not (guessed and guessed in IMAGE_CONTENT_TYPES):
                    log.debug("skip non-image url=%s ct=%s", url, content_type)
                    return None
            data = resp.content
            if len(data) > self.settings.max_download_bytes:
                log.debug("oversized url=%s bytes=%d", url, len(data))
                return None
            cache_path.write_bytes(data)
            meta_path.write_text(content_type or "", encoding="utf-8")
            return DownloadedImage(url=url, data=data, content_type=content_type,
                                   path=cache_path, from_cache=False)


class DownloadedImage:
    __slots__ = ("url", "data", "content_type", "path", "from_cache")

    def __init__(self, url: str, data: bytes, content_type: str | None,
                 path: Path, from_cache: bool) -> None:
        self.url = url
        self.data = data
        self.content_type = content_type
        self.path = path
        self.from_cache = from_cache
