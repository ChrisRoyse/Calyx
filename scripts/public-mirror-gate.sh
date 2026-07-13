#!/usr/bin/env bash
# Run the public mirror gate with isolated, auto-cleaned temp and Cargo roots.
set -euo pipefail

usage() {
  cat >&2 <<'USAGE'
Usage: bash scripts/public-mirror-gate.sh --public-dir <path> [--target-dir <path>]

Runs the public checkout validation sequence from Calyx-Dev tooling while
binding every stage to one run-owned Calyx TMPDIR and keeping Cargo output in a
disposable target dir. The cleanup trap attempts identity-bound removal of the
run TMPDIR, Cargo target, and exact known public-scratch leaf. Failure returns
125 with recovery state retained; success prints bounded absence/audit readbacks.
USAGE
}

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
dev_root="$(cd "$script_dir/.." && pwd -P)"
CALYX_GATE_STAGE_OWNER="public-mirror-gate"
# shellcheck source=gate-stage-runner.sh
source "$script_dir/gate-stage-runner.sh"
public_dir=""
target_dir=""
target_cleanup_ready=0
target_created=0
target_root_identity=""
target_identity=""
gate_lock_fd=""
gate_lock_path=""
run_tmp_dir=""
run_tmp_owned=0
run_tmp_created=0
run_tmp_root_identity=""
run_tmp_identity=""
tmp_guard_baseline=""
tmp_guard_baseline_identity=""
tmp_guard_baseline_bytes=""
tmp_guard_baseline_sha=""
tmp_guard_started=0
public_cleanup_ready=0

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
[[ "$public_dir" != *$'\n'* && "$public_dir" != *$'\r'* \
  && "$target_dir" != *$'\n'* && "$target_dir" != *$'\r'* ]] || {
  echo "public-mirror-gate: ERROR public/target paths must be single-line" >&2
  exit 2
}

public_root="$(git -C "$public_dir" rev-parse --show-toplevel)"
public_root="$(cd "$public_root" && pwd -P)"
if [[ "$dev_root" == "$public_root" ]]; then
  echo "public-mirror-gate: ERROR public checkout must be separate from Calyx-Dev" >&2
  exit 2
fi
public_root_identity="$(stat -c '%d:%i' -- "$public_root")" || exit 125

origin_url="$(git -C "$public_root" remote get-url origin 2>/dev/null || true)"
case "$origin_url" in
  https://github.com/ChrisRoyse/Calyx|https://github.com/ChrisRoyse/Calyx.git|\
  git@github.com:ChrisRoyse/Calyx|git@github.com:ChrisRoyse/Calyx.git|\
  ssh://git@github.com/ChrisRoyse/Calyx|ssh://git@github.com/ChrisRoyse/Calyx.git) ;;
  *)
  echo "public-mirror-gate: ERROR public checkout origin is not ChrisRoyse/Calyx: $origin_url" >&2
  exit 2
  ;;
esac
[[ "$(stat -c '%d:%i' -- "$public_root")" == "$public_root_identity" ]] || {
  echo "public-mirror-gate: ERROR public checkout identity changed during origin validation" >&2
  exit 125
}

if [[ -f "$HOME/.cargo/env" ]]; then
  # shellcheck source=/dev/null
  source "$HOME/.cargo/env"
fi
if [[ -f "$dev_root/env.sh" ]]; then
  # shellcheck source=/dev/null
  source "$dev_root/env.sh"
fi
calyx_gate_resolve_home calyx_home
tmp_root="$calyx_home/tmp"
calyx_gate_resolve_target_root scratch_root "$calyx_home"
case "$dev_root:$public_root" in
  "$calyx_home"/*:"$calyx_home"/*) ;;
  *)
    echo "public-mirror-gate: ERROR dev and public roots must both be inside CALYX_HOME=$calyx_home" >&2
    exit 2
    ;;
esac
if [[ -n "${CALYX_FSV_ROOT:-}" ]]; then
  echo "public-mirror-gate: ERROR CALYX_FSV_ROOT is set ('${CALYX_FSV_ROOT}'); the full public gate refuses a suite-wide FSV root" >&2
  exit 1
fi
if ! command -v cargo >/dev/null 2>&1; then
  echo "public-mirror-gate: ERROR cargo not found on PATH" >&2
  exit 127
fi
if ! command -v cargo-nextest >/dev/null 2>&1; then
  echo "public-mirror-gate: ERROR cargo-nextest not installed" >&2
  exit 127
fi

reject_unstaged_public_changes() {
  local phase="$1" status_file="$TMPDIR/public-status-${2:?}" line prefix dirty=""
  if calyx_gate_run git -C "$public_root" status --porcelain >"$status_file"; then
    :
  else
    local status=$?
    echo "public-mirror-gate: ERROR public status read failed $phase: status=$status" >&2
    exit "$status"
  fi
  while IFS= read -r line; do
    prefix="${line:0:2}"
    if [[ "$prefix" == "??" || "${prefix:1:1}" != " " ]]; then
      dirty+="${dirty:+$'\n'}$line"
    fi
  done <"$status_file"
  if [[ -n "$dirty" ]]; then
    echo "public-mirror-gate: ERROR public checkout has unstaged/untracked changes $phase:" >&2
    while IFS= read -r line; do echo "  $line" >&2; done <<<"$dirty"
    exit 1
  fi
}

cleanup_public_scratch() {
  local rel='crates/calyx-poly/C:\code\poly\target\fsv\issue029_ws_market_client_20260707'
  local path="$public_root/$rel"
  if [[ ! -e "$path" && ! -L "$path" ]]; then
    echo "public-mirror-gate: scratch_absent path=$path"
    return 0
  fi

  local lexical physical owner_uid identity relative
  [[ "$(stat -c '%d:%i' -- "$public_root")" == "$public_root_identity" ]] || {
    echo "public-mirror-gate: ERROR public checkout identity changed before scratch cleanup" >&2
    return 125
  }
  relative="${path#"$public_root"/}"
  if [[ -L "$path" ]]; then
    owner_uid="$(stat -c '%u' -- "$path")" || return 125
    identity="$(stat -c '%d:%i' -- "$path")" || return 125
    calyx_gate_safe_remove_entry "$public_root" "$public_root_identity" "$relative" \
      "$identity" any public-scratch "$owner_uid" "$owner_uid" "" "" || return 125
    [[ ! -e "$path" && ! -L "$path" ]] || return 125
    echo "public-mirror-gate: scratch_absent path=$path"
    return 0
  fi
  calyx_gate_lexical_path lexical "public scratch" "$path" || return $?
  physical="$(realpath -m -- "$path")" || return 125
  [[ "$lexical" == "$physical" && "$lexical" == "$public_root/"* && -d "$path" ]] || {
    echo "public-mirror-gate: ERROR refusing unstable public scratch cleanup: lexical=$lexical physical=$physical" >&2
    return 125
  }
  owner_uid="$(stat -c '%u' -- "$path")" || return 125
  [[ "$owner_uid" == "$(id -u)" ]] || return 125
  calyx_gate_assert_no_mounts_at_or_below "$path" "public scratch" || return $?
  identity="$(stat -c '%d:%i' -- "$path")" || return 125
  calyx_gate_safe_remove_entry "$public_root" "$public_root_identity" "$relative" \
    "$identity" directory public-scratch "$owner_uid" "$owner_uid" "" "" || return 125
  [[ ! -e "$path" && ! -L "$path" ]] || return 125
  echo "public-mirror-gate: scratch_absent path=$path"
}

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

  if [[ "$scratch_cleanup_safe" -eq 1 && "$public_cleanup_ready" -eq 1 ]]; then
    cleanup_public_scratch
    step_status=$?
    if [[ "$step_status" -ne 0 ]]; then
      cleanup_status="$step_status"
    fi
  fi

  if [[ "$scratch_cleanup_safe" -eq 1 && "$target_cleanup_ready" -eq 1 && -n "$target_dir" ]]; then
    calyx_gate_run bash "$dev_root/scripts/cargo-target-clean.sh" \
      --repo-root "$public_root" \
      --target-dir "$target_dir" \
      --expected-target-identity "$target_identity" \
      --gate-lock-fd "$gate_lock_fd" \
      --also-workspace-target \
      --also-repo-targets \
      --also-calyx-home-target
    step_status=$?
    if [[ "$step_status" -ne 0 ]]; then
      echo "public-mirror-gate: ERROR target cleanup failed: status=$step_status target=$target_dir" >&2
      [[ "$cleanup_status" -ne 0 ]] || cleanup_status="$step_status"
    fi
  fi

  if [[ "$scratch_cleanup_safe" -eq 1 && "$tmp_guard_started" -eq 1 ]]; then
    calyx_gate_run bash "$dev_root/scripts/tmp_scratch_guard.sh" post "$tmp_guard_baseline" \
      "$tmp_guard_baseline_identity" "$tmp_guard_baseline_bytes" "$tmp_guard_baseline_sha"
    step_status=$?
    if [[ "$step_status" -ne 0 ]]; then
      echo "public-mirror-gate: ERROR tmp guard cleanup failed: status=$step_status baseline=$tmp_guard_baseline" >&2
      [[ "$cleanup_status" -ne 0 ]] || cleanup_status="$step_status"
    fi
  fi

  if [[ "$scratch_cleanup_safe" -eq 1 && "$run_tmp_owned" -eq 1 ]]; then
    calyx_gate_run bash "$dev_root/scripts/check-tmpdir.sh" cleanup \
      "$tmp_root" "$run_tmp_dir" \
      "$run_tmp_root_identity" "$run_tmp_identity"
    step_status=$?
    if [[ "$step_status" -ne 0 ]]; then
      echo "public-mirror-gate: ERROR TMPDIR cleanup failed: status=$step_status run=$run_tmp_dir" >&2
      [[ "$cleanup_status" -ne 0 ]] || cleanup_status="$step_status"
    fi
  fi

  calyx_gate_close_lock gate_lock_fd
  step_status=$?
  if [[ "$step_status" -ne 0 ]]; then
    echo "public-mirror-gate: ERROR gate lock close failed: status=$step_status path=$gate_lock_path" >&2
    [[ "$cleanup_status" -ne 0 ]] || cleanup_status="$step_status"
  fi

  calyx_gate_finalize
  step_status=$?
  if [[ "$step_status" -ne 0 ]]; then
    echo "public-mirror-gate: ERROR cgroup finalize failed: status=$step_status slice=${CALYX_GATE_RUN_SLICE:-unset}" >&2
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

trap cleanup_gate EXIT
trap 'handle_signal HUP 129' HUP
trap 'handle_signal INT 130' INT
trap 'handle_signal TERM 143' TERM

calyx_gate_signals_defer
calyx_gate_plan_owned_dir run_tmp_dir "$tmp_root" check
if calyx_gate_create_owned_dir run_tmp_created run_tmp_root_identity run_tmp_identity \
  "$calyx_home" bash "$dev_root/scripts/check-tmpdir.sh" create-at \
  "$tmp_root" "$run_tmp_dir" >/dev/null; then
  run_tmp_owned=1
  run_tmp_created=0
else
  setup_status=$?
  if [[ "$run_tmp_created" -eq 1 ]]; then
    run_tmp_owned=1
    run_tmp_created=0
  fi
  calyx_gate_signals_publish
  exit "$setup_status"
fi
calyx_gate_signals_publish
export TMPDIR="$run_tmp_dir"
export CALYX_GATE_TMPDIR="$TMPDIR"
export CALYX_GATE_TMP_ROOT="$tmp_root"
export CALYX_GATE_TMP_ROOT_IDENTITY="$run_tmp_root_identity"
export CALYX_GATE_TMP_IDENTITY="$run_tmp_identity"
tmp_guard_baseline="$TMPDIR/tmp-guard-baseline"
(umask 077; : >"$tmp_guard_baseline")
gate_lock_path="$tmp_root/.cargo-gate.lock"
calyx_gate_acquire_lock gate_lock_fd "$gate_lock_path"
export CALYX_GATE_LOCK_PATH="$gate_lock_path"
export CALYX_GATE_LOCK_FD="$gate_lock_fd"
calyx_gate_run bash "$dev_root/scripts/tmp_scratch_guard.sh" pre "$tmp_guard_baseline"
tmp_guard_baseline_identity="$(stat -c '%d:%i' -- "$tmp_guard_baseline")"
tmp_guard_baseline_bytes="$(stat -c '%s' -- "$tmp_guard_baseline")"
tmp_guard_baseline_sha="$(sha256sum -- "$tmp_guard_baseline" | awk '{print $1}')"
tmp_guard_started=1

if [[ -z "$target_dir" ]]; then
  calyx_gate_signals_defer
  calyx_gate_plan_owned_dir target_dir "$scratch_root" public-mirror-gate
  if calyx_gate_create_owned_dir target_created target_root_identity target_identity \
    "$calyx_home" bash "$dev_root/scripts/check-tmpdir.sh" create-target-at \
    "$scratch_root" public-mirror-gate "$target_dir" >/dev/null; then
    target_cleanup_ready=1
    target_created=0
  else
    setup_status=$?
    if [[ "$target_created" -eq 1 ]]; then
      target_cleanup_ready=1
      target_created=0
    fi
    calyx_gate_signals_publish
    exit "$setup_status"
  fi
  calyx_gate_signals_publish
else
  calyx_gate_lexical_path target_dir "explicit Cargo target" "$target_dir"
  if [[ "$(dirname -- "$target_dir")" != "$scratch_root" ]]; then
    echo "public-mirror-gate: ERROR explicit target must be an immediate child of $scratch_root: $target_dir" >&2
    exit 2
  fi
  if calyx_gate_run bash "$dev_root/scripts/cargo-target-clean.sh" \
    --repo-root "$public_root" \
    --target-dir "$target_dir" \
    --gate-lock-fd "$gate_lock_fd" \
    --also-workspace-target \
    --also-repo-targets \
    --also-calyx-home-target; then
    :
  else
    setup_status=$?
    echo "public-mirror-gate: ERROR explicit target preclean failed: status=$setup_status target=$target_dir" >&2
    exit "$setup_status"
  fi
  calyx_gate_signals_defer
  if calyx_gate_create_owned_dir target_created target_root_identity target_identity \
    "$calyx_home" bash "$dev_root/scripts/check-tmpdir.sh" create-target-path-at \
    "$scratch_root" "$target_dir" >/dev/null; then
    target_cleanup_ready=1
    target_created=0
  else
    setup_status=$?
    if [[ "$target_created" -eq 1 ]]; then
      target_cleanup_ready=1
      target_created=0
    fi
    calyx_gate_signals_publish
    exit "$setup_status"
  fi
  calyx_gate_signals_publish
fi
public_cleanup_ready=1

export CARGO_INCREMENTAL=0
export CARGO_TARGET_DIR="$target_dir"
export CALYX_GATE_TARGET_ROOT="$scratch_root"
export CALYX_GATE_TARGET_ROOT_IDENTITY="$target_root_identity"
export CALYX_GATE_TARGET_DIR="$target_dir"
export CALYX_GATE_TARGET_IDENTITY="$target_identity"

echo "public-mirror-gate: dev_root=$dev_root"
echo "public-mirror-gate: public_root=$public_root"
echo "public-mirror-gate: TMPDIR=$TMPDIR"
echo "public-mirror-gate: target_dir=$CARGO_TARGET_DIR"

cleanup_public_scratch
reject_unstaged_public_changes "before gate" before
calyx_gate_run_in "$public_root" bash "$dev_root/scripts/secret-scan.sh" --staged
calyx_gate_run bash "$dev_root/scripts/public-leak-scan.sh" --public-tree "$public_root"
if calyx_gate_run git -C "$public_root" diff --cached --check; then
  :
else
  setup_status=$?
  echo "public-mirror-gate: ERROR staged diff check failed: status=$setup_status" >&2
  exit "$setup_status"
fi
calyx_gate_run bash "$dev_root/scripts/cargo-fmt-workspace.sh" --check --repo-root "$public_root"
calyx_gate_run_in "$public_root" cargo check --workspace --all-targets
calyx_gate_run_in "$public_root" cargo clippy --workspace --all-targets -- -D warnings
calyx_gate_run_in "$public_root" bash "$dev_root/scripts/fuzz-gate.sh" --repo-root "$public_root"
calyx_gate_run_in "$public_root" cargo nextest run --workspace
calyx_gate_run_in "$public_root" cargo test --workspace --doc
calyx_gate_run bash "$dev_root/scripts/orphan_rs.sh" --repo-root "$public_root"
calyx_gate_run bash "$dev_root/scripts/linecount.sh" --repo-root "$public_root"
cleanup_public_scratch
reject_unstaged_public_changes "after gate" after
