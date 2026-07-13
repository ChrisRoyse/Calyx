#!/usr/bin/env bash
# Source-only public HUP/INT cgroup containment scenarios for issue #1421.
[[ "${BASH_SOURCE[0]}" != "$0" ]] || {
  echo "source this module from gate-wrapper-public-regression.sh" >&2
  exit 64
}

for signal_case in HUP:129 INT:130; do
  signal_name="${signal_case%%:*}"
  expected_signal_rc="${signal_case##*:}"
  signal_key="${signal_name,,}"
  signal_log="$work/public/$signal_key.log"
  signal_stages="$work/public/$signal_key-stages.log"
  signal_ready="$work/public/$signal_key-ready"
  setsid env --default-signal=HUP,INT,TERM \
    -u CALYX_FSV_ROOT -u CALYX_GATE_RUN_SLICE -u CALYX_GATE_RUN_SLICE_OWNER_PID \
    -u CALYX_GATE_RUN_SLICE_OWNER_STARTTIME -u CALYX_GATE_TMPDIR -u CALYX_GATE_TMP_ROOT \
    -u CALYX_GATE_LOCK_PATH -u CALYX_GATE_LOCK_FD -u CARGO_TARGET_DIR -u TMPDIR \
    HOME="$public_dev/home" PATH="$public_dev/fakebin:$PATH" \
    CALYX_TMP_ROOT="$public_dev/global-tmp" \
    CALYX_TMP_MIN_FREE_INODE_PERCENT=0 CALYX_TMP_MIN_FREE_BLOCK_PERCENT=0 \
    CALYX_TARGET_SCRATCH_ROOT="$(dirname "$public_dev")/target-scratch" \
    CALYX_GATE_SIGNAL_GRACE_SECONDS=0.5 \
    FAKE_TMP_LOG="$signal_stages" FAKE_DELETE_BASELINE=0 FAKE_FAIL_STAGE= \
    FAKE_BLOCK_STAGE=check FAKE_BLOCK_SCOPE=gate FAKE_READY_FILE="$signal_ready" \
    FAKE_TARGET_CLEAN_FAIL_ONCE_FILE= FAKE_TARGET_CLEAN_LOG= \
    FAKE_DELAY_CREATE_PREFIX= FAKE_CREATE_READY= \
    FAKE_STUBBORN_CHILD=1 FAKE_REQUIRE_OWNED_MODE=1 \
    bash "$public_dev/scripts/public-mirror-gate.sh" --public-dir "$public_repo" \
    >"$signal_log" 2>&1 &
  signal_pid=$!
  owned_pids="$signal_pid"
  signal_gate_pgid="$(ps -o pgid= -p "$signal_pid" 2>/dev/null | tr -d ' ')"
  [[ "$signal_gate_pgid" == "$signal_pid" ]] && owned_pgids="$signal_gate_pgid"
  gate_test_wait_ready "$signal_ready" || true
  if [[ -s "$signal_ready" && "$signal_gate_pgid" == "$signal_pid" ]]; then
    read -r signal_stage_pid_field signal_stage_pgid_field signal_cgroup_field signal_cgroup_id_field \
      signal_child_pid_field signal_child_pgid_field signal_child_cgroup_field <"$signal_ready"
    signal_stage_pid="${signal_stage_pid_field#pid=}"
    signal_stage_pgid="${signal_stage_pgid_field#pgid=}"
    signal_cgroup="${signal_cgroup_field#cgroup=}"
    signal_cgroup_id="${signal_cgroup_id_field#cgroup_id=}"
    signal_child_pid="${signal_child_pid_field#child=}"
    signal_child_pgid="${signal_child_pgid_field#child_pgid=}"
    signal_child_cgroup="${signal_child_cgroup_field#child_cgroup=}"
    [[ "$signal_stage_pid" == "$signal_stage_pgid" ]] \
      && owned_pgids="$owned_pgids $signal_stage_pgid $signal_child_pgid"
    kill -s "$signal_name" "$signal_pid"
    signal_watchdog_marker="$work/public/$signal_key-watchdog-fired"
    (
      sleep 8
      printf 'fired\n' >"$signal_watchdog_marker"
      kill -KILL "$signal_pid" 2>/dev/null || true
      gate_test_stop_group_bounded "$signal_stage_pgid" || true
      [[ -n "$signal_child_pid" ]] && kill -KILL "$signal_child_pid" 2>/dev/null || true
    ) &
    signal_watchdog=$!
    set +e
    wait "$signal_pid"; signal_rc=$?
    set -e
    kill -KILL "$signal_watchdog" 2>/dev/null || true
    wait "$signal_watchdog" 2>/dev/null || true
    signal_tmp="$(sed -n 's/^public-mirror-gate: TMPDIR=//p' "$signal_log" | head -n1)"
    signal_target="$(sed -n 's/^public-mirror-gate: target_dir=//p' "$signal_log" | head -n1)"
    [[ "$signal_rc" -eq "$expected_signal_rc" && ! -e "$signal_watchdog_marker" \
      && -n "$signal_tmp" && ! -e "$signal_tmp" \
      && -n "$signal_target" && ! -e "$signal_target" \
      && "$signal_child_pgid" == "$signal_child_pid" \
      && "$signal_child_pgid" != "$signal_stage_pgid" \
      && "$signal_child_cgroup" == "$signal_cgroup" && ! -e "/proc/$signal_child_pid" ]] \
      && gate_test_cgroup_absent "$signal_cgroup" "$signal_cgroup_id" \
      && grep -Fq "forwarding $signal_name to run_slice=" "$signal_log" \
      && pass "public PID-only $signal_name drains its setsid cgroup and owned roots" \
      || bad "public PID-only $signal_name rc=$signal_rc or state remained"
  else
    bad "public $signal_name never reached a verified PID-only state"
    [[ "$signal_gate_pgid" == "$signal_pid" ]] \
      && gate_test_stop_group_bounded "$signal_gate_pgid" || kill -KILL "$signal_pid" 2>/dev/null || true
    wait "$signal_pid" 2>/dev/null || true
  fi
  owned_pids="" owned_pgids=""
done
