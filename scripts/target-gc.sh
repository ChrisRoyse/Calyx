#!/usr/bin/env bash
# ============================================================================
#  target-gc.sh — bound an explicit Cargo target dir on high-churn build hosts.
# ----------------------------------------------------------------------------
#  Cargo never garbage-collects superseded build artifacts. With several agents
#  rebuilding the workspace continuously, a target dir accumulates a fresh set of
#  hashed dep/test executables on every build and grew ~190 GB within ~2 weeks
#  on aiwonder (measured 2026-06-12). This caps the directory by removing the
#  OLDEST artifacts (via cargo-sweep --maxsize) until it is under the limit,
#  keeping the current working set intact (worst case: a stale crate recompiles
#  on the next build — never a correctness issue).
#
#  Run from cron on the build host (wired by scripts/aiwonder-build-setup.sh).
#  Override the limit with CALYX_TARGET_MAXSIZE (default 40GB).
#
#  Fail-loud, no fallbacks: if cargo-sweep is missing or the target dir cannot
#  be resolved, this errors out so the gap is visible rather than silently
#  letting the disk fill.
# ============================================================================
set -euo pipefail

if [[ -f "$HOME/.cargo/env" ]]; then
  source "$HOME/.cargo/env"
fi

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# cargo-sweep takes the PROJECT dir (it runs `cargo metadata` to find the target
# dir, honoring CARGO_TARGET_DIR) — NOT the target dir itself. Resolve the target
# dir the same way cargo does for reporting, export it so metadata agrees, and
# hand cargo-sweep the workspace root.
if [[ -n "${CARGO_TARGET_DIR:-}" ]]; then
  target_dir="$CARGO_TARGET_DIR"
else
  target_dir="$repo_root/target"
fi
export CARGO_TARGET_DIR="$target_dir"

if [[ ! -d "$target_dir" ]]; then
  echo "ERROR: target dir does not exist: $target_dir" >&2
  echo "       set CARGO_TARGET_DIR or run a build first." >&2
  exit 1
fi

if ! command -v cargo-sweep >/dev/null 2>&1; then
  echo "ERROR: cargo-sweep not installed (expected on PATH)." >&2
  echo "       run scripts/aiwonder-build-setup.sh to provision it." >&2
  exit 1
fi

if [[ ! -f "$repo_root/Cargo.toml" ]]; then
  echo "ERROR: no Cargo.toml at workspace root: $repo_root" >&2
  exit 1
fi

maxsize="${CALYX_TARGET_MAXSIZE:-40GB}"

before="$(du -sh "$target_dir" 2>/dev/null | cut -f1)"
echo "[target-gc] $(date -u +%FT%TZ) target=$target_dir before=$before limit=$maxsize"
cargo sweep --maxsize "$maxsize" "$repo_root"
after="$(du -sh "$target_dir" 2>/dev/null | cut -f1)"
echo "[target-gc] $(date -u +%FT%TZ) after=$after"
