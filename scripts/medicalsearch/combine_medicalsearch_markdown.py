#!/usr/bin/env python3
"""Combine the medicalsearch Markdown findings into one ordered file."""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
from pathlib import Path
import re
from typing import Iterable


GENERATED_NAME_PATTERNS = (
    re.compile(r"^medicalsearch_combined(?:[_-].*)?\.md$", re.IGNORECASE),
    re.compile(r"^combined_medicalsearch(?:[_-].*)?\.md$", re.IGNORECASE),
)


def repo_root() -> Path:
    return Path(__file__).resolve().parents[2]


def natural_doc_key(path: Path) -> tuple[int, int, str]:
    """Sort numbered docs by numeric prefix, then everything else by name."""
    if path.name.lower() == "index.md":
        return (2, 0, path.name.lower())
    match = re.match(r"^(\d+)_", path.name)
    if match:
        return (1, int(match.group(1)), path.name.lower())
    return (0, 0, path.name.lower())


def is_generated_output(path: Path, output_path: Path) -> bool:
    if path.resolve() == output_path.resolve():
        return True
    return any(pattern.match(path.name) for pattern in GENERATED_NAME_PATTERNS)


def iter_markdown_files(
    source_dir: Path,
    output_path: Path,
    include_template: bool,
) -> Iterable[Path]:
    files = []
    for path in source_dir.glob("*.md"):
        if is_generated_output(path, output_path):
            continue
        if not include_template and path.name == "_TEMPLATE.md":
            continue
        files.append(path)
    yield from sorted(files, key=natural_doc_key)


def heading_for(path: Path) -> str:
    return f"## {path.name}\n"


def read_markdown(path: Path) -> str:
    text = path.read_text(encoding="utf-8")
    return text.rstrip() + "\n"


def combine(source_dir: Path, output_path: Path, include_template: bool) -> int:
    source_dir = source_dir.resolve()
    output_path = output_path.resolve()
    files = list(iter_markdown_files(source_dir, output_path, include_template))
    if not files:
        raise SystemExit(f"no markdown files found in {source_dir}")

    output_path.parent.mkdir(parents=True, exist_ok=True)
    generated_at = datetime.now(timezone.utc).isoformat(timespec="seconds")
    lines = [
        "# Combined Medical Search Findings Log\n",
        "\n",
        f"Generated from `{source_dir}` at `{generated_at}`.\n",
        f"Files combined: `{len(files)}`.\n",
        "\n",
        "This is a generated convenience file. Source-of-truth association state "
        "must still be read from Calyx/Aster, not from this Markdown export.\n",
        "\n",
        "## Table Of Contents\n",
        "\n",
    ]
    for path in files:
        anchor = path.name.lower().replace(".", "").replace("_", "-")
        lines.append(f"- [{path.name}](#{anchor})\n")

    lines.append("\n---\n\n")
    for path in files:
        lines.append(heading_for(path))
        lines.append("\n")
        lines.append(read_markdown(path))
        lines.append("\n---\n\n")

    output_path.write_text("".join(lines), encoding="utf-8", newline="\n")
    return len(files)


def parse_args() -> argparse.Namespace:
    root = repo_root()
    default_source = root / "docs" / "medicalsearch"
    default_output = default_source / "medicalsearch_combined.md"

    parser = argparse.ArgumentParser(
        description="Combine docs/medicalsearch Markdown files into one ordered Markdown file.",
    )
    parser.add_argument(
        "--source-dir",
        type=Path,
        default=default_source,
        help=f"Directory containing Markdown files. Default: {default_source}",
    )
    parser.add_argument(
        "--output",
        type=Path,
        default=default_output,
        help=f"Combined Markdown output path. Default: {default_output}",
    )
    parser.add_argument(
        "--include-template",
        action="store_true",
        help="Include _TEMPLATE.md. By default it is skipped.",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    count = combine(args.source_dir, args.output, args.include_template)
    print(f"wrote {args.output} from {count} markdown files")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
