#!/usr/bin/env bash
# Source-only PID-targeted HUP/INT fuzz wrapper scenarios for issue #1421.
[[ "${BASH_SOURCE[0]}" != "$0" ]] || {
  echo "source this module from gate-wrapper-tmpdir-regression.sh" >&2
  exit 64
}

for signal_case in HUP:129 INT:130; do
  signal_name="${signal_case%%:*}"
  expected_signal_rc="${signal_case##*:}"
  signal_key="${signal_name,,}"
  signal_log="$work/fuzz/$signal_key.log"
  signal_stages="$work/fuzz/$signal_key-stages.log"
  signal_ready="$work/fuzz/$signal_key-ready"
  setsid env --default-signal=HUP,INT,TERM \
    -u CALYX_FSV_ROOT -u CALYX_GATE_RUN_SLICE -u CALYX_GATE_RUN_SLICE_OWNER_PID -u CALYX_GATE_TMPDIR -u CALYX_GATE_TMP_ROOT \
    -u CALYX_GATE_LOCK_PATH -u CALYX_GATE_LOCK_FD -u CARGO_TARGET_DIR -u TMPDIR \
    HOME="$fuzz_repo/home" PATH="$fuzz_repo/fakebin:$PATH" \
    CALYX_GATE_SIGNAL_GRACE_SECONDS=0.5 \
    FAKE_TMP_LOG="$signal_stages" FAKE_DELETE_BASELINE=0 FAKE_FAIL_STAGE= \
    FAKE_BLOCK_STAGE=check FAKE_BLOCK_SCOPE=fuzz FAKE_READY_FILE="$signal_ready" \
    FAKE_STUBBORN_CHILD=1 FAKE_REQUIRE_OWNED_MODE=1 \
    bash "$fuzz_repo/scripts/fuzz-gate.sh" >"$signal_log" 2>&1 &
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
    [[ "$signal_stage_pid" == "$signal_stage_pgid" ]] && owned_pgids="$owned_pgids $signal_stage_pgid $signal_child_pgid"
    kill -s "$signal_name" "$signal_pid"
    signal_watchdog_marker="$work/fuzz/$signal_key-watchdog-fired"
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
    signal_tmp="$(sed -n 's/^fuzz-gate: TMPDIR=//p' "$signal_log" | head -n1)"
    signal_target="$(sed -n 's/^fuzz-gate: CARGO_TARGET_DIR=//p' "$signal_log" | head -n1)"
    [[ "$signal_rc" -eq "$expected_signal_rc" && ! -e "$signal_watchdog_marker" \
      && -n "$signal_tmp" && ! -e "$signal_tmp" \
      && -n "$signal_target" && ! -e "$signal_target" \
      && -n "$signal_child_pid" && "$signal_child_pgid" == "$signal_child_pid" \
      && "$signal_child_pgid" != "$signal_stage_pgid" \
      && "$signal_child_cgroup" == "$signal_cgroup" && ! -e "/proc/$signal_child_pid" ]] \
      && ! gate_test_group_alive "$signal_stage_pgid" \
      && gate_test_cgroup_absent "$signal_cgroup" "$signal_cgroup_id" \
      && grep -Fq "forwarding $signal_name to run_slice=" "$signal_log" \
      && pass "standalone fuzz PID-only $signal_name drains its setsid cgroup and roots" \
      || bad "standalone fuzz PID-only $signal_name rc=$signal_rc or state remained"
  else
    bad "standalone fuzz $signal_name never reached a verified PID-only state"
    [[ "$signal_gate_pgid" == "$signal_pid" ]] \
      && gate_test_stop_group_bounded "$signal_gate_pgid" || kill -KILL "$signal_pid" 2>/dev/null || true
    wait "$signal_pid" 2>/dev/null || true
  fi
  owned_pids="" owned_pgids=""
done

cleanup_block_log="$work/fuzz/cleanup-block.log"
cleanup_block_stages="$work/fuzz/cleanup-block-stages.log"
cleanup_block_stage_ready="$work/fuzz/cleanup-block-stage-ready"
cleanup_block_ready="$work/fuzz/cleanup-block-cleaner-ready"
cleanup_block_release="$work/fuzz/cleanup-block-release"
cleanup_stage_pgid=""
setsid env --default-signal=HUP,INT,TERM \
  -u CALYX_FSV_ROOT -u CALYX_GATE_RUN_SLICE -u CALYX_GATE_RUN_SLICE_OWNER_PID -u CALYX_GATE_TMPDIR -u CALYX_GATE_TMP_ROOT \
  -u CALYX_GATE_LOCK_PATH -u CALYX_GATE_LOCK_FD -u CARGO_TARGET_DIR -u TMPDIR \
  HOME="$fuzz_repo/home" PATH="$fuzz_repo/fakebin:$PATH" \
  CALYX_GATE_SIGNAL_GRACE_SECONDS=0.5 \
  FAKE_TMP_LOG="$cleanup_block_stages" FAKE_DELETE_BASELINE=0 FAKE_FAIL_STAGE= \
  FAKE_BLOCK_STAGE=check FAKE_BLOCK_SCOPE=fuzz FAKE_READY_FILE="$cleanup_block_stage_ready" \
  FAKE_STUBBORN_CHILD=0 FAKE_REQUIRE_OWNED_MODE=1 \
  FAKE_TARGET_CLEAN_BLOCK_READY="$cleanup_block_ready" \
  FAKE_TARGET_CLEAN_BLOCK_RELEASE="$cleanup_block_release" \
  bash "$fuzz_repo/scripts/fuzz-gate.sh" >"$cleanup_block_log" 2>&1 &
cleanup_block_pid=$!
owned_pids="$cleanup_block_pid" owned_pgids="$cleanup_block_pid"
cleanup_watchdog_marker="$work/fuzz/cleanup-block-watchdog-fired"
(
  sleep 12
  printf 'fired\n' >"$cleanup_watchdog_marker"
  kill -TERM -- "-$cleanup_block_pid" 2>/dev/null || true
  sleep 0.5
  kill -KILL -- "-$cleanup_block_pid" 2>/dev/null || true
) &
cleanup_watchdog=$!
gate_test_wait_ready "$cleanup_block_stage_ready" || true
if [[ -s "$cleanup_block_stage_ready" ]]; then
  kill -TERM "$cleanup_block_pid"
else
  bad "blocking-cleaner fixture never reached its stage barrier"
  kill -TERM "$cleanup_block_pid" 2>/dev/null || true
fi
gate_test_wait_ready "$cleanup_block_ready" || true
if [[ -s "$cleanup_block_ready" ]]; then
  read -r cleanup_pid_field cleanup_pgid_field _ <"$cleanup_block_ready"
  cleanup_stage_pid="${cleanup_pid_field#pid=}"
  cleanup_stage_pgid="${cleanup_pgid_field#pgid=}"
  owned_pgids="$owned_pgids $cleanup_stage_pgid"
  kill -HUP "$cleanup_block_pid" 2>/dev/null || true
  kill -INT "$cleanup_block_pid" 2>/dev/null || true
  [[ -e "/proc/$cleanup_block_pid" && "$cleanup_stage_pid" == "$cleanup_stage_pgid" \
    && -e "/proc/$cleanup_stage_pid" ]] \
    && pass "cleanup ignores reentrant signals while retaining its exact owned group" \
    || bad "cleanup wrapper or owned cleaner vanished before release"
  printf 'release\n' >"$cleanup_block_release"
else
  bad "signal cleanup never entered the blocking cleaner fixture"
  kill -TERM -- "-$cleanup_block_pid" 2>/dev/null || true
fi
set +e
wait "$cleanup_block_pid"; cleanup_block_rc=$?
set -e
kill -KILL "$cleanup_watchdog" 2>/dev/null || true
wait "$cleanup_watchdog" 2>/dev/null || true
cleanup_block_tmp="$(sed -n 's/^fuzz-gate: TMPDIR=//p' "$cleanup_block_log" | head -n1)"
cleanup_block_target="$(sed -n 's/^fuzz-gate: CARGO_TARGET_DIR=//p' "$cleanup_block_log" | head -n1)"
[[ "$cleanup_block_rc" -eq 143 && ! -e "$cleanup_watchdog_marker" \
  && ! -e "$cleanup_block_tmp" && ! -e "$cleanup_block_target" \
  && -n "$cleanup_stage_pgid" ]] \
  && ! gate_test_group_alive "$cleanup_stage_pgid" \
  && pass "released signal cleanup reaps its helper group and both owned roots" \
  || bad "blocking cleanup rc=$cleanup_block_rc leaked, hung, or left an owned group"
owned_pids="" owned_pgids=""
