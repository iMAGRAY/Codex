from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import List

import numpy as np
from rapidfuzz import fuzz

from . import common


def build_argparser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Семантический поиск по документации Codex")
    parser.add_argument("query", help="Пользовательский запрос")
    parser.add_argument(
        "--index",
        type=Path,
        default=common.DEFAULT_INDEX_PATH,
        help="Файл индекса, созданный docsearch index",
    )
    parser.add_argument(
        "--top-k",
        type=int,
        default=5,
        help="Количество совпадений в выдаче",
    )
    parser.add_argument(
        "--show-text",
        action="store_true",
        help="Выводить текстовые чанки в результатах",
    )
    common.add_common_arguments(parser)
    return parser


def main(argv: List[str] | None = None) -> int:
    parser = build_argparser()
    args = parser.parse_args(argv)

    if not args.index.exists():
        parser.error(
            f"Индекс {args.index} не найден. Сначала выполните `codex doc index`."
        )

    model = common.load_model(args.model_path, device=args.device)
    query_embedding = common.encode_query(
        model,
        args.query,
        truncate_dim=args.truncate_dim,
    )

    matrix, embedding_records, metadata = common.load_index(args.index)
    results = []

    if matrix.size > 0 and query_embedding is not None:
        scores = np.dot(matrix, query_embedding)
        top_k = min(args.top_k, scores.shape[0])
        best_indices = np.argsort(scores)[::-1][:top_k]

        for idx in best_indices:
            entry = embedding_records[idx]
            result = {
                "id": entry["id"],
                "path": entry["path"],
                "chunk_id": entry["chunk_id"],
                "score": float(scores[idx]),
            }
            if args.show_text:
                result["text"] = entry["text"]
            results.append(result)

    if not results:
        results = fallback_lexical(args.query, metadata, args.top_k, args.show_text)

    print(json.dumps({"results": results}, ensure_ascii=False, indent=2))
    return 0


def fallback_lexical(query: str, records: list[dict], top_k: int, show_text: bool) -> list[dict]:
    scored: list[tuple[float, dict]] = []
    for entry in records:
        chunk = entry.get("text", "")
        if not chunk:
            continue
        score = fuzz.token_set_ratio(query, chunk) / 100.0
        scored.append((score, entry))
    scored.sort(key=lambda item: item[0], reverse=True)
    results = []
    for score, entry in scored[:top_k]:
        result = {
            "id": entry["id"],
            "path": entry["path"],
            "chunk_id": entry["chunk_id"],
            "score": score,
        }
        if show_text:
            result["text"] = entry.get("text", "")
        results.append(result)
    return results


if __name__ == "__main__":
    raise SystemExit(main())
