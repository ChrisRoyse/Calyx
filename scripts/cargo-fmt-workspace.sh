#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat >&2 <<'USAGE'
Usage: bash scripts/cargo-fmt-workspace.sh [--check|--write] [--repo-root <path>]

Formats every Cargo workspace package one package at a time. This avoids the
native Windows command-line expansion failure that can happen with
`cargo fmt --all` in long worktree paths while still failing on the first
unformatted package.
USAGE
}

mode="check"
repo_root_arg=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --check) mode="check" ;;
    --write) mode="write" ;;
    --repo-root)
      if [[ $# -lt 2 ]]; then
        echo "cargo-fmt-workspace: ERROR --repo-root requires a path" >&2
        usage
        exit 2
      fi
      repo_root_arg="$2"
      shift
      ;;
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
  shift
done

if [[ -n "$repo_root_arg" ]]; then
  if [[ ! -d "$repo_root_arg" ]]; then
    echo "cargo-fmt-workspace: ERROR repo root does not exist or is not a directory: $repo_root_arg" >&2
    exit 2
  fi
  repo_root="$(cd "$repo_root_arg" && pwd)"
else
  repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fi
if [[ ! -f "$repo_root/Cargo.toml" ]]; then
  echo "cargo-fmt-workspace: ERROR repo root is not a Cargo workspace root (missing Cargo.toml): $repo_root" >&2
  exit 2
fi
cd "$repo_root"

if [[ -f "$HOME/.cargo/env" ]]; then
  # Direct script invocations from WSL are non-login shells; source rustup's
  # environment so the same command works outside scripts/check.sh.
  # shellcheck source=/dev/null
  source "$HOME/.cargo/env"
fi

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
    print(name)' |
    tr -d '\r'
)

echo "cargo-fmt-workspace: repo=$repo_root mode=$mode package_count=${#packages[@]}" >&2
for package in "${packages[@]}"; do
  if [[ "$mode" == "check" ]]; then
    echo "cargo-fmt-workspace: package=$package command=cargo fmt -p $package -- --check" >&2
    cargo fmt -p "$package" -- --check
  else
    echo "cargo-fmt-workspace: package=$package command=cargo fmt -p $package" >&2
    cargo fmt -p "$package"
  fi
done
