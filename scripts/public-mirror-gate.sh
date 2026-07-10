#!/usr/bin/env bash
# Run the public mirror gate with an isolated, auto-cleaned Cargo target dir.
set -euo pipefail

usage() {
  cat >&2 <<'USAGE'
Usage: bash scripts/public-mirror-gate.sh --public-dir <path> [--target-dir <path>]

Runs the public checkout validation sequence from Calyx-Dev tooling while
keeping Cargo output in a disposable target dir. The cleanup trap removes the
explicit target and the public checkout's workspace target, then prints bounded
target_absent/target_size readback lines.
USAGE
}

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
dev_root="$(cd "$script_dir/.." && pwd -P)"
public_dir=""
target_dir=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --public-dir)
      [[ $# -ge 2 ]] || { usage; exit 2; }
      public_dir="$2"
      shift 2
      ;;
    --target-dir)
      [[ $# -ge 2 ]] || { usage; exit 2; }
      target_dir="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "public-mirror-gate: ERROR unknown argument: $1" >&2
      usage
      exit 2
      ;;
  esac
done

if [[ -z "$public_dir" ]]; then
  usage
  exit 2
fi

public_root="$(git -C "$public_dir" rev-parse --show-toplevel)"
public_root="$(cd "$public_root" && pwd -P)"
if [[ "$dev_root" == "$public_root" ]]; then
  echo "public-mirror-gate: ERROR public checkout must be separate from Calyx-Dev" >&2
  exit 2
fi

origin_url="$(git -C "$public_root" remote get-url origin 2>/dev/null || true)"
if [[ "$origin_url" != *"ChrisRoyse/Calyx"* ]]; then
  echo "public-mirror-gate: ERROR public checkout origin is not ChrisRoyse/Calyx: $origin_url" >&2
  exit 2
fi

if [[ -f "$HOME/.cargo/env" ]]; then
  # shellcheck source=/dev/null
  source "$HOME/.cargo/env"
fi
if [[ -f "$dev_root/env.sh" ]]; then
  # shellcheck source=/dev/null
  source "$dev_root/env.sh"
fi
if ! command -v cargo >/dev/null 2>&1; then
  echo "public-mirror-gate: ERROR cargo not found on PATH" >&2
  exit 127
fi
if ! command -v cargo-nextest >/dev/null 2>&1; then
  echo "public-mirror-gate: ERROR cargo-nextest not installed" >&2
  exit 127
fi

if [[ -z "$target_dir" ]]; then
  scratch_root="${CALYX_TARGET_SCRATCH_ROOT:-$(dirname "$dev_root")/target-scratch}"
  export CALYX_TARGET_SCRATCH_ROOT="$scratch_root"
  mkdir -p "$scratch_root"
  target_dir="$(mktemp -d "$scratch_root/public-mirror-gate.XXXXXX")"
else
  scratch_root="${CALYX_TARGET_SCRATCH_ROOT:-$(dirname "$dev_root")/target-scratch}"
  export CALYX_TARGET_SCRATCH_ROOT="$scratch_root"
  bash "$dev_root/scripts/cargo-target-clean.sh" \
    --repo-root "$public_root" \
    --target-dir "$target_dir" \
    --also-workspace-target \
    --also-repo-targets \
    --also-calyx-home-target >/dev/null
  mkdir -p "$target_dir"
fi
target_dir="$(cd "$target_dir" && pwd -P)"

public_unstaged_status() {
  git -C "$public_root" status --porcelain | grep -E '^(.[^ ]|\?\?)' || true
}

reject_unstaged_public_changes() {
  local phase="$1"
  local dirty
  dirty="$(public_unstaged_status)"
  if [[ -n "$dirty" ]]; then
    echo "public-mirror-gate: ERROR public checkout has unstaged/untracked changes $phase:" >&2
    echo "$dirty" | sed 's/^/  /' >&2
    exit 1
  fi
}

cleanup_public_scratch() {
  local rel='crates/calyx-poly/C:\code\poly\target\fsv\issue029_ws_market_client_20260707'
  local path="$public_root/$rel"
  if [[ ! -e "$path" ]]; then
    echo "public-mirror-gate: scratch_absent path=$path"
    return 0
  fi

  local abs
  abs="$(realpath -m "$path")"
  case "$abs" in
    "$public_root"/*)
      rm -rf -- "$path"
      echo "public-mirror-gate: scratch_absent path=$path"
      ;;
    *)
      echo "public-mirror-gate: ERROR refusing scratch cleanup outside public checkout: $abs" >&2
      return 1
      ;;
  esac
}

cleanup_targets() {
  local status=$?
  local cleanup_status=0
  cleanup_public_scratch || cleanup_status=$?
  bash "$dev_root/scripts/cargo-target-clean.sh" \
    --repo-root "$public_root" \
    --target-dir "$target_dir" \
    --also-workspace-target \
    --also-repo-targets \
    --also-calyx-home-target || cleanup_status=$?
  if [[ "$status" -eq 0 && "$cleanup_status" -ne 0 ]]; then
    exit "$cleanup_status"
  fi
  exit "$status"
}
trap cleanup_targets EXIT

export CARGO_INCREMENTAL=0
export CARGO_TARGET_DIR="$target_dir"

echo "public-mirror-gate: dev_root=$dev_root"
echo "public-mirror-gate: public_root=$public_root"
echo "public-mirror-gate: target_dir=$CARGO_TARGET_DIR"

reject_unstaged_public_changes "before gate"
( cd "$public_root" && bash "$dev_root/scripts/secret-scan.sh" --staged )
bash "$dev_root/scripts/public-leak-scan.sh" --public-tree "$public_root"
git -C "$public_root" diff --cached --check
bash "$dev_root/scripts/cargo-fmt-workspace.sh" --check --repo-root "$public_root"
(
  cd "$public_root"
  cargo check --workspace --all-targets
  cargo clippy --workspace --all-targets -- -D warnings
  bash "$dev_root/scripts/fuzz-gate.sh" --repo-root "$public_root"
  cargo nextest run --workspace
  cargo test --workspace --doc
)
bash "$dev_root/scripts/orphan_rs.sh" --repo-root "$public_root"
bash "$dev_root/scripts/linecount.sh" --repo-root "$public_root"
cleanup_public_scratch
reject_unstaged_public_changes "after gate"
