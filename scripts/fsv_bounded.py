#!/usr/bin/env python3
"""Bounded-output helper for Calyx FSV artifacts.

Full FSV bytes stay in files under the FSV root. This helper prints only file
metadata and selected scalar JSON leaves so Codex terminals never need to stream
large readback JSON, catalogs, or one-line JSON logs.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
import sys
from pathlib import Path
from typing import Any


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def emit_file(prefix: str, path: Path) -> None:
    if not path.exists():
        print(f"{prefix}_path={path}")
        print(f"{prefix}_exists=false")
        return
    print(f"{prefix}_path={path}")
    print(f"{prefix}_bytes={path.stat().st_size}")
    print(f"{prefix}_sha256={sha256_file(path)}")


def parse_field(raw: str) -> tuple[str, str]:
    if "=" not in raw:
        raise argparse.ArgumentTypeError("field must be NAME=JSON_PATH")
    name, path = raw.split("=", 1)
    if not name:
        raise argparse.ArgumentTypeError("field name must not be empty")
    if not path:
        raise argparse.ArgumentTypeError("field path must not be empty")
    return name, path


def path_tokens(path: str) -> list[str | int]:
    text = path.strip()
    if text.startswith("$"):
        text = text[1:]
    if text.startswith("."):
        text = text[1:]
    tokens: list[str | int] = []
    token = ""
    idx = 0
    while idx < len(text):
        ch = text[idx]
        if ch == ".":
            if token:
                tokens.append(token)
                token = ""
            idx += 1
            continue
        if ch == "[":
            if token:
                tokens.append(token)
                token = ""
            end = text.find("]", idx)
            if end == -1:
                raise ValueError(f"unterminated index in {path!r}")
            tokens.append(int(text[idx + 1 : end]))
            idx = end + 1
            continue
        token += ch
        idx += 1
    if token:
        tokens.append(token)
    return tokens


def select_json(value: Any, path: str) -> Any:
    current = value
    for token in path_tokens(path):
        if isinstance(token, int):
            if not isinstance(current, list):
                raise KeyError(path)
            current = current[token]
        else:
            if not isinstance(current, dict):
                raise KeyError(path)
            current = current[token]
    return current


def emit_fields(path: Path, fields: list[tuple[str, str]]) -> None:
    if not fields:
        return
    with path.open("r", encoding="utf-8") as handle:
        value = json.load(handle)
    for name, field_path in fields:
        selected = select_json(value, field_path)
        if isinstance(selected, (dict, list)):
            raise SystemExit(
                f"CALYX_FSV_BOUNDED_FIELD_NOT_SCALAR: {name}={field_path}"
            )
        print(f"{name}={json.dumps(selected, sort_keys=True)}")


def summarize(args: argparse.Namespace) -> int:
    artifact = Path(args.artifact)
    emit_file("artifact", artifact)
    if not artifact.exists():
        return 2
    emit_fields(artifact, args.field)
    return 0


def capture(args: argparse.Namespace) -> int:
    if not args.command:
        raise SystemExit("CALYX_FSV_BOUNDED_COMMAND_MISSING")
    stdout_path = Path(args.stdout)
    stderr_path = Path(args.stderr)
    stdout_path.parent.mkdir(parents=True, exist_ok=True)
    stderr_path.parent.mkdir(parents=True, exist_ok=True)
    with stdout_path.open("wb") as stdout, stderr_path.open("wb") as stderr:
        proc = subprocess.run(args.command, stdout=stdout, stderr=stderr)
    print(f"exit_status={proc.returncode}")
    emit_file("stdout", stdout_path)
    emit_file("stderr", stderr_path)
    if args.field and stdout_path.exists() and stdout_path.stat().st_size > 0:
        emit_fields(stdout_path, args.field)
    return proc.returncode


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="bounded FSV output helper")
    sub = parser.add_subparsers(dest="cmd", required=True)

    summarize_cmd = sub.add_parser("summarize", help="summarize an artifact file")
    summarize_cmd.add_argument("artifact")
    summarize_cmd.add_argument(
        "--field", action="append", default=[], type=parse_field, metavar="NAME=PATH"
    )
    summarize_cmd.set_defaults(func=summarize)

    capture_cmd = sub.add_parser("capture", help="capture a command without tee")
    capture_cmd.add_argument("--stdout", required=True)
    capture_cmd.add_argument("--stderr", required=True)
    capture_cmd.add_argument(
        "--field", action="append", default=[], type=parse_field, metavar="NAME=PATH"
    )
    capture_cmd.add_argument("command", nargs=argparse.REMAINDER)
    capture_cmd.set_defaults(func=capture)
    return parser


def main() -> int:
    parser = build_parser()
    args = parser.parse_args()
    if getattr(args, "command", None) and args.command[0:1] == ["--"]:
        args.command = args.command[1:]
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main())
