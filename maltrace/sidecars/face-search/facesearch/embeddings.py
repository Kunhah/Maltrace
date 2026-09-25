"""Face detection + 512-d embedding.

Powered by InsightFace (ArcFace, `buffalo_l`). Embeddings are L2-normalised
so cosine similarity collapses to a dot product, which `utils.cosine_similarity`
exploits when scoring candidates in bulk.
"""

from __future__ import annotations

import logging
import threading
from dataclasses import dataclass, field
from typing import Optional

import numpy as np
from PIL import Image

from .config import Settings
from .utils import pil_to_rgb_array

log = logging.getLogger("facesearch.embed")


EMBEDDING_DIM = 512


@dataclass(slots=True)
class FaceEncoding:
    embedding: np.ndarray            # L2-normalised 512-d vector
    bbox: tuple[int, int, int, int]  # (x1, y1, x2, y2)
    det_score: float
    landmarks: Optional[np.ndarray] = field(default=None)


class FaceEmbedder:
    """Thread-safe wrapper around the InsightFace pipeline.

    The model is loaded lazily on the first call. ONNX sessions are not
    fork-safe; callers must instantiate one embedder per process.
    """

    backend = "insightface"
    embedding_dim = EMBEDDING_DIM

    def __init__(self, settings: Settings) -> None:
        self.settings = settings
        self._lock = threading.Lock()
        self._app = None

    def _load(self) -> None:
        with self._lock:
            if self._app is not None:
                return
            try:
                from insightface.app import FaceAnalysis  # type: ignore
            except ImportError as exc:
                raise RuntimeError(
                    "InsightFace is not installed. Run `pip install -r "
                    "requirements.txt` inside the face-search sidecar venv."
                ) from exc
            providers = list(self.settings.providers)
            app = FaceAnalysis(
                name=self.settings.model_name,
                providers=providers,
                allowed_modules=["detection", "recognition"],
            )
            app.prepare(
                ctx_id=0 if "CUDAExecutionProvider" in providers else -1,
                det_size=(640, 640),
            )
            self._app = app
            log.info("loaded InsightFace model=%s", self.settings.model_name)

    def encode(self, image: Image.Image) -> list[FaceEncoding]:
        """Return all face encodings in the image, strongest face first."""
        if self._app is None:
            self._load()
        rgb = pil_to_rgb_array(image)
        # InsightFace expects BGR.
        bgr = rgb[:, :, ::-1].copy()
        try:
            faces = self._app.get(bgr)  # type: ignore[union-attr]
        except Exception as exc:  # noqa: BLE001
            log.warning("insightface inference error: %s", exc)
            return []
        out: list[FaceEncoding] = []
        for f in faces:
            emb = f.normed_embedding.astype(np.float32, copy=False)
            x1, y1, x2, y2 = (int(v) for v in f.bbox.tolist())
            out.append(
                FaceEncoding(
                    embedding=emb,
                    bbox=(x1, y1, x2, y2),
                    det_score=float(f.det_score),
                    landmarks=getattr(f, "kps", None),
                )
            )
        out.sort(key=lambda f: f.det_score, reverse=True)
        return out

    def encode_primary(self, image: Image.Image) -> Optional[FaceEncoding]:
        """Return the highest-confidence face above det_score_min, else None."""
        faces = self.encode(image)
        for face in faces:
            if face.det_score >= self.settings.det_score_min:
                return face
        return faces[0] if faces else None
