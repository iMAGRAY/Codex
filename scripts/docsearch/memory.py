from __future__ import annotations

import argparse
import json
import sys
from dataclasses import dataclass
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Iterable, List, Optional
from uuid import uuid4

import numpy as np
from rapidfuzz import fuzz

from . import common

DEFAULT_MEMORY_PATH = Path.home() / ".codex" / "memory" / "memory.jsonl"
DEFAULT_MAX_RECORDS = 500
IMPORTANCE_LEVELS = {"low", "medium", "high"}


@dataclass
class MemoryRecord:
    id: str
    text: str
    tags: list[str]
    source: Optional[str]
    importance: Optional[str]
    pinned: bool
    created_at: str
    updated_at: str
    expires_at: Optional[str]
    embedding: Optional[list[float]]

    def to_json(self) -> dict:
        data = {
            "id": self.id,
            "text": self.text,
            "tags": self.tags,
            "source": self.source,
            "importance": self.importance,
            "pinned": self.pinned,
            "created_at": self.created_at,
            "updated_at": self.updated_at,
            "expires_at": self.expires_at,
            "embedding": self.embedding,
        }
        return data

    @staticmethod
    def from_json(payload: dict) -> "MemoryRecord":
        return MemoryRecord(
            id=payload["id"],
            text=payload.get("text", ""),
            tags=sorted(set(payload.get("tags", []))),
            source=payload.get("source"),
            importance=payload.get("importance"),
            pinned=bool(payload.get("pinned", False)),
            created_at=payload.get("created_at", ""),
            updated_at=payload.get("updated_at", ""),
            expires_at=payload.get("expires_at"),
            embedding=payload.get("embedding"),
        )


def load_records(path: Path) -> list[MemoryRecord]:
    if not path.exists():
        return []
    records: list[MemoryRecord] = []
    with path.open("r", encoding="utf-8") as fp:
        for line in fp:
            line = line.strip()
            if not line:
                continue
            try:
                payload = json.loads(line)
            except json.JSONDecodeError:
                continue
            records.append(MemoryRecord.from_json(payload))
    return records


def save_records(path: Path, records: Iterable[MemoryRecord]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", encoding="utf-8") as fp:
        for record in records:
            fp.write(json.dumps(record.to_json(), ensure_ascii=False) + "\n")


def encode_entry(model, text: str, truncate_dim: Optional[int]) -> Optional[list[float]]:
    embeddings = common.encode_documents(
        model,
        [text],
        batch_size=1,
        truncate_dim=truncate_dim,
    )
    if embeddings and embeddings[0] is not None:
        return embeddings[0].astype(float).tolist()
    return None


def add_parser(subparsers: argparse._SubParsersAction[argparse.ArgumentParser]) -> None:
    remember = subparsers.add_parser("remember", help="Сохранить запись в память")
    remember.add_argument("text", nargs="?", help="Текст записи")
    remember.add_argument(
        "--file",
        type=Path,
        help="Прочитать текст записи из файла",
    )
    remember.add_argument(
        "--tag",
        action="append",
        dest="tags",
        default=[],
        help="Теги для группировки (можно указывать несколько раз)",
    )
    remember.add_argument(
        "--source",
        help="Свободная отметка источника записи",
    )
    remember.add_argument(
        "--importance",
        help="Уровень важности (low, medium, high)",
    )
    remember.add_argument(
        "--ttl",
        help="Срок жизни записи (например '24h', '7d', '3600s')",
    )
    remember.add_argument(
        "--pinned",
        action="store_true",
        help="Пометить запись как закреплённую (не удалять при автоматической очистке)",
    )
    remember.add_argument(
        "--replace",
        help="ID существующей записи, которую нужно заменить",
    )
    remember.add_argument(
        "--memory",
        type=Path,
        default=DEFAULT_MEMORY_PATH,
        help="Путь до файла памяти (JSONL)",
    )
    common.add_common_arguments(remember)

    forget = subparsers.add_parser("forget", help="Удалить записи из памяти")
    forget.add_argument(
        "--id",
        action="append",
        dest="ids",
        default=[],
        help="ID записей для удаления",
    )
    forget.add_argument(
        "--tag",
        action="append",
        dest="tags",
        default=[],
        help="Удалить записи с указанными тегами",
    )
    forget.add_argument(
        "--memory",
        type=Path,
        default=DEFAULT_MEMORY_PATH,
        help="Путь до файла памяти",
    )

    list_cmd = subparsers.add_parser("list", help="Показать сохранённые записи")
    list_cmd.add_argument(
        "--tag",
        action="append",
        dest="tags",
        default=[],
        help="Фильтр по тегам",
    )
    list_cmd.add_argument(
        "--memory",
        type=Path,
        default=DEFAULT_MEMORY_PATH,
        help="Путь до файла памяти",
    )

    prune_cmd = subparsers.add_parser("prune", help="Очистить память по критериям")
    prune_cmd.add_argument(
        "--memory",
        type=Path,
        default=DEFAULT_MEMORY_PATH,
        help="Путь до файла памяти",
    )
    prune_cmd.add_argument(
        "--max-records",
        type=int,
        help="Максимальное количество записей (старые будут удалены кроме закреплённых)",
    )
    prune_cmd.add_argument(
        "--older-than",
        help="Удалить записи старше указанного ISO-времени",
    )
    prune_cmd.add_argument(
        "--drop-expired",
        action="store_true",
        help="Удалить истёкшие записи (по умолчанию true)",
    )
    prune_cmd.add_argument(
        "--keep-expired",
        action="store_true",
        help="Сохранить истёкшие записи (переключает поведение --drop-expired)",
    )
    prune_cmd.add_argument(
        "--dedupe",
        action="store_true",
        help="Удалить дубликаты (по совпадению текста без регистра)",
    )

    search = subparsers.add_parser("search", help="Поиск по памяти")
    search.add_argument("query", help="Запрос")
    search.add_argument(
        "--top-k",
        type=int,
        default=5,
        help="Количество результатов",
    )
    search.add_argument(
        "--tag",
        action="append",
        dest="tags",
        default=[],
        help="Фильтр по тегам",
    )
    search.add_argument(
        "--show-text",
        action="store_true",
        help="Выводить текст записей",
    )
    search.add_argument(
        "--memory",
        type=Path,
        default=DEFAULT_MEMORY_PATH,
        help="Путь до файла памяти",
    )
    common.add_common_arguments(search)


def build_argparser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Локальная память Codex на базе EmbeddingGemma")
    subparsers = parser.add_subparsers(dest="command", required=True)
    add_parser(subparsers)
    return parser


def now_iso() -> str:
    return datetime.now(timezone.utc).isoformat()


def coalesce_text(args: argparse.Namespace) -> str:
    if args.text:
        return args.text.strip()
    if args.file:
        return args.file.read_text(encoding="utf-8").strip()
    data = sys.stdin.read().strip()
    if data:
        return data
    raise SystemExit("Нет текста для сохранения. Используйте позиционный аргумент, --file или передайте данные через stdin.")


def parse_ttl(ttl: Optional[str]) -> Optional[datetime]:
    if not ttl:
        return None

    value = ttl.strip().lower()
    if value in {"0", "none"}:
        return None

    units = {
        "s": "seconds",
        "sec": "seconds",
        "secs": "seconds",
        "second": "seconds",
        "seconds": "seconds",
        "m": "minutes",
        "min": "minutes",
        "mins": "minutes",
        "minute": "minutes",
        "minutes": "minutes",
        "h": "hours",
        "hr": "hours",
        "hrs": "hours",
        "hour": "hours",
        "hours": "hours",
        "d": "days",
        "day": "days",
        "days": "days",
        "w": "weeks",
        "week": "weeks",
        "weeks": "weeks",
    }

    for suffix, field in sorted(units.items(), key=lambda item: -len(item[0])):
        if value.endswith(suffix):
            number = value[: -len(suffix)].strip()
            try:
                magnitude = float(number)
            except ValueError as exc:
                raise SystemExit(f"Не удалось разобрать TTL '{ttl}'") from exc
            delta_kwargs = {field: magnitude}
            return datetime.now(timezone.utc) + timedelta(**delta_kwargs)

    try:
        magnitude = float(value)
        return datetime.now(timezone.utc) + timedelta(seconds=magnitude)
    except ValueError as exc:
        raise SystemExit(
            "TTL должен быть числом секунд или иметь суффикс s/m/h/d/w (например '24h')."
        ) from exc


def remember_command(args: argparse.Namespace) -> None:
    records = load_records(args.memory)
    content = coalesce_text(args)
    tags = sorted(set(args.tags))

    importance = None
    if args.importance:
        normalized = args.importance.lower()
        if normalized not in IMPORTANCE_LEVELS:
            raise SystemExit(
                f"Недопустимый уровень важности '{args.importance}'. Допустимо: {sorted(IMPORTANCE_LEVELS)}"
            )
        importance = normalized

    expires_at_dt = parse_ttl(args.ttl)
    expires_at = expires_at_dt.isoformat() if expires_at_dt else None

    model = common.load_model(args.model_path, device=args.device)
    embedding = encode_entry(model, content, args.truncate_dim)

    timestamp = now_iso()
    target_id = args.replace
    output_id = None
    if target_id:
        updated = False
        for record in records:
            if record.id == target_id:
                record.text = content
                record.tags = tags
                record.source = args.source or record.source
                record.importance = importance or record.importance
                record.pinned = args.pinned or record.pinned
                record.updated_at = timestamp
                record.embedding = embedding
                record.expires_at = expires_at if expires_at else record.expires_at
                updated = True
                output_id = record.id
                break
        if not updated:
            raise SystemExit(f"Запись с id={target_id} не найдена")
    else:
        record = MemoryRecord(
            id=str(uuid4()),
            text=content,
            tags=tags,
            source=args.source,
            importance=importance,
            pinned=args.pinned,
            created_at=timestamp,
            updated_at=timestamp,
            expires_at=expires_at,
            embedding=embedding,
        )
        records.append(record)
        output_id = record.id

    save_records(args.memory, records)
    payload = {
        "memory": args.memory.as_posix(),
        "id": output_id,
        "tags": tags,
        "importance": importance,
        "pinned": args.pinned,
        "updated_at": timestamp,
        "expires_at": expires_at,
    }
    print(json.dumps(payload, ensure_ascii=False))


def forget_command(args: argparse.Namespace) -> None:
    if not args.ids and not args.tags:
        raise SystemExit("Укажите --id или --tag для удаления записей")

    records = load_records(args.memory)
    remaining: list[MemoryRecord] = []
    removed: list[str] = []

    tag_set = set(args.tags)
    id_set = set(args.ids)

    for record in records:
        should_remove = False
        if record.id in id_set:
            should_remove = True
        elif tag_set and tag_set.intersection(record.tags):
            should_remove = True
        if should_remove:
            removed.append(record.id)
        else:
            remaining.append(record)

    save_records(args.memory, remaining)
    payload = {
        "memory": args.memory.as_posix(),
        "removed": removed,
        "remaining": len(remaining),
    }
    print(json.dumps(payload, ensure_ascii=False))


def list_command(args: argparse.Namespace) -> None:
    records = load_records(args.memory)
    tag_filter = set(args.tags)
    if tag_filter:
        records = [rec for rec in records if tag_filter.issubset(set(rec.tags))]
    prune_expired = []
    now = datetime.now(timezone.utc)
    for rec in records:
        if rec.expires_at:
            try:
                expire_dt = datetime.fromisoformat(rec.expires_at)
            except ValueError:
                prune_expired.append(rec)
                continue
            if expire_dt < now:
                prune_expired.append(rec)
    if prune_expired:
        records = [rec for rec in records if rec not in prune_expired]
        save_records(args.memory, records)

    output = [rec.to_json() for rec in sorted(records, key=lambda r: r.updated_at, reverse=True)]
    print(json.dumps({"records": output}, ensure_ascii=False, indent=2))


def search_command(args: argparse.Namespace) -> None:
    records = load_records(args.memory)
    tag_filter = set(args.tags)
    if tag_filter:
        records = [rec for rec in records if tag_filter.issubset(set(rec.tags))]

    if not records:
        print(json.dumps({"results": []}, ensure_ascii=False, indent=2))
        return

    model = common.load_model(args.model_path, device=args.device)
    query_vector = common.encode_query(
        model,
        args.query,
        truncate_dim=args.truncate_dim,
    )

    with_embeddings = [rec for rec in records if rec.embedding]
    results = []
    if query_vector is not None and with_embeddings:
        matrix = np.vstack([np.asarray(rec.embedding, dtype=np.float32) for rec in with_embeddings])
        scores = np.dot(matrix, query_vector)
        top_k = min(args.top_k, scores.shape[0])
        indices = np.argsort(scores)[::-1][:top_k]
        for idx in indices:
            rec = with_embeddings[idx]
            item = {
                "id": rec.id,
                "tags": rec.tags,
                "source": rec.source,
                "importance": rec.importance,
                "pinned": rec.pinned,
                "score": float(scores[idx]),
            }
            if args.show_text:
                item["text"] = rec.text
            results.append(item)

    if not results:
        results = lexical_fallback(args.query, records, args.top_k, args.show_text)

    print(json.dumps({"results": results}, ensure_ascii=False, indent=2))


def lexical_fallback(query: str, records: List[MemoryRecord], top_k: int, show_text: bool) -> list[dict]:
    scored: list[tuple[float, MemoryRecord]] = []
    for record in records:
        if not record.text:
            continue
        score = fuzz.token_set_ratio(query, record.text) / 100.0
        if record.importance == "high":
            score += 0.05
        elif record.importance == "low":
            score -= 0.05
        scored.append((score, record))
    scored.sort(key=lambda item: item[0], reverse=True)
    results = []
    for score, record in scored[:top_k]:
        entry = {
            "id": record.id,
            "tags": record.tags,
            "source": record.source,
            "importance": record.importance,
            "pinned": record.pinned,
            "score": score,
        }
        if show_text:
            entry["text"] = record.text
        results.append(entry)
    return results


def prune_command(args: argparse.Namespace) -> None:
    records = load_records(args.memory)
    before_count = len(records)

    if args.keep_expired:
        drop_expired = False
    elif args.drop_expired:
        drop_expired = True
    else:
        drop_expired = True
    now = datetime.now(timezone.utc)

    pruned: list[MemoryRecord] = []
    seen_text: set[str] = set()

    def normalized_text(text: str) -> str:
        return " ".join(text.lower().split())

    for record in records:
        # skip pinned from expiration logic
        if drop_expired and not record.pinned and record.expires_at:
            try:
                expires_dt = datetime.fromisoformat(record.expires_at)
            except ValueError:
                # treat invalid timestamp as expired
                continue
            if expires_dt < now:
                continue
        if args.older_than and not record.pinned:
            try:
                boundary = datetime.fromisoformat(args.older_than)
            except ValueError as exc:
                raise SystemExit("older-than должен быть в ISO-формате, например 2025-01-01T00:00:00+00:00") from exc
            try:
                updated = datetime.fromisoformat(record.updated_at)
            except ValueError:
                continue
            if updated < boundary:
                continue
        if args.dedupe:
            key = normalized_text(record.text)
            if key in seen_text and not record.pinned:
                continue
            seen_text.add(key)
        pruned.append(record)

    # enforce max-records (exclude pinned first)
    max_records = args.max_records or DEFAULT_MAX_RECORDS
    if max_records > 0 and len(pruned) > max_records:
        pinned = [rec for rec in pruned if rec.pinned]
        mutable = sorted(
            [rec for rec in pruned if not rec.pinned],
            key=lambda rec: rec.updated_at,
            reverse=True,
        )
        allowed = max_records - len(pinned)
        if allowed < 0:
            allowed = 0
        mutable = mutable[:allowed]
        pruned = pinned + mutable

    save_records(args.memory, pruned)
    payload = {
        "memory": args.memory.as_posix(),
        "before": before_count,
        "after": len(pruned),
        "removed": max(before_count - len(pruned), 0),
    }
    print(json.dumps(payload, ensure_ascii=False))


def main(argv: Optional[list[str]] = None) -> int:
    parser = build_argparser()
    args = parser.parse_args(argv)

    if args.command == "remember":
        remember_command(args)
    elif args.command == "forget":
        forget_command(args)
    elif args.command == "list":
        list_command(args)
    elif args.command == "search":
        search_command(args)
    elif args.command == "prune":
        prune_command(args)
    else:
        parser.error("Неизвестная команда")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
