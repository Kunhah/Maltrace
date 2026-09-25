"""Registry of search backends."""

from __future__ import annotations

from typing import Callable

from ..config import Settings
from .base import CandidateHit, SearchBackend
from .bing import BingBackend
from .facecheck import FaceCheckBackend
from .google_lens import GoogleLensBackend
from .pimeyes import PimEyesBackend
from .yandex import YandexBackend


# Map backend name -> factory(settings) -> SearchBackend.
_FACTORIES: dict[str, Callable[[Settings], SearchBackend]] = {
    "yandex": lambda s: YandexBackend(),
    "bing": lambda s: BingBackend(s),
    "google_lens": lambda s: GoogleLensBackend(),
    "facecheck": lambda s: FaceCheckBackend(s),
    "pimeyes": lambda s: PimEyesBackend(s),
}


def available_backend_names() -> list[str]:
    return list(_FACTORIES.keys())


def build_backends(names: list[str], settings: Settings) -> list[SearchBackend]:
    out: list[SearchBackend] = []
    for n in names:
        factory = _FACTORIES.get(n)
        if factory is None:
            raise ValueError(f"Unknown backend: {n}. "
                             f"Choose from {sorted(_FACTORIES)}.")
        out.append(factory(settings))
    return out


__all__ = [
    "CandidateHit",
    "SearchBackend",
    "available_backend_names",
    "build_backends",
]
