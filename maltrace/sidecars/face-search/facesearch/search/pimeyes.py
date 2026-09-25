"""PimEyes adapter — STUB.

PimEyes has no public API and their terms of service prohibit automated
scraping. This file exists as a pluggable slot: if you have a private API
endpoint (some enterprise contracts grant one), fill in `_upload` and
`_poll` below and the rest of the pipeline will pick the backend up.

Without configuration this backend reports unavailable and is skipped.
"""

from __future__ import annotations

import logging

from ..config import Settings
from ..fetch import Fetcher
from .base import CandidateHit

log = logging.getLogger("facesearch.search.pimeyes")


class PimEyesBackend:
    name = "pimeyes"

    def __init__(self, settings: Settings) -> None:
        self.settings = settings
        self.api_key = settings.pimeyes_api_key
        self.endpoint = None  # set by an enterprise integration

    def is_available(self) -> bool:
        return bool(self.api_key and self.endpoint)

    async def search(self, image_bytes: bytes, fetcher: Fetcher,
                     *, limit: int) -> list[CandidateHit]:
        if not self.is_available():
            log.info("pimeyes not configured; skipping")
            return []
        # Wire up your private endpoint here. See facecheck.py for the
        # upload + poll pattern to copy.
        raise NotImplementedError("Configure a PimEyes endpoint to use this backend.")
