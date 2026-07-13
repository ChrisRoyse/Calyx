#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat >&2 <<'USAGE'
Usage: bash scripts/fuzz-gate.sh [--repo-root <path>]

Formats, checks, and lints the standalone fuzz workspace. If CARGO_TARGET_DIR
is not already set by an outer gate, this script creates an isolated scratch
target and attempts identity-bound cleanup on every exit. Cleanup failure returns
125 and retains the exact recovery path. A validated outer gate TMPDIR is
inherited; otherwise the standalone gate owns its Calyx run TMPDIR.
USAGE
}

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
CALYX_GATE_STAGE_OWNER="fuzz-gate"
# shellcheck source=gate-stage-runner.sh
source "$script_dir/gate-stage-runner.sh"
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
calyx_gate_resolve_home calyx_home
tmp_root="$calyx_home/tmp"
calyx_gate_resolve_target_root scratch_root "$calyx_home"
if ! command -v cargo >/dev/null 2>&1; then
  echo "fuzz-gate: ERROR cargo not found on PATH" >&2
  exit 127
fi
if [[ -n "${CARGO_TARGET_DIR:-}" && -z "${CALYX_GATE_TMPDIR:-}" ]]; then
  echo "fuzz-gate: ERROR inherited CARGO_TARGET_DIR requires a validated outer gate TMPDIR and lock" >&2
  exit 2
fi

owned_target_dir=""
owned_target_ready=0
owned_target_created=0
owned_target_root_identity=""
owned_target_identity=""
gate_lock_fd=""
gate_lock_path=""
owned_gate_lock=0
owned_tmp_dir=""
owned_tmp_ready=0
owned_tmp_created=0
owned_tmp_root_identity=""
owned_tmp_identity=""
cleanup_owned_scratch() {
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

  if [[ "$scratch_cleanup_safe" -eq 1 && "$owned_target_ready" -eq 1 ]]; then
    calyx_gate_run bash "$script_dir/cargo-target-clean.sh" \
      --repo-root "$repo_root" \
      --target-dir "$owned_target_dir" \
      --expected-target-identity "$owned_target_identity" \
      --gate-lock-fd "$gate_lock_fd" \
      --also-workspace-target \
      --also-repo-targets \
      --also-calyx-home-target
    step_status=$?
    if [[ "$step_status" -ne 0 ]]; then
      echo "fuzz-gate: ERROR target cleanup failed: status=$step_status target=$owned_target_dir" >&2
      cleanup_status="$step_status"
    fi
  fi
  if [[ "$scratch_cleanup_safe" -eq 1 && "$owned_tmp_ready" -eq 1 ]]; then
    calyx_gate_run bash "$script_dir/check-tmpdir.sh" cleanup \
      "$tmp_root" "$owned_tmp_dir" \
      "$owned_tmp_root_identity" "$owned_tmp_identity"
    step_status=$?
    if [[ "$step_status" -ne 0 ]]; then
      echo "fuzz-gate: ERROR TMPDIR cleanup failed: status=$step_status run=$owned_tmp_dir" >&2
      [[ "$cleanup_status" -ne 0 ]] || cleanup_status="$step_status"
    fi
  fi
  if [[ "$owned_gate_lock" -eq 1 ]]; then
    calyx_gate_close_lock gate_lock_fd
    step_status=$?
    if [[ "$step_status" -ne 0 ]]; then
      echo "fuzz-gate: ERROR gate lock close failed: status=$step_status path=$gate_lock_path" >&2
      [[ "$cleanup_status" -ne 0 ]] || cleanup_status="$step_status"
    fi
  fi
  calyx_gate_finalize
  step_status=$?
  if [[ "$step_status" -ne 0 ]]; then
    echo "fuzz-gate: ERROR cgroup finalize failed: status=$step_status slice=${CALYX_GATE_RUN_SLICE:-unset}" >&2
    [[ "$cleanup_status" -ne 0 ]] || cleanup_status="$step_status"
  fi
  [[ "$cleanup_status" -eq 0 ]] || exit 125
  exit "$status"
}

handle_signal() {
  local signal_name="$1" signal_status="$2"
  trap '' HUP INT TERM
  calyx_gate_stop_active "$signal_name"
  exit "$signal_status"
}

trap cleanup_owned_scratch EXIT
trap 'handle_signal HUP 129' HUP
trap 'handle_signal INT 130' INT
trap 'handle_signal TERM 143' TERM

if [[ -n "${CALYX_GATE_TMPDIR:-}" ]]; then
  if [[ "${TMPDIR:-}" != "$CALYX_GATE_TMPDIR" || -z "${CALYX_GATE_TMP_ROOT:-}" \
    || -z "${CALYX_GATE_TMP_ROOT_IDENTITY:-}" || -z "${CALYX_GATE_TMP_IDENTITY:-}" ]]; then
    echo "fuzz-gate: ERROR outer gate TMPDIR/root marker is incomplete or inconsistent" >&2
    exit 2
  fi
  [[ "$CALYX_GATE_TMP_ROOT" == "$tmp_root" ]] || {
    echo "fuzz-gate: ERROR outer gate TMPDIR root is not canonical: expected=$tmp_root actual=$CALYX_GATE_TMP_ROOT" >&2
    exit 125
  }
  if [[ -z "${CARGO_TARGET_DIR:-}" || -z "${CALYX_GATE_TARGET_ROOT:-}" \
    || -z "${CALYX_GATE_TARGET_ROOT_IDENTITY:-}" || -z "${CALYX_GATE_TARGET_DIR:-}" \
    || -z "${CALYX_GATE_TARGET_IDENTITY:-}" \
    || "$CARGO_TARGET_DIR" != "$CALYX_GATE_TARGET_DIR" ]]; then
    echo "fuzz-gate: ERROR outer gate Cargo target marker is incomplete or inconsistent" >&2
    exit 2
  fi
  [[ "$CALYX_GATE_TARGET_ROOT" == "$scratch_root" ]] || {
    echo "fuzz-gate: ERROR outer gate Cargo target root is not canonical: expected=$scratch_root actual=$CALYX_GATE_TARGET_ROOT" >&2
    exit 125
  }
  caller_pgid="$(ps -o pgid= -p "$BASHPID" 2>/dev/null | tr -d ' ')"
  if [[ "$caller_pgid" != "$BASHPID" ]]; then
    echo "fuzz-gate: ERROR validated outer TMPDIR requires fuzz to be an owned process-group leader: pid=$BASHPID pgid=${caller_pgid:-missing}" >&2
    exit 125
  fi
  calyx_gate_run bash "$script_dir/check-tmpdir.sh" validate "$tmp_root" "$CALYX_GATE_TMPDIR" \
    "$CALYX_GATE_TMP_ROOT_IDENTITY" "$CALYX_GATE_TMP_IDENTITY"
  calyx_gate_run bash "$script_dir/check-tmpdir.sh" validate-target \
    "$scratch_root" "$CARGO_TARGET_DIR" \
    "$CALYX_GATE_TARGET_ROOT_IDENTITY" "$CALYX_GATE_TARGET_IDENTITY"
  gate_lock_path="${CALYX_GATE_LOCK_PATH:-}"
  gate_lock_fd="${CALYX_GATE_LOCK_FD:-}"
  [[ -n "$gate_lock_path" && -n "$gate_lock_fd" ]] || {
    echo "fuzz-gate: ERROR outer gate Cargo lock marker is incomplete" >&2
    exit 125
  }
  [[ "$gate_lock_path" == "$tmp_root/.cargo-gate.lock" ]] || {
    echo "fuzz-gate: ERROR outer gate Cargo lock path is not canonical: expected=$tmp_root/.cargo-gate.lock actual=$gate_lock_path" >&2
    exit 125
  }
  calyx_gate_validate_lock "$gate_lock_path" "$gate_lock_fd"
else
  calyx_gate_signals_defer
  calyx_gate_plan_owned_dir owned_tmp_dir "$tmp_root" check
  if calyx_gate_create_owned_dir owned_tmp_created owned_tmp_root_identity owned_tmp_identity \
    "$calyx_home" "$tmp_root" "$owned_tmp_dir" \
    bash "$script_dir/check-tmpdir.sh" create-at \
    "$tmp_root" "$owned_tmp_dir" >/dev/null; then
    owned_tmp_ready=1
    owned_tmp_created=0
  else
    setup_status=$?
    if [[ "$owned_tmp_created" -eq 1 ]]; then
      owned_tmp_ready=1
      owned_tmp_created=0
    fi
    calyx_gate_signals_publish
    exit "$setup_status"
  fi
  calyx_gate_signals_publish
  export TMPDIR="$owned_tmp_dir"
  export CALYX_GATE_TMPDIR="$TMPDIR"
  export CALYX_GATE_TMP_ROOT="$tmp_root"
  export CALYX_GATE_TMP_ROOT_IDENTITY="$owned_tmp_root_identity"
  export CALYX_GATE_TMP_IDENTITY="$owned_tmp_identity"
  gate_lock_path="$tmp_root/.cargo-gate.lock"
  calyx_gate_acquire_lock gate_lock_fd "$gate_lock_path"
  owned_gate_lock=1
  export CALYX_GATE_LOCK_PATH="$gate_lock_path"
  export CALYX_GATE_LOCK_FD="$gate_lock_fd"
fi

if [[ -z "${CARGO_TARGET_DIR:-}" ]]; then
  calyx_gate_signals_defer
  calyx_gate_plan_owned_dir owned_target_dir "$scratch_root" fuzz-gate
  if calyx_gate_create_owned_dir owned_target_created owned_target_root_identity owned_target_identity \
    "$calyx_home" "$scratch_root" "$owned_target_dir" \
    bash "$script_dir/check-tmpdir.sh" create-target-at \
    "$scratch_root" fuzz-gate "$owned_target_dir" >/dev/null; then
    owned_target_ready=1
    owned_target_created=0
  else
    setup_status=$?
    if [[ "$owned_target_created" -eq 1 ]]; then
      owned_target_ready=1
      owned_target_created=0
    fi
    calyx_gate_signals_publish
    exit "$setup_status"
  fi
  calyx_gate_signals_publish
  export CARGO_TARGET_DIR="$owned_target_dir"
  export CARGO_INCREMENTAL="${CARGO_INCREMENTAL:-0}"
  export CALYX_GATE_TARGET_ROOT="$scratch_root"
  export CALYX_GATE_TARGET_ROOT_IDENTITY="$owned_target_root_identity"
  export CALYX_GATE_TARGET_DIR="$owned_target_dir"
  export CALYX_GATE_TARGET_IDENTITY="$owned_target_identity"
fi

echo "fuzz-gate: repo=$repo_root"
echo "fuzz-gate: manifest=$fuzz_manifest"
echo "fuzz-gate: TMPDIR=${TMPDIR:-<unset>}"
echo "fuzz-gate: CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-<cargo-default>}"
fuzz_gate_pgid="$(ps -o pgid= -p "$BASHPID" 2>/dev/null | tr -d ' ')"
echo "fuzz-gate: pid=$BASHPID pgid=${fuzz_gate_pgid:-missing}"

calyx_gate_run cargo fmt --manifest-path "$fuzz_manifest" -- --check
calyx_gate_run cargo check --locked --manifest-path "$fuzz_manifest" --bins
calyx_gate_run cargo clippy --locked --manifest-path "$fuzz_manifest" --bins -- -D warnings
