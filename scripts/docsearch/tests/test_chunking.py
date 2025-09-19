from __future__ import annotations

from scripts.docsearch import common


def test_chunk_text_basic() -> None:
    text = "Первый абзац.\n\nВторой абзац, достаточно длинный, чтобы пройти порог." * 2
    chunks = common.chunk_text(text, min_chars=10, max_chars=60)
    assert chunks, "Ожидается минимум один чанк"
    for chunk in chunks:
        assert len(chunk) >= 10
        assert len(chunk) <= 60
