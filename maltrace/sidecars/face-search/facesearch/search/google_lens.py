"""Google Lens reverse-image search via Playwright.

There is no public Lens API. The implementation drives a headless Chromium:
  1. Open https://www.google.com/?hl=en
  2. Click the camera icon to open the Lens upload UI
  3. Upload the query image
  4. Wait for results to load and switch to "Find image source"
  5. Scrape the result anchors

This backend is OFF by default. Enable via `--backend google_lens` and
ensure `playwright install chromium` has been run. Bot-detection makes it
fragile; if it stops working the first thing to check is whether the
selectors below still match the current Lens DOM.
"""

from __future__ import annotations

import asyncio
import logging
import tempfile
from pathlib import Path
from typing import Iterable, Optional

from ..fetch import Fetcher
from .base import CandidateHit

log = logging.getLogger("facesearch.search.google_lens")


class GoogleLensBackend:
    name = "google_lens"

    def __init__(self) -> None:
        self._playwright_ok: Optional[bool] = None

    def is_available(self) -> bool:
        if self._playwright_ok is None:
            try:
                import playwright.async_api  # noqa: F401
                self._playwright_ok = True
            except ImportError:
                self._playwright_ok = False
        return bool(self._playwright_ok)

    async def search(self, image_bytes: bytes, fetcher: Fetcher,
                     *, limit: int) -> list[CandidateHit]:
        if not self.is_available():
            log.warning("playwright not installed; skipping google_lens")
            return []
        try:
            return await self._run(image_bytes, limit)
        except Exception as exc:  # noqa: BLE001
            log.warning("google_lens failed: %s", exc)
            return []

    async def _run(self, image_bytes: bytes, limit: int) -> list[CandidateHit]:
        from playwright.async_api import async_playwright  # type: ignore

        with tempfile.NamedTemporaryFile(suffix=".jpg", delete=False) as tmp:
            tmp.write(image_bytes)
            tmp_path = Path(tmp.name)
        try:
            async with async_playwright() as p:
                browser = await p.chromium.launch(headless=True, args=[
                    "--disable-blink-features=AutomationControlled",
                ])
                context = await browser.new_context(
                    viewport={"width": 1280, "height": 900},
                    user_agent=(
                        "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 "
                        "(KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36"
                    ),
                )
                page = await context.new_page()
                hits: list[CandidateHit] = []
                try:
                    await page.goto("https://lens.google.com/", wait_until="domcontentloaded")
                    # The Lens landing page has a file input; if not, fall back
                    # to images.google.com which always renders one.
                    file_input = await page.query_selector("input[type=file]")
                    if file_input is None:
                        await page.goto("https://images.google.com/?hl=en", wait_until="domcontentloaded")
                        # Click the camera icon to open the upload dialog.
                        await page.click("[aria-label*='Search by image' i], [role='button']:has-text('camera')")
                        await page.wait_for_selector("input[type=file]", timeout=8000)
                        file_input = await page.query_selector("input[type=file]")
                    if file_input is None:
                        log.warning("google_lens: no file input found")
                        return []
                    await file_input.set_input_files(str(tmp_path))
                    # Lens redirects to a results URL once the upload is processed.
                    await page.wait_for_url("**lens.google.com/search**", timeout=20000)
                    await page.wait_for_load_state("networkidle", timeout=15000)
                    # Switch to "Find image source" if available — that lists
                    # the pages containing the image.
                    try:
                        await page.click("text=/Find image source/i", timeout=3000)
                        await page.wait_for_load_state("networkidle", timeout=10000)
                    except Exception:
                        pass
                    hits.extend(await self._scrape_results(page, limit))
                finally:
                    await context.close()
                    await browser.close()
                return hits
        finally:
            tmp_path.unlink(missing_ok=True)

    async def _scrape_results(self, page, limit: int) -> Iterable[CandidateHit]:
        # Pages-containing-the-image cards have a thumbnail <img> inside an <a>.
        # Lens DOM changes frequently — be permissive.
        anchors = await page.query_selector_all("a[href^='http']")
        seen: set[str] = set()
        out: list[CandidateHit] = []
        for a in anchors:
            href = await a.get_attribute("href")
            if not href or href.startswith("https://www.google.com/"):
                continue
            if "google.com/search" in href or "google.com/imgres" in href:
                continue
            img = await a.query_selector("img")
            img_src = None
            if img is not None:
                img_src = await img.get_attribute("src") or await img.get_attribute("data-src")
            if not img_src:
                continue
            if href in seen:
                continue
            seen.add(href)
            title = (await a.inner_text())[:200] if a else None
            out.append(CandidateHit(
                image_url=img_src,
                source_url=href,
                backend=self.name,
                title=title or None,
            ))
            if len(out) >= limit:
                break
        return out
