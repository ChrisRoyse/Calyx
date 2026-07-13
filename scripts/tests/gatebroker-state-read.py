#!/usr/bin/env python3
"""Read a gatebroker SQLite source of truth without mutating it."""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
import os
import sqlite3
import stat
import sys
from pathlib import Path
from typing import Any
from urllib.parse import quote


class ReadbackError(RuntimeError):
    pass


def file_readback(path: Path) -> dict[str, Any]:
    value = path.stat(follow_symlinks=False)
    if not stat.S_ISREG(value.st_mode):
        raise ReadbackError(f"source of truth is not a regular file: {path}")
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return {
        "path": str(path),
        "device": value.st_dev,
        "inode": value.st_ino,
        "uid": value.st_uid,
        "gid": value.st_gid,
        "mode": f"{stat.S_IMODE(value.st_mode):04o}",
        "size": value.st_size,
        "mtime_ns": value.st_mtime_ns,
        "sha256": digest.hexdigest(),
    }


def json_value(value: Any) -> Any:
    if isinstance(value, bytes):
        return {
            "type": "blob",
            "length": len(value),
            "sha256": hashlib.sha256(value).hexdigest(),
            "base64": base64.b64encode(value).decode("ascii"),
        }
    if value is None or isinstance(value, (bool, int, float, str)):
        return value
    raise ReadbackError(f"unsupported SQLite value type: {type(value).__name__}")


def identifier(raw: str) -> str:
    return '"' + raw.replace('"', '""') + '"'


def rows_as_objects(cursor: sqlite3.Cursor, rows: list[tuple[Any, ...]]) -> list[dict[str, Any]]:
    names = [description[0] for description in cursor.description or ()]
    return [
        {name: json_value(value) for name, value in zip(names, row, strict=True)}
        for row in rows
    ]


def matching_rows(
    connection: sqlite3.Connection,
    table: str,
    columns: list[str],
    tokens: list[str],
    maximum: int,
) -> list[dict[str, Any]]:
    if not columns or not tokens:
        return []
    predicates: list[str] = []
    parameters: list[str] = []
    for token in tokens:
        token_predicates = [
            f"instr(CAST({identifier(column)} AS TEXT), ?) > 0" for column in columns
        ]
        predicates.append("(" + " OR ".join(token_predicates) + ")")
        parameters.extend([token] * len(columns))
    sql = (
        f"SELECT * FROM {identifier(table)} WHERE "
        + " OR ".join(predicates)
        + " LIMIT ?"
    )
    parameters.append(str(maximum))
    cursor = connection.execute(sql, parameters)
    return rows_as_objects(cursor, cursor.fetchall())


def table_readback(
    connection: sqlite3.Connection,
    name: str,
    schema: str | None,
    tokens: list[str],
    maximum: int,
) -> dict[str, Any]:
    table_id = identifier(name)
    columns_cursor = connection.execute(f"PRAGMA table_xinfo({table_id})")
    column_rows = columns_cursor.fetchall()
    columns = [str(row[1]) for row in column_rows]
    count = int(connection.execute(f"SELECT COUNT(*) FROM {table_id}").fetchone()[0])
    sample_cursor = connection.execute(f"SELECT * FROM {table_id} LIMIT ?", (maximum,))
    sample = rows_as_objects(sample_cursor, sample_cursor.fetchall())
    return {
        "name": name,
        "schema": schema,
        "row_count": count,
        "sample_truncated": count > maximum,
        "columns": [
            {
                "cid": row[0],
                "name": row[1],
                "type": row[2],
                "not_null": bool(row[3]),
                "default": row[4],
                "primary_key_position": row[5],
                "hidden": row[6],
            }
            for row in column_rows
        ],
        "sample_rows": sample,
        "matching_rows": matching_rows(connection, name, columns, tokens, maximum),
    }


def database_readback(database: Path, tokens: list[str], maximum: int) -> dict[str, Any]:
    resolved = database.resolve(strict=True)
    before = file_readback(resolved)
    uri = f"file:{quote(str(resolved), safe='/')}?mode=ro"
    connection = sqlite3.connect(uri, uri=True, timeout=5.0)
    try:
        connection.execute("PRAGMA query_only = ON")
        connection.execute("PRAGMA busy_timeout = 5000")
        connection.execute("BEGIN")
        quick_check = [row[0] for row in connection.execute("PRAGMA quick_check")]
        if quick_check != ["ok"]:
            raise ReadbackError(f"SQLite quick_check failed: {quick_check!r}")
        schema_rows = connection.execute(
            "SELECT type, name, sql FROM sqlite_schema "
            "WHERE type IN ('table', 'view') ORDER BY type, name"
        ).fetchall()
        tables = [
            table_readback(connection, name, schema, tokens, maximum)
            for object_type, name, schema in schema_rows
            if object_type == "table"
        ]
        views = [
            {"name": name, "schema": schema}
            for object_type, name, schema in schema_rows
            if object_type == "view"
        ]
        user_version = int(connection.execute("PRAGMA user_version").fetchone()[0])
        schema_version = int(connection.execute("PRAGMA schema_version").fetchone()[0])
        data_version = int(connection.execute("PRAGMA data_version").fetchone()[0])
        connection.rollback()
    finally:
        connection.close()

    physical_files: list[dict[str, Any]] = []
    for candidate in (resolved, Path(f"{resolved}-wal"), Path(f"{resolved}-shm")):
        if candidate.exists():
            physical_files.append(file_readback(candidate))
    after = file_readback(resolved)
    return {
        "source_of_truth": "sqlite-read-only-transaction",
        "database": str(resolved),
        "database_before": before,
        "database_after": after,
        "physical_files": physical_files,
        "quick_check": quick_check,
        "user_version": user_version,
        "schema_version": schema_version,
        "data_version": data_version,
        "match_tokens": tokens,
        "max_rows_per_table": maximum,
        "tables": tables,
        "views": views,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--database", required=True, type=Path)
    parser.add_argument("--match", action="append", default=[])
    parser.add_argument("--max-rows", type=int, default=200)
    args = parser.parse_args()
    if args.max_rows < 1 or args.max_rows > 10_000:
        parser.error("--max-rows must be between 1 and 10000")
    try:
        readback = database_readback(args.database, args.match, args.max_rows)
    except (OSError, sqlite3.Error, ReadbackError) as error:
        print(
            json.dumps(
                {
                    "status": "error",
                    "error": type(error).__name__,
                    "detail": str(error),
                    "database": str(args.database),
                },
                sort_keys=True,
            ),
            file=sys.stderr,
        )
        return 1
    print(json.dumps(readback, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
