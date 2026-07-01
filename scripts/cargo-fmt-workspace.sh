#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat >&2 <<'USAGE'
Usage: bash scripts/cargo-fmt-workspace.sh [--check|--write]

Formats every Cargo workspace package one package at a time. This avoids the
native Windows command-line expansion failure that can happen with
`cargo fmt --all` in long worktree paths while still failing on the first
unformatted package.
USAGE
}

mode="check"
if [[ $# -gt 1 ]]; then
  usage
  exit 2
fi
if [[ $# -eq 1 ]]; then
  case "$1" in
    --check) mode="check" ;;
    --write) mode="write" ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "cargo-fmt-workspace: ERROR unknown argument '$1'" >&2
      usage
      exit 2
      ;;
  esac
fi

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

if ! command -v cargo >/dev/null 2>&1; then
  echo "cargo-fmt-workspace: ERROR cargo not found on PATH" >&2
  exit 127
fi
if ! command -v python3 >/dev/null 2>&1; then
  echo "cargo-fmt-workspace: ERROR python3 not found on PATH; required to parse cargo metadata" >&2
  exit 127
fi

mapfile -t packages < <(
  cargo metadata --no-deps --format-version 1 |
    python3 -c 'import json, sys
metadata = json.load(sys.stdin)
packages = {package["id"]: package["name"] for package in metadata.get("packages", [])}
members = metadata.get("workspace_members", [])
missing = [member for member in members if member not in packages]
if missing:
    sys.stderr.write("cargo-fmt-workspace: ERROR cargo metadata omitted workspace members: " + ", ".join(missing) + "\n")
    sys.exit(1)
names = [packages[member] for member in members]
if not names:
    sys.stderr.write("cargo-fmt-workspace: ERROR cargo metadata returned zero workspace packages\n")
    sys.exit(1)
for name in names:
    print(name)'
)

echo "cargo-fmt-workspace: mode=$mode package_count=${#packages[@]}" >&2
for package in "${packages[@]}"; do
  if [[ "$mode" == "check" ]]; then
    echo "cargo-fmt-workspace: package=$package command=cargo fmt -p $package -- --check" >&2
    cargo fmt -p "$package" -- --check
  else
    echo "cargo-fmt-workspace: package=$package command=cargo fmt -p $package" >&2
    cargo fmt -p "$package"
  fi
done
