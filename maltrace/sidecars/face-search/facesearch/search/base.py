"""SearchBackend interface and result types."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Optional, Protocol, runtime_checkable

from ..fetch import Fetcher


@dataclass(slots=True)
class CandidateHit:
    """A single result from a reverse-image search engine.

    `image_url` is the URL of an actual image we should download and embed.
    `source_url` is the page that contained it (for citing the result).
    `backend` is the originator. `score` is the backend's own confidence
    (0..1) when available, else None.
    """
    image_url: str
    source_url: Optional[str] = None
    backend: str = ""
    score: Optional[float] = None
    title: Optional[str] = None
    width: Optional[int] = None
    height: Optional[int] = None
    extra: dict = field(default_factory=dict)


@runtime_checkable
class SearchBackend(Protocol):
    name: str

    def is_available(self) -> bool:
        ...

    async def search(self, image_bytes: bytes, fetcher: Fetcher,
                     *, limit: int) -> list[CandidateHit]:
        ...
