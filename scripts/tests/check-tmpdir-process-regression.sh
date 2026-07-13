#!/usr/bin/env bash
# Source-only process-lifecycle scenarios for issue #1421.
[[ "${BASH_SOURCE[0]}" != "$0" ]] || { echo "source this module from check-tmpdir-regression.sh" >&2; exit 64; }

echo
echo "=================================================================="
echo "SCENARIO 4 - TERM maps to 143 and removes the owned run root"
echo "=================================================================="
repo4="$work/signal/calyx/repo"
mkdir -p "$repo4"
setup_repo "$repo4"
log4="$work/signal/gate.log"
stages4="$work/signal/stages.log"
ready4="$work/signal/ready"
setsid env -u CALYX_FSV_ROOT -u CALYX_GATE_RUN_SLICE -u CALYX_GATE_RUN_SLICE_OWNER_PID \
  HOME="$repo4/home" PATH="$repo4/fakebin:$PATH" \
  CALYX_TMP_ROOT="$repo4/global-tmp" \
  CALYX_TMP_MIN_FREE_INODE_PERCENT=0 CALYX_TMP_MIN_FREE_BLOCK_PERCENT=0 \
  CALYX_TARGET_SCRATCH_ROOT="$(dirname "$repo4")/target-scratch" \
  CALYX_GATE_SIGNAL_GRACE_SECONDS=0.5 \
  FAKE_TMP_LOG="$stages4" FAKE_FAIL_STAGE= FAKE_DELETE_BASELINE=0 \
  FAKE_BLOCK_STAGE=check FAKE_BLOCK_SCOPE=fuzz FAKE_READY_FILE="$ready4" FAKE_DELAY_SECONDS= \
  FAKE_STUBBORN_CHILD=1 FAKE_LEAK_DESCENDANT=0 \
  FAKE_REQUIRE_OWNED_MODE=1 \
  bash "$repo4/scripts/check.sh" >"$log4" 2>&1 &
signal_pid=$!
owned_gate_pid="$signal_pid"
pgid4="$(ps -o pgid= -p "$signal_pid" 2>/dev/null | tr -d ' ')"
[[ "$pgid4" == "$signal_pid" ]] && owned_pgid="$pgid4"
gate_test_wait_ready "$ready4" || true
if [[ -f "$ready4" && "$pgid4" == "$signal_pid" ]]; then
  read -r stage_pid_field stage_pgid_field stage_cgroup_field stage_cgroup_id_field \
    child_pid_field child_pgid_field child_cgroup_field <"$ready4"
  stage_pid="${stage_pid_field#pid=}"
  stage_pgid="${stage_pgid_field#pgid=}"
  stage_cgroup="${stage_cgroup_field#cgroup=}"
  stage_cgroup_id="${stage_cgroup_id_field#cgroup_id=}"
  child_pid="${child_pid_field#child=}"
  child_pgid="${child_pgid_field#child_pgid=}"
  child_cgroup="${child_cgroup_field#child_cgroup=}"
  [[ "$stage_pgid" =~ ^[1-9][0-9]*$ ]] && owned_stage_pgid="$stage_pgid"
  signal_fuzz_pid="$(sed -n 's/^fuzz-gate: pid=\([0-9]*\) pgid=[0-9]*$/\1/p' "$log4" | head -n1)"
  [[ "$stage_pid" == "$stage_pgid" && "$signal_fuzz_pid" != "$stage_pid" \
    && "$stage_cgroup" == */calyxgate-*.scope ]] \
    && pass "nested fuzz Cargo owns a distinct scope and process group" \
    || bad "nested fuzz stage scope ownership is invalid"
  [[ -n "$child_pid" && "$child_pgid" == "$child_pid" \
    && "$child_pgid" != "$stage_pgid" && "$child_cgroup" == "$stage_cgroup" ]] \
    && pass "setsid descendant escaped the PGID but remained in the stage cgroup" \
    || bad "setsid descendant containment is invalid for stage pgid=$stage_pgid"
  # Signal only the gate shell. Its trap must immediately forward TERM to the
  # separately owned stage group; a group-wide kill would mask a missing trap.
  kill -TERM "$signal_pid"
  signal_watchdog_marker="$work/signal/watchdog-fired"
  (
    sleep 8
    printf 'fired\n' >"$signal_watchdog_marker"
    kill -TERM "$signal_pid" 2>/dev/null || true
    [[ -n "$owned_stage_pgid" ]] && kill -TERM -- "-$owned_stage_pgid" 2>/dev/null || true
    sleep 0.5
    kill -KILL "$signal_pid" 2>/dev/null || true
    [[ -n "$owned_stage_pgid" ]] && kill -KILL -- "-$owned_stage_pgid" 2>/dev/null || true
    [[ -n "$child_pid" ]] && kill -KILL "$child_pid" 2>/dev/null || true
  ) &
  signal_watchdog=$!
  set +e
  wait "$signal_pid"
  rc4=$?
  set -e
  kill -KILL "$signal_watchdog" 2>/dev/null || true
  wait "$signal_watchdog" 2>/dev/null || true
  [[ ! -e "$signal_watchdog_marker" ]] \
    && pass "TERM completed within the bounded regression watchdog" \
    || bad "TERM required the regression watchdog rescue"
  if gate_test_group_alive "$stage_pgid"; then
    bad "TERM left the owned stage group $stage_pgid alive"
    gate_test_stop_group_bounded "$stage_pgid" || true
  else
    pass "TERM reaped the leader and TERM-ignoring descendant group"
  fi
  [[ -z "$child_pid" || ! -e "/proc/$child_pid" ]] \
    && gate_test_cgroup_absent "$stage_cgroup" "$stage_cgroup_id" \
    && pass "stubborn descendant pid and exact stage cgroup are absent" \
    || bad "stubborn descendant pid/cgroup remains: pid=$child_pid cgroup=$stage_cgroup"
  owned_pgid=""
  owned_gate_pid=""
  owned_stage_pgid=""
  [[ "$rc4" -eq 143 ]] && pass "TERM exits with 143" || bad "TERM gate exited $rc4"
else
  [[ -n "$pgid4" && "$pgid4" == "$signal_pid" ]] \
    && gate_test_stop_group_bounded "$signal_pid" || kill -KILL "$signal_pid" 2>/dev/null || true
  wait "$signal_pid" 2>/dev/null || true
  owned_pgid=""
  owned_gate_pid=""
  bad "signal gate never reached an owned process-group ready state (pgid=$pgid4)"
fi
assert_gate_clean "$repo4" "$log4" "TERM"

for signal_case in HUP:129 INT:130; do
  signal_name="${signal_case%%:*}"
  expected_signal_rc="${signal_case##*:}"
  signal_key="${signal_name,,}"
  signal_repo="$work/signal-$signal_key/calyx/repo"
  mkdir -p "$signal_repo"
  setup_repo "$signal_repo"
  signal_log="$work/signal-$signal_key/gate.log"
  signal_stages="$work/signal-$signal_key/stages.log"
  signal_ready="$work/signal-$signal_key/ready"
  setsid env --default-signal=HUP,INT,TERM -u CALYX_FSV_ROOT -u CALYX_GATE_RUN_SLICE -u CALYX_GATE_RUN_SLICE_OWNER_PID \
    HOME="$signal_repo/home" PATH="$signal_repo/fakebin:$PATH" \
    CALYX_TMP_ROOT="$signal_repo/global-tmp" \
    CALYX_TMP_MIN_FREE_INODE_PERCENT=0 CALYX_TMP_MIN_FREE_BLOCK_PERCENT=0 \
    CALYX_TARGET_SCRATCH_ROOT="$(dirname "$signal_repo")/target-scratch" \
    CALYX_GATE_SIGNAL_GRACE_SECONDS=0.5 \
    FAKE_TMP_LOG="$signal_stages" FAKE_FAIL_STAGE= FAKE_DELETE_BASELINE=0 \
    FAKE_BLOCK_STAGE=check FAKE_BLOCK_SCOPE=gate FAKE_READY_FILE="$signal_ready" FAKE_DELAY_SECONDS= \
    FAKE_STUBBORN_CHILD=1 FAKE_LEAK_DESCENDANT=0 FAKE_LEAK_SCOPE=gate \
    FAKE_REQUIRE_OWNED_MODE=1 \
    bash "$signal_repo/scripts/check.sh" >"$signal_log" 2>&1 &
  signal_pid=$!
  owned_gate_pid="$signal_pid"
  signal_gate_pgid="$(ps -o pgid= -p "$signal_pid" 2>/dev/null | tr -d ' ')"
  [[ "$signal_gate_pgid" == "$signal_pid" ]] && owned_pgid="$signal_gate_pgid"
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
    [[ "$signal_stage_pid" == "$signal_stage_pgid" ]] && owned_stage_pgid="$signal_stage_pgid"
    kill -s "$signal_name" "$signal_pid"
    signal_watchdog_marker="$work/signal-$signal_key/watchdog-fired"
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
    signal_tmp="$(gate_tmp_from_log "$signal_log")"
    signal_target="$(gate_target_from_log "$signal_log")"
    [[ "$signal_rc" -eq "$expected_signal_rc" && ! -e "$signal_watchdog_marker" \
      && -n "$signal_tmp" && ! -e "$signal_tmp" \
      && -n "$signal_target" && ! -e "$signal_target" \
      && -n "$signal_child_pid" && "$signal_child_pgid" == "$signal_child_pid" \
      && "$signal_child_pgid" != "$signal_stage_pgid" \
      && "$signal_child_cgroup" == "$signal_cgroup" && ! -e "/proc/$signal_child_pid" ]] \
      && ! gate_test_group_alive "$signal_stage_pgid" \
      && gate_test_cgroup_absent "$signal_cgroup" "$signal_cgroup_id" \
      && grep -Fq "forwarding $signal_name to run_slice=" "$signal_log" \
      && pass "PID-only $signal_name drains the setsid descendant cgroup and both roots" \
      || bad "PID-only $signal_name rc=$signal_rc or owned process/scratch remained"
  else
    bad "$signal_name gate never reached a verified PID-only signal state"
    [[ "$signal_gate_pgid" == "$signal_pid" ]] \
      && gate_test_stop_group_bounded "$signal_gate_pgid" || kill -KILL "$signal_pid" 2>/dev/null || true
    wait "$signal_pid" 2>/dev/null || true
  fi
  owned_pgid="" owned_gate_pid="" owned_stage_pgid=""
done

echo
echo "=================================================================="
echo "SCENARIO 5 - normal leader exit with a live descendant fails closed"
echo "=================================================================="
repo5="$work/normal-descendant/calyx/repo"
mkdir -p "$repo5"
setup_repo "$repo5"
log5="$work/normal-descendant/gate.log"
stages5="$work/normal-descendant/stages.log"
ready5="$work/normal-descendant/ready"
set +e
env -u CALYX_FSV_ROOT -u CALYX_GATE_RUN_SLICE -u CALYX_GATE_RUN_SLICE_OWNER_PID \
  HOME="$repo5/home" PATH="$repo5/fakebin:$PATH" \
  CALYX_TMP_ROOT="$repo5/global-tmp" \
  CALYX_TMP_MIN_FREE_INODE_PERCENT=0 CALYX_TMP_MIN_FREE_BLOCK_PERCENT=0 \
  CALYX_TARGET_SCRATCH_ROOT="$(dirname "$repo5")/target-scratch" \
  CALYX_GATE_SIGNAL_GRACE_SECONDS=0.5 \
  FAKE_TMP_LOG="$stages5" FAKE_FAIL_STAGE= FAKE_DELETE_BASELINE=0 \
  FAKE_BLOCK_STAGE= FAKE_BLOCK_SCOPE=gate FAKE_READY_FILE="$ready5" FAKE_DELAY_SECONDS= \
  FAKE_STUBBORN_CHILD=0 FAKE_LEAK_DESCENDANT=1 \
  FAKE_LEAK_SCOPE=gate \
  FAKE_REQUIRE_OWNED_MODE=1 \
  timeout --signal=TERM --kill-after=2s 30s bash "$repo5/scripts/check.sh" >"$log5" 2>&1
rc5=$?
set -e
if [[ -s "$ready5" ]]; then
  read -r leak_stage_pid_field leak_pgid_field leak_cgroup_field leak_cgroup_id_field \
    leak_pid_field leak_child_pgid_field leak_child_cgroup_field <"$ready5"
  leak_stage_pid="${leak_stage_pid_field#pid=}"
  leak_pgid="${leak_pgid_field#pgid=}"
  leak_cgroup="${leak_cgroup_field#cgroup=}"
  leak_cgroup_id="${leak_cgroup_id_field#cgroup_id=}"
  leak_pid="${leak_pid_field#child=}"
  leak_child_pgid="${leak_child_pgid_field#child_pgid=}"
  leak_child_cgroup="${leak_child_cgroup_field#child_cgroup=}"
  owned_extra_pgids="$leak_pgid $leak_child_pgid"
  if [[ "$leak_stage_pid" == "$leak_pgid" && "$leak_pid" == "$leak_child_pgid" \
    && "$leak_pgid" != "$leak_child_pgid" && "$leak_cgroup" == "$leak_child_cgroup" \
    && ! -e "/proc/$leak_pid" ]] && gate_test_cgroup_absent "$leak_cgroup" "$leak_cgroup_id"; then
    pass "normal-return setsid descendant and exact cgroup were drained completely"
    owned_extra_pgids=""
  else
    bad "normal-return descendant survived pgid=$leak_pgid child=$leak_pid"
  fi
else
  bad "normal-return descendant readiness evidence is absent"
fi
[[ "$rc5" -eq 125 ]] \
  && grep -Fq 'stage left processes in scope=' "$log5" \
  && pass "normal-return descendant changes a green stage to lifecycle rc 125" \
  || bad "normal-return descendant rc=$rc5 or diagnostic missing"
! grep -Fq 'stage=cargo:clippy ' "$stages5" \
  && pass "no later stage ran after the lifecycle failure" \
  || bad "a later stage ran after the descendant leak"
assert_gate_clean "$repo5" "$log5" "normal-descendant"

repo5_nested="$work/normal-nested-descendant/calyx/repo"
mkdir -p "$repo5_nested"
setup_repo "$repo5_nested"
log5_nested="$work/normal-nested-descendant/gate.log"
stages5_nested="$work/normal-nested-descendant/stages.log"
ready5_nested="$work/normal-nested-descendant/ready"
set +e
env -u CALYX_FSV_ROOT -u CALYX_GATE_RUN_SLICE -u CALYX_GATE_RUN_SLICE_OWNER_PID \
  HOME="$repo5_nested/home" PATH="$repo5_nested/fakebin:$PATH" \
  CALYX_TMP_ROOT="$repo5_nested/global-tmp" \
  CALYX_TMP_MIN_FREE_INODE_PERCENT=0 CALYX_TMP_MIN_FREE_BLOCK_PERCENT=0 \
  CALYX_TARGET_SCRATCH_ROOT="$(dirname "$repo5_nested")/target-scratch" \
  CALYX_GATE_SIGNAL_GRACE_SECONDS=0.5 \
  FAKE_TMP_LOG="$stages5_nested" FAKE_FAIL_STAGE= FAKE_DELETE_BASELINE=0 \
  FAKE_BLOCK_STAGE= FAKE_BLOCK_SCOPE=gate FAKE_READY_FILE="$ready5_nested" FAKE_DELAY_SECONDS= \
  FAKE_STUBBORN_CHILD=0 FAKE_LEAK_DESCENDANT=1 FAKE_LEAK_SCOPE=fuzz \
  FAKE_REQUIRE_OWNED_MODE=1 \
  timeout --signal=TERM --kill-after=2s 30s bash "$repo5_nested/scripts/check.sh" \
  >"$log5_nested" 2>&1
rc5_nested=$?
set -e
if [[ -s "$ready5_nested" ]]; then
  read -r nested_stage_pid_field nested_leak_pgid_field nested_cgroup_field nested_cgroup_id_field \
    nested_leak_pid_field nested_leak_child_pgid_field nested_child_cgroup_field <"$ready5_nested"
  nested_stage_pid="${nested_stage_pid_field#pid=}"
  nested_leak_pgid="${nested_leak_pgid_field#pgid=}"
  nested_cgroup="${nested_cgroup_field#cgroup=}"
  nested_cgroup_id="${nested_cgroup_id_field#cgroup_id=}"
  nested_leak_pid="${nested_leak_pid_field#child=}"
  nested_leak_child_pgid="${nested_leak_child_pgid_field#child_pgid=}"
  nested_child_cgroup="${nested_child_cgroup_field#child_cgroup=}"
else
  nested_stage_pid="" nested_leak_pgid="" nested_cgroup="" nested_cgroup_id=""
  nested_leak_pid="" nested_leak_child_pgid="" nested_child_cgroup=""
fi
[[ "$rc5_nested" -eq 125 && -n "$nested_leak_pid" \
  && "$nested_stage_pid" == "$nested_leak_pgid" \
  && "$nested_leak_pid" == "$nested_leak_child_pgid" \
  && "$nested_leak_pgid" != "$nested_leak_child_pgid" \
  && "$nested_cgroup" == "$nested_child_cgroup" \
  && ! -e "/proc/$nested_leak_pid" ]] \
  && gate_test_cgroup_absent "$nested_cgroup" "$nested_cgroup_id" \
  && grep -Fq 'stage left processes in scope=' "$log5_nested" \
  && pass "normal nested-fuzz descendants are drained before wrapper scratch cleanup" \
  || bad "normal nested-fuzz descendant escaped or rc=$rc5_nested"
assert_gate_clean "$repo5_nested" "$log5_nested" "normal-nested-descendant"
