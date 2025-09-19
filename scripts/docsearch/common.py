from __future__ import annotations

import argparse
import json
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable, Iterator, List, Sequence

import numpy as np
from typing import Optional

DEFAULT_MODEL_PATH = Path(__file__).resolve().parents[2] / "embeddinggemma-300m"
DEFAULT_INDEX_PATH = Path("docs/.docsearch/index.jsonl")
SUPPORTED_SUFFIXES = {".md", ".markdown", ".txt"}


@dataclass
class DocChunk:
    path: Path
    chunk_id: int
    text: str

    @property
    def identifier(self) -> str:
        return f"{self.path.as_posix()}::chunk-{self.chunk_id}"


def iter_documents(root: Path, recursive: bool = True) -> Iterator[Path]:
    if recursive:
        for path in root.rglob("*"):
            if path.suffix.lower() in SUPPORTED_SUFFIXES and path.is_file():
                yield path
    else:
        for path in root.glob("*"):
            if path.suffix.lower() in SUPPORTED_SUFFIXES and path.is_file():
                yield path


def chunk_text(text: str, *, min_chars: int = 120, max_chars: int = 1200) -> List[str]:
    """Return normalized chunks split on double newlines.

    Chunks shorter than ``min_chars`` are dropped; chunks longer than
    ``max_chars`` are split greedily on sentence endings.
    """

    import re

    raw_chunks = [segment.strip() for segment in text.split("\n\n")]
    chunks: List[str] = []
    for segment in raw_chunks:
        normalized = " ".join(segment.split())
        if len(normalized) < min_chars:
            continue
        if len(normalized) <= max_chars:
            chunks.append(normalized)
            continue
        # Greedy sentence split to respect max_chars.
        sentences = re.split(r"(?<=[.!?])\s+", normalized)
        buffer: List[str] = []
        for sentence in sentences:
            candidate = " ".join(buffer + [sentence]) if buffer else sentence
            if len(candidate) <= max_chars:
                buffer.append(sentence)
            else:
                if buffer:
                    chunks.append(" ".join(buffer))
                buffer = [sentence]
        if buffer:
            joined = " ".join(buffer)
            if len(joined) >= min_chars:
                chunks.append(joined)
    return chunks


def load_model(model_path: Path, device: str = "cpu") -> Optional[object]:
    try:
        from sentence_transformers import SentenceTransformer
    except (ImportError, ModuleNotFoundError):
        return None

    try:
        model = SentenceTransformer(str(model_path), device=device)
        return model
    except Exception:
        return None


def encode_documents(
    model,
    documents: Sequence[str],
    *,
    batch_size: int = 16,
    truncate_dim: int | None = 512,
) -> np.ndarray:
    if model is None:
        return [None for _ in documents]

    kwargs = {
        "batch_size": batch_size,
        "normalize_embeddings": True,
        "convert_to_numpy": True,
    }
    if truncate_dim is not None:
        kwargs["truncate_dim"] = truncate_dim
    return model.encode_document(documents, **kwargs)


def encode_query(
    model,
    query: str,
    *,
    truncate_dim: int | None = 512,
) -> np.ndarray | None:
    if model is None:
        return None

    kwargs = {
        "normalize_embeddings": True,
        "convert_to_numpy": True,
    }
    if truncate_dim is not None:
        kwargs["truncate_dim"] = truncate_dim
    return model.encode_query(query, **kwargs)


def save_index(path: Path, records: Iterable[dict]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", encoding="utf-8") as fp:
        for record in records:
            fp.write(json.dumps(record, ensure_ascii=False) + "\n")


def load_index(path: Path) -> tuple[np.ndarray, List[dict], List[dict]]:
    embeddings: List[np.ndarray] = []
    embedding_records: List[dict] = []
    metadata: List[dict] = []
    with path.open("r", encoding="utf-8") as fp:
        for line in fp:
            entry = json.loads(line)
            emb = entry.get("embedding")
            if emb is not None:
                embeddings.append(np.asarray(emb, dtype=np.float32))
                embedding_records.append(entry)
            metadata.append(entry)
    matrix = np.vstack(embeddings) if embeddings else np.zeros((0,))
    return matrix, embedding_records, metadata


def add_common_arguments(parser: argparse.ArgumentParser) -> None:
    parser.add_argument(
        "--model-path",
        type=Path,
        default=DEFAULT_MODEL_PATH,
        help="Путь к локальной папке EmbeddingGemma-300M",
    )
    parser.add_argument(
        "--device",
        default="cpu",
        choices=["cpu", "cuda", "auto"],
        help="Устройство инференса sentence-transformers",
    )
    parser.add_argument(
        "--truncate-dim",
        type=int,
        default=512,
        choices=[768, 512, 256, 128],
        help="Размерность эмбеддинга после усечения Matryoshka",
    )
