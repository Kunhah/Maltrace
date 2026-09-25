"""facesearch — reverse face search across multiple engines.

Pipeline: query image -> detect & embed face -> reverse-image search across
backends -> download candidates -> embed faces in candidates -> score by
cosine similarity -> rank and report.
"""

from .config import Settings, load_settings
from .embeddings import FaceEmbedder, FaceEncoding
from .pipeline import SearchPipeline, MatchResult

__all__ = [
    "Settings",
    "load_settings",
    "FaceEmbedder",
    "FaceEncoding",
    "SearchPipeline",
    "MatchResult",
]

__version__ = "0.1.0"
