#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat >&2 <<'USAGE'
Usage: bash scripts/fuzz-gate.sh [--repo-root <path>]

Formats, checks, and lints the standalone fuzz workspace. If CARGO_TARGET_DIR
is not already set by an outer gate, this script creates an isolated scratch
target and verifies cleanup on exit.
USAGE
}

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
repo_root_arg=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --repo-root)
      if [[ $# -lt 2 ]]; then
        echo "fuzz-gate: ERROR --repo-root requires a path" >&2
        usage
        exit 2
      fi
      repo_root_arg="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "fuzz-gate: ERROR unknown argument '$1'" >&2
      usage
      exit 2
      ;;
  esac
done

if [[ -n "$repo_root_arg" ]]; then
  if [[ ! -d "$repo_root_arg" ]]; then
    echo "fuzz-gate: ERROR repo root does not exist or is not a directory: $repo_root_arg" >&2
    exit 2
  fi
  repo_root="$(cd "$repo_root_arg" && pwd -P)"
else
  repo_root="$(cd "$script_dir/.." && pwd -P)"
fi

fuzz_manifest="$repo_root/fuzz/Cargo.toml"
if [[ ! -f "$fuzz_manifest" ]]; then
  echo "fuzz-gate: ERROR missing fuzz manifest: $fuzz_manifest" >&2
  exit 2
fi

if [[ -f "$HOME/.cargo/env" ]]; then
  # shellcheck source=/dev/null
  source "$HOME/.cargo/env"
fi
if [[ -f "$repo_root/env.sh" ]]; then
  # shellcheck source=/dev/null
  source "$repo_root/env.sh"
fi
if ! command -v cargo >/dev/null 2>&1; then
  echo "fuzz-gate: ERROR cargo not found on PATH" >&2
  exit 127
fi

owned_target_dir=""
if [[ -z "${CARGO_TARGET_DIR:-}" ]]; then
  scratch_root="${CALYX_TARGET_SCRATCH_ROOT:-$(dirname "$repo_root")/target-scratch}"
  export CALYX_TARGET_SCRATCH_ROOT="$scratch_root"
  mkdir -p "$scratch_root"
  owned_target_dir="$(mktemp -d "$scratch_root/fuzz-gate.XXXXXX")"
  export CARGO_TARGET_DIR="$owned_target_dir"
  export CARGO_INCREMENTAL="${CARGO_INCREMENTAL:-0}"
fi

cleanup_owned_target() {
  local status=$?
  local cleanup_status=0
  if [[ -n "$owned_target_dir" ]]; then
    bash "$script_dir/cargo-target-clean.sh" \
      --repo-root "$repo_root" \
      --target-dir "$owned_target_dir" \
      --also-workspace-target \
      --also-repo-targets \
      --also-calyx-home-target || cleanup_status=$?
  fi
  if [[ "$status" -eq 0 && "$cleanup_status" -ne 0 ]]; then
    exit "$cleanup_status"
  fi
  exit "$status"
}
trap cleanup_owned_target EXIT

echo "fuzz-gate: repo=$repo_root"
echo "fuzz-gate: manifest=$fuzz_manifest"
echo "fuzz-gate: CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-<cargo-default>}"

cargo fmt --manifest-path "$fuzz_manifest" -- --check
cargo check --locked --manifest-path "$fuzz_manifest" --bins
cargo clippy --locked --manifest-path "$fuzz_manifest" --bins -- -D warnings
