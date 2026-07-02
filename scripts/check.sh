#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

is_wsl_shell() {
  if [[ -n "${WSL_DISTRO_NAME:-}" || -n "${WSL_INTEROP:-}" ]]; then
    return 0
  fi

  local uname_release
  uname_release="$(uname -r 2>/dev/null || true)"
  [[ "$uname_release" == *Microsoft* || "$uname_release" == *microsoft* ]]
}

guard_wsl_windows_gitdir() {
  local git_file="$repo_root/.git"
  [[ -f "$git_file" ]] || return 0

  local gitdir
  gitdir="$(sed -n 's/^gitdir:[[:space:]]*//p' "$git_file" | head -n 1 | tr -d '\r')"
  [[ -n "$gitdir" ]] || return 0

  case "$gitdir" in
    [A-Za-z]:/* | [A-Za-z]:\\*) ;;
    *) return 0 ;;
  esac

  if is_wsl_shell || [[ "$repo_root" == /mnt/[A-Za-z]/* ]]; then
    echo "ERROR: CALYX_WSL_WINDOWS_WORKTREE_GITDIR: scripts/check.sh is running under WSL from a Windows-created git worktree." >&2
    echo "ERROR: .git points to '${gitdir}', which WSL git interprets relative to '${repo_root}' and cannot use." >&2
    echo "ERROR: remove this worktree and recreate it from WSL, for example:" >&2
    echo "ERROR:   git -C /mnt/c/code/Calyx-Dev worktree add /mnt/c/code/Calyx-Dev-issue1127 origin/main" >&2
    echo "ERROR: or run the gate from a checkout whose .git file uses a WSL/Linux gitdir path." >&2
    exit 1
  fi
}

guard_wsl_windows_gitdir

source "$HOME/.cargo/env"
cd "$repo_root"

if [[ -f "$repo_root/env.sh" ]]; then
  source "$repo_root/env.sh"
fi

# A suite-wide CALYX_FSV_ROOT points every FSV test at one shared evidence
# root, so independent tests collide on the same directories (issue #1014,
# observed as 11 spurious failures during the #1078 gate). Gate tests own
# their FSV roots; refuse the inherited value instead of silently unsetting.
# This refusal runs before the tmp-guard baseline/trap so an aborted gate
# never triggers the post sweep against a baseline that `pre` never wrote.
if [[ -n "${CALYX_FSV_ROOT:-}" ]]; then
  echo "ERROR: CALYX_FSV_ROOT is set ('${CALYX_FSV_ROOT}'); the full gate refuses a suite-wide FSV root." >&2
  echo "ERROR: unset CALYX_FSV_ROOT and rerun; give individual manual FSV runs their own absolute root instead (issue #1014)." >&2
  exit 1
fi

tmp_guard_baseline="$(mktemp -t calyx-check-tmp-baseline.XXXXXX)"
cleanup_tmp_guard() {
  local status=$?
  bash "$repo_root/scripts/tmp_scratch_guard.sh" post "$tmp_guard_baseline" || true
  rm -f "$tmp_guard_baseline"
  exit "$status"
}
trap cleanup_tmp_guard EXIT

# The gate is a one-shot build (no edit-rebuild loop), so incremental
# compilation only adds overhead and disk churn (its cache grew to ~61 GB on the
# shared build host). Disable it for the manual aiwonder gate; interactive dev
# keeps its own default. This project has no Actions/hosted CI gate.
export CARGO_INCREMENTAL=0

bash "$repo_root/scripts/tmp_scratch_guard.sh" pre "$tmp_guard_baseline"

bash "$repo_root/scripts/cargo-fmt-workspace.sh" --check
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings

# Test execution with nextest: it runs every test across every binary in a
# single parallel pool sized to all logical CPUs, whereas `cargo test` runs each
# test binary sequentially and leaves most cores idle. With 1500+ tests across
# 250+ binaries that is the difference between saturating the box and waiting on
# one core. Fail-loud: if cargo-nextest is missing the gate errors and points
# to the focused local provisioning script rather than silently skipping.
if ! command -v cargo-nextest >/dev/null 2>&1; then
  echo "ERROR: cargo-nextest not installed." >&2
  echo "ERROR: run 'bash scripts/install-cargo-nextest.sh' for Bash/WSL, or" >&2
  echo "ERROR: run 'pwsh -File scripts/install-cargo-nextest.ps1' for native Windows PowerShell." >&2
  exit 1
fi
cargo nextest run --workspace
# nextest does not run doctests; run them with the built-in harness so doc
# examples stay covered.
cargo test --workspace --doc

bash "$repo_root/scripts/orphan_rs.sh"
bash "$repo_root/scripts/linecount.sh"
# Dataset MANIFEST tooling (PH69 T01): synthetic known-I/O + edge battery in a
# temp root - fast, hermetic, and keeps the digest algorithm pinned.
bash "$repo_root/scripts/verify_dataset.sh" --self-test
# DATA BUILD_DONE coverage gate (PH69 T08): hermetic synthetic-MANIFEST
# battery pinning the 12 required (modality x outcome) cells.
bash "$repo_root/scripts/check_manifest_coverage.sh" --self-test
