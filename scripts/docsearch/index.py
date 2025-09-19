from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import List

from . import common


def build_argparser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Индексатор документации Codex с EmbeddingGemma")
    parser.add_argument(
        "docs_root",
        type=Path,
        nargs="?",
        default=Path("docs"),
        help="Каталог с документацией",
    )
    parser.add_argument(
        "--output",
        type=Path,
        default=common.DEFAULT_INDEX_PATH,
        help="Путь до файла индекса (JSONL)",
    )
    parser.add_argument(
        "--min-chars",
        type=int,
        default=160,
        help="Минимальное количество символов в чанке",
    )
    parser.add_argument(
        "--max-chars",
        type=int,
        default=1200,
        help="Максимальное количество символов в чанке",
    )
    parser.add_argument(
        "--batch-size",
        type=int,
        default=16,
        help="Размер батча при инференсе",
    )
    parser.add_argument(
        "--recursive",
        action="store_true",
        help="Рекурсивно обходить подкаталоги",
    )
    common.add_common_arguments(parser)
    return parser


def main(argv: List[str] | None = None) -> int:
    parser = build_argparser()
    args = parser.parse_args(argv)

    docs_root: Path = args.docs_root
    if not docs_root.exists():
        parser.error(f"Каталог {docs_root} не найден")

    model = common.load_model(args.model_path, device=args.device)

    records = []
    total_chunks = 0
    for path in common.iter_documents(docs_root, recursive=args.recursive):
        text = path.read_text(encoding="utf-8", errors="ignore")
        chunks = common.chunk_text(
            text,
            min_chars=args.min_chars,
            max_chars=args.max_chars,
        )
        if not chunks:
            continue
        embeddings = common.encode_documents(
            model,
            chunks,
            batch_size=args.batch_size,
            truncate_dim=args.truncate_dim,
        )
        for idx, (chunk_text, embedding) in enumerate(zip(chunks, embeddings), start=1):
            record = {
                "id": f"{path.as_posix()}::chunk-{idx}",
                "path": path.as_posix(),
                "chunk_id": idx,
                "text": chunk_text,
                "embedding": embedding.astype(float).tolist() if embedding is not None else None,
            }
            records.append(record)
        total_chunks += len(chunks)

    if not records:
        parser.error("Не найдено подходящих документов для индексации")

    common.save_index(args.output, records)
    print(
        json.dumps(
            {
                "docs": len(records),
                "chunks": total_chunks,
                "output": args.output.as_posix(),
                "truncate_dim": args.truncate_dim,
            },
            ensure_ascii=False,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
