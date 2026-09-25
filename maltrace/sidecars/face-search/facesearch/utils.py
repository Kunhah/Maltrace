"""Pure helpers: hashing, similarity, image I/O."""

from __future__ import annotations

import hashlib
import io
import logging
from pathlib import Path
from typing import Iterable

import numpy as np
from PIL import Image, ImageOps

log = logging.getLogger("facesearch")


def cosine_similarity(a: np.ndarray, b: np.ndarray) -> float:
    """Cosine similarity for already-L2-normalized vectors == dot product.

    Falls back to a safe normalized form if either input is not unit-norm.
    """
    a = np.asarray(a, dtype=np.float32).ravel()
    b = np.asarray(b, dtype=np.float32).ravel()
    if a.size == 0 or b.size == 0 or a.size != b.size:
        return 0.0
    na = float(np.linalg.norm(a))
    nb = float(np.linalg.norm(b))
    if na < 1e-8 or nb < 1e-8:
        return 0.0
    if abs(na - 1.0) < 1e-3 and abs(nb - 1.0) < 1e-3:
        return float(np.dot(a, b))
    return float(np.dot(a, b) / (na * nb))


def confidence_label(score: float, match_threshold: float) -> str:
    if score >= max(match_threshold, 0.62):
        return "strong"
    if score >= match_threshold:
        return "likely"
    if score >= match_threshold - 0.15:
        return "possible"
    return "weak"


def sha1_bytes(data: bytes) -> str:
    return hashlib.sha1(data).hexdigest()


def perceptual_hash(image: Image.Image, size: int = 16) -> str:
    """A cheap difference-hash. Used only for dedup, not similarity."""
    small = image.convert("L").resize((size + 1, size), Image.Resampling.LANCZOS)
    pixels = np.asarray(small, dtype=np.int32)
    diff = pixels[:, 1:] > pixels[:, :-1]
    bits = diff.flatten()
    out = 0
    for bit in bits:
        out = (out << 1) | int(bit)
    return f"{out:0{(size * size) // 4}x}"


def load_image_bytes(data: bytes) -> Image.Image:
    """Decode bytes into a PIL image, normalizing orientation and mode."""
    img = Image.open(io.BytesIO(data))
    img.load()
    img = ImageOps.exif_transpose(img)
    if img.mode not in ("RGB", "RGBA"):
        img = img.convert("RGB")
    elif img.mode == "RGBA":
        bg = Image.new("RGB", img.size, (0, 0, 0))
        bg.paste(img, mask=img.split()[-1])
        img = bg
    return img


def pil_to_rgb_array(img: Image.Image) -> np.ndarray:
    if img.mode != "RGB":
        img = img.convert("RGB")
    return np.asarray(img, dtype=np.uint8)


def ensure_dir(path: Path) -> Path:
    path.mkdir(parents=True, exist_ok=True)
    return path


def chunked(seq: Iterable, n: int):
    buf = []
    for item in seq:
        buf.append(item)
        if len(buf) >= n:
            yield buf
            buf = []
    if buf:
        yield buf
