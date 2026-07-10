#!/usr/bin/env bash
# Clean a Cargo target directory and print a bounded absence/size readback.
set -euo pipefail

usage() {
  cat >&2 <<'USAGE'
Usage: bash scripts/cargo-target-clean.sh [--repo-root <path>] [--target-dir <path>] [--also-workspace-target]

Safely removes a Cargo target directory only when the resolved path is:
  - the workspace target directory,
  - a child of the workspace target directory, or
  - a child of CALYX_TARGET_SCRATCH_ROOT (default: <repo-parent>/target-scratch).

The script always prints a readback line for each requested target:
  cargo-target-clean: target_absent path=<path>
  cargo-target-clean: target_size path=<path> size=<du-size>
USAGE
}

repo_root_arg=""
target_dir_arg=""
also_workspace_target=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --repo-root)
      [[ $# -ge 2 ]] || { usage; exit 2; }
      repo_root_arg="$2"
      shift 2
      ;;
    --target-dir)
      [[ $# -ge 2 ]] || { usage; exit 2; }
      target_dir_arg="$2"
      shift 2
      ;;
    --also-workspace-target)
      also_workspace_target=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "cargo-target-clean: ERROR unknown argument: $1" >&2
      usage
      exit 2
      ;;
  esac
done

if ! command -v realpath >/dev/null 2>&1; then
  echo "cargo-target-clean: ERROR realpath not found on PATH" >&2
  exit 127
fi

if [[ -n "$repo_root_arg" ]]; then
  [[ -d "$repo_root_arg" ]] || {
    echo "cargo-target-clean: ERROR repo root does not exist: $repo_root_arg" >&2
    exit 2
  }
  repo_root="$(cd "$repo_root_arg" && pwd -P)"
else
  repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
fi

workspace_target="$(realpath -m -- "$repo_root/target")"
scratch_root_raw="${CALYX_TARGET_SCRATCH_ROOT:-$(dirname "$repo_root")/target-scratch}"
scratch_root="$(realpath -m -- "$scratch_root_raw")"

if [[ -z "$target_dir_arg" ]]; then
  target_dir_arg="${CARGO_TARGET_DIR:-$workspace_target}"
fi

target_dirs=("$target_dir_arg")
if [[ "$also_workspace_target" -eq 1 ]]; then
  target_dirs+=("$workspace_target")
fi

is_safe_target() {
  local target="$1"
  case "$target" in
    "$workspace_target"|"$workspace_target"/*) return 0 ;;
    "$scratch_root"/*) return 0 ;;
    *) return 1 ;;
  esac
}

clean_one() {
  local raw="$1"
  local target
  target="$(realpath -m -- "$raw")"

  if ! is_safe_target "$target"; then
    echo "cargo-target-clean: ERROR refusing unsafe target path: $target" >&2
    echo "cargo-target-clean: workspace_target=$workspace_target" >&2
    echo "cargo-target-clean: scratch_root=$scratch_root" >&2
    return 2
  fi

  if [[ ! -e "$target" ]]; then
    echo "cargo-target-clean: target_absent path=$target"
    return 0
  fi

  if [[ ! -d "$target" ]]; then
    echo "cargo-target-clean: ERROR target path exists but is not a directory: $target" >&2
    return 2
  fi

  rm -rf -- "$target"

  if [[ -e "$target" ]]; then
    local size
    size="$(du -sh -- "$target" 2>/dev/null | cut -f1 || printf unknown)"
    echo "cargo-target-clean: target_size path=$target size=$size"
    return 1
  fi

  echo "cargo-target-clean: target_absent path=$target"
}

seen=""
rc=0
for target in "${target_dirs[@]}"; do
  resolved="$(realpath -m -- "$target")"
  case "
$seen
" in
    *"
$resolved
"*) continue ;;
  esac
  seen="${seen}${resolved}"$'\n'
  clean_one "$resolved" || rc=$?
done

exit "$rc"
