"""Runtime configuration loaded from env vars and CLI overrides."""

from __future__ import annotations

import os
from dataclasses import dataclass, field
from pathlib import Path
from typing import Iterable


DEFAULT_USER_AGENT = (
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 "
    "(KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36"
)


@dataclass(slots=True)
class Settings:
    # Embedding model. "buffalo_l" is InsightFace's 512-d ArcFace bundle.
    model_name: str = "buffalo_l"
    # Provider preference for ONNX runtime.
    providers: tuple[str, ...] = ("CUDAExecutionProvider", "CPUExecutionProvider")
    # Minimum detection confidence to consider a face.
    det_score_min: float = 0.50
    # Minimum cosine similarity to keep a candidate (broad filter).
    candidate_threshold: float = 0.35
    # Cosine similarity at which we label something a "match".
    match_threshold: float = 0.55
    # Maximum candidates per backend.
    per_backend_limit: int = 40
    # Maximum total candidates carried through to download.
    total_candidate_limit: int = 200
    # Concurrent downloads.
    download_concurrency: int = 16
    # HTTP timeout (seconds) for general fetches.
    http_timeout: float = 20.0
    # Max bytes per downloaded candidate (12 MB).
    max_download_bytes: int = 12 * 1024 * 1024
    # Cache directory for downloaded candidates and search responses.
    cache_dir: Path = field(default_factory=lambda: Path.home() / ".cache" / "facesearch")
    # User-Agent for outbound HTTP.
    user_agent: str = DEFAULT_USER_AGENT
    # API keys (read from env if not provided).
    bing_api_key: str | None = None
    facecheck_api_key: str | None = None
    pimeyes_api_key: str | None = None
    # Optional Tor / proxy support.
    http_proxy: str | None = None
    # Backends enabled by default. CLI can override.
    default_backends: tuple[str, ...] = ("yandex",)

    def env_overlay(self) -> "Settings":
        """Return a copy with API keys / proxy filled in from env when blank."""
        return Settings(
            model_name=self.model_name,
            providers=self.providers,
            det_score_min=self.det_score_min,
            candidate_threshold=self.candidate_threshold,
            match_threshold=self.match_threshold,
            per_backend_limit=self.per_backend_limit,
            total_candidate_limit=self.total_candidate_limit,
            download_concurrency=self.download_concurrency,
            http_timeout=self.http_timeout,
            max_download_bytes=self.max_download_bytes,
            cache_dir=self.cache_dir,
            user_agent=self.user_agent,
            bing_api_key=self.bing_api_key or os.environ.get("BING_VISUAL_SEARCH_KEY"),
            facecheck_api_key=self.facecheck_api_key or os.environ.get("FACECHECK_API_KEY"),
            pimeyes_api_key=self.pimeyes_api_key or os.environ.get("PIMEYES_API_KEY"),
            http_proxy=self.http_proxy or os.environ.get("FACESEARCH_HTTP_PROXY"),
            default_backends=self.default_backends,
        )

    def with_backends(self, backends: Iterable[str]) -> "Settings":
        return Settings(**{**self.__dict__, "default_backends": tuple(backends)})


def load_settings() -> Settings:
    """Build a Settings from env defaults."""
    return Settings().env_overlay()
