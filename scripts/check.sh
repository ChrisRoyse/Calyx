#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
CALYX_GATE_STAGE_OWNER="scripts/check.sh"
# shellcheck source=gate-stage-runner.sh
source "$repo_root/scripts/gate-stage-runner.sh"

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

calyx_gate_resolve_home calyx_home
calyx_tmp_root="$calyx_home/tmp"
calyx_gate_resolve_target_root target_scratch_root "$calyx_home"

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

check_tmp_dir=""
check_tmp_owned=0
check_tmp_created=0
check_tmp_root_identity=""
check_tmp_identity=""
tmp_guard_baseline=""
tmp_guard_baseline_identity=""
tmp_guard_baseline_bytes=""
tmp_guard_baseline_sha=""
tmp_guard_started=0
check_target_dir=""
check_target_owned=0
check_target_created=0
check_target_root_identity=""
check_target_identity=""
check_gate_lock_fd=""
check_gate_lock_path=""
cleanup_gate() {
  local status=$?
  local cleanup_status=0 step_status=0 scratch_cleanup_safe=1
  trap - EXIT
  trap '' HUP INT TERM
  set +e

  calyx_gate_cleanup_safe
  step_status=$?
  if [[ "$step_status" -ne 0 ]]; then
    cleanup_status="$step_status"
    scratch_cleanup_safe=0
  fi

  if [[ "$scratch_cleanup_safe" -eq 1 && "$check_target_owned" -eq 1 ]]; then
    calyx_gate_run bash "$repo_root/scripts/cargo-target-clean.sh" \
      --repo-root "$repo_root" \
      --target-dir "$check_target_dir" \
      --expected-target-identity "$check_target_identity" \
      --gate-lock-fd "$check_gate_lock_fd" \
      --also-workspace-target \
      --also-repo-targets \
      --also-calyx-home-target
    step_status=$?
    if [[ "$step_status" -ne 0 ]]; then
      echo "ERROR: CALYX_CHECK_TARGET_CLEANUP_FAILED: status=$step_status target=$check_target_dir" >&2
      [[ "$cleanup_status" -ne 0 ]] || cleanup_status="$step_status"
    fi
  fi

  # Audit shared-temp leakage after every Cargo cleanup subprocess has exited,
  # while the private baseline is still readable inside the run TMPDIR.
  if [[ "$scratch_cleanup_safe" -eq 1 && "$tmp_guard_started" -eq 1 ]]; then
    calyx_gate_run bash "$repo_root/scripts/tmp_scratch_guard.sh" post "$tmp_guard_baseline" \
      "$tmp_guard_baseline_identity" "$tmp_guard_baseline_bytes" "$tmp_guard_baseline_sha"
    step_status=$?
    if [[ "$step_status" -ne 0 ]]; then
      echo "ERROR: CALYX_CHECK_TMP_GUARD_CLEANUP_FAILED: status=$step_status baseline=$tmp_guard_baseline" >&2
      [[ "$cleanup_status" -ne 0 ]] || cleanup_status="$step_status"
    fi
  fi

  if [[ "$scratch_cleanup_safe" -eq 1 && "$check_tmp_owned" -eq 1 ]]; then
    calyx_gate_run bash "$repo_root/scripts/check-tmpdir.sh" cleanup \
      "$calyx_tmp_root" "$check_tmp_dir" \
      "$check_tmp_root_identity" "$check_tmp_identity"
    step_status=$?
    if [[ "$step_status" -ne 0 ]]; then
      echo "ERROR: CALYX_CHECK_TMPDIR_CLEANUP_FAILED: status=$step_status run=$check_tmp_dir" >&2
      [[ "$cleanup_status" -ne 0 ]] || cleanup_status="$step_status"
    fi
  fi

  calyx_gate_close_lock check_gate_lock_fd
  step_status=$?
  if [[ "$step_status" -ne 0 ]]; then
    echo "ERROR: CALYX_CHECK_GATE_LOCK_CLOSE_FAILED: status=$step_status path=$check_gate_lock_path" >&2
    [[ "$cleanup_status" -ne 0 ]] || cleanup_status="$step_status"
  fi

  calyx_gate_finalize
  step_status=$?
  if [[ "$step_status" -ne 0 ]]; then
    echo "ERROR: CALYX_CHECK_CGROUP_FINALIZE_FAILED: status=$step_status slice=${CALYX_GATE_RUN_SLICE:-unset}" >&2
    [[ "$cleanup_status" -ne 0 ]] || cleanup_status="$step_status"
  fi

  echo "scripts/check.sh: cleanup_summary status=$status cleanup_status=$cleanup_status" >&2
  [[ "$cleanup_status" -eq 0 ]] || exit 125
  exit "$status"
}

handle_signal() {
  local signal_name="$1" signal_status="$2"
  trap '' HUP INT TERM
  calyx_gate_stop_active "$signal_name"
  exit "$signal_status"
}

trap cleanup_gate EXIT
trap 'handle_signal HUP 129' HUP
trap 'handle_signal INT 130' INT
trap 'handle_signal TERM 143' TERM

calyx_gate_signals_defer
calyx_gate_plan_owned_dir check_tmp_dir "$calyx_tmp_root" check
if calyx_gate_create_owned_dir check_tmp_created check_tmp_root_identity check_tmp_identity \
  "$calyx_home" "$calyx_tmp_root" "$check_tmp_dir" \
  bash "$repo_root/scripts/check-tmpdir.sh" create-at \
  "$calyx_tmp_root" "$check_tmp_dir" >/dev/null; then
  check_tmp_owned=1
  check_tmp_created=0
else
  setup_status=$?
  if [[ "$check_tmp_created" -eq 1 ]]; then
    check_tmp_owned=1
    check_tmp_created=0
  fi
  calyx_gate_signals_publish
  exit "$setup_status"
fi
calyx_gate_signals_publish
export TMPDIR="$check_tmp_dir"
export CALYX_GATE_TMPDIR="$TMPDIR"
export CALYX_GATE_TMP_ROOT="$calyx_tmp_root"
export CALYX_GATE_TMP_ROOT_IDENTITY="$check_tmp_root_identity"
export CALYX_GATE_TMP_IDENTITY="$check_tmp_identity"
tmp_guard_baseline="$TMPDIR/tmp-guard-baseline"
(umask 077; : >"$tmp_guard_baseline")

check_gate_lock_path="$calyx_home/tmp/.cargo-gate.lock"
calyx_gate_acquire_lock check_gate_lock_fd "$check_gate_lock_path"
export CALYX_GATE_LOCK_PATH="$check_gate_lock_path"
export CALYX_GATE_LOCK_FD="$check_gate_lock_fd"

calyx_gate_signals_defer
calyx_gate_plan_owned_dir check_target_dir "$target_scratch_root" check
if calyx_gate_create_owned_dir check_target_created check_target_root_identity check_target_identity \
  "$calyx_home" "$target_scratch_root" "$check_target_dir" \
  bash "$repo_root/scripts/check-tmpdir.sh" create-target-at \
  "$target_scratch_root" check "$check_target_dir" >/dev/null; then
  check_target_owned=1
  check_target_created=0
else
  setup_status=$?
  if [[ "$check_target_created" -eq 1 ]]; then
    check_target_owned=1
    check_target_created=0
  fi
  calyx_gate_signals_publish
  exit "$setup_status"
fi
calyx_gate_signals_publish

# The gate is a one-shot build (no edit-rebuild loop), so incremental
# compilation only adds overhead and disk churn (its cache grew to ~61 GB on the
# shared build host). Disable it for the manual aiwonder gate; interactive dev
# keeps its own default. This project has no Actions/hosted CI gate.
export CARGO_INCREMENTAL=0
export CARGO_TARGET_DIR="$check_target_dir"
export CALYX_GATE_TARGET_ROOT="$target_scratch_root"
export CALYX_GATE_TARGET_ROOT_IDENTITY="$check_target_root_identity"
export CALYX_GATE_TARGET_DIR="$check_target_dir"
export CALYX_GATE_TARGET_IDENTITY="$check_target_identity"
echo "scripts/check.sh: TMPDIR=$TMPDIR"
echo "scripts/check.sh: CARGO_TARGET_DIR=$CARGO_TARGET_DIR"

calyx_gate_run bash "$repo_root/scripts/tmp_scratch_guard.sh" pre "$tmp_guard_baseline"
tmp_guard_baseline_identity="$(stat -c '%d:%i' -- "$tmp_guard_baseline")"
tmp_guard_baseline_bytes="$(stat -c '%s' -- "$tmp_guard_baseline")"
tmp_guard_baseline_sha="$(sha256sum -- "$tmp_guard_baseline" | awk '{print $1}')"
tmp_guard_started=1

calyx_gate_run bash "$repo_root/scripts/secret-scan.sh"
calyx_gate_run bash "$repo_root/scripts/tests/check-tmpdir-regression.sh"
calyx_gate_run bash "$repo_root/scripts/tests/gate-wrapper-tmpdir-regression.sh"
calyx_gate_run bash "$repo_root/scripts/cargo-fmt-workspace.sh" --check
calyx_gate_run cargo check --workspace --all-targets
calyx_gate_run cargo clippy --workspace --all-targets -- -D warnings
calyx_gate_run bash "$repo_root/scripts/fuzz-gate.sh" --repo-root "$repo_root"

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
calyx_gate_run cargo nextest run --workspace
# nextest does not run doctests; run them with the built-in harness so doc
# examples stay covered.
calyx_gate_run cargo test --workspace --doc

calyx_gate_run bash "$repo_root/scripts/orphan_rs.sh"
calyx_gate_run bash "$repo_root/scripts/linecount.sh"
# Dataset MANIFEST tooling (PH69 T01): synthetic known-I/O + edge battery in a
# temp root - fast, hermetic, and keeps the digest algorithm pinned.
calyx_gate_run bash "$repo_root/scripts/verify_dataset.sh" --self-test
# DATA BUILD_DONE coverage gate (PH69 T08): hermetic synthetic-MANIFEST
# battery pinning the 12 required (modality x outcome) cells.
calyx_gate_run bash "$repo_root/scripts/check_manifest_coverage.sh" --self-test
