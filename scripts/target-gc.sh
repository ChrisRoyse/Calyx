#!/usr/bin/env bash
# ============================================================================
#  target-gc.sh — bound the shared Cargo target dir on high-churn build hosts.
# ----------------------------------------------------------------------------
#  Cargo never garbage-collects superseded build artifacts. With several agents
#  rebuilding the workspace continuously, target/ accumulates a fresh set of
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

# Resolve the target dir the same way cargo does: CARGO_TARGET_DIR wins, else
# the repo's own target/. Error out rather than guess if neither resolves.
if [[ -n "${CARGO_TARGET_DIR:-}" ]]; then
  target_dir="$CARGO_TARGET_DIR"
else
  target_dir="$repo_root/target"
fi

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

maxsize="${CALYX_TARGET_MAXSIZE:-40GB}"

before="$(du -sh "$target_dir" 2>/dev/null | cut -f1)"
echo "[target-gc] $(date -u +%FT%TZ) target=$target_dir before=$before limit=$maxsize"
cargo sweep --maxsize "$maxsize" "$target_dir"
after="$(du -sh "$target_dir" 2>/dev/null | cut -f1)"
echo "[target-gc] $(date -u +%FT%TZ) after=$after"
