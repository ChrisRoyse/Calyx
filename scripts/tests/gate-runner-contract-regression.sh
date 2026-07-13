#!/usr/bin/env bash
# Source-only runner configuration and cgroup-containment mutations (#1421).
[[ "${BASH_SOURCE[0]}" != "$0" ]] || {
  echo "source this module from check-tmpdir-regression.sh" >&2
  exit 64
}

echo
echo "=================================================================="
echo "RUNNER CONTRACTS - grace, cgroup publication, and escape drain"
echo "=================================================================="
runner_contract_root="$work/runner-contracts"
mkdir -p "$runner_contract_root"
runner_contract_had_calyx_home=0
runner_contract_previous_calyx_home="${CALYX_HOME-}"
[[ -z "${CALYX_HOME+x}" ]] || runner_contract_had_calyx_home=1
mkdir -p "$runner_contract_root/channel-home"
chmod 700 "$runner_contract_root/channel-home"
export CALYX_HOME="$runner_contract_root/channel-home"
for invalid_grace in -1 30.1 31 banana; do
  set +e
  CALYX_GATE_SIGNAL_GRACE_SECONDS="$invalid_grace" \
    timeout --signal=TERM --kill-after=1s 3s bash -c 'source "$1"' _ "$runner_src" \
    >"$runner_contract_root/grace-$invalid_grace.log" 2>&1
  grace_rc=$?
  set -e
  [[ "$grace_rc" -eq 64 ]] \
    && pass "invalid grace '$invalid_grace' fails closed with rc 64" \
    || bad "invalid grace '$invalid_grace' returned $grace_rc"
done

for valid_grace in 0 0.5 30 30.0; do
  CALYX_GATE_SIGNAL_GRACE_SECONDS="$valid_grace" \
    timeout --signal=TERM --kill-after=1s 3s bash -c 'source "$1"' _ "$runner_src" \
    >"$runner_contract_root/grace-valid-$valid_grace.log" 2>&1 \
    && pass "valid grace '$valid_grace' is accepted" \
    || bad "valid grace '$valid_grace' was rejected"
done

monitor_marker="$runner_contract_root/monitor-child-ran"
set +e
env -u CALYX_GATE_RUN_SLICE -u CALYX_GATE_RUN_SLICE_OWNER_PID -u CALYX_GATE_RUN_SLICE_OWNER_STARTTIME \
  CALYX_GATE_SIGNAL_GRACE_SECONDS=0.5 RUNNER_SRC="$runner_src" MONITOR_MARKER="$monitor_marker" \
  timeout --signal=TERM --kill-after=1s 5s bash -c '
    set -uo pipefail
    source "$RUNNER_SRC"
    set -m
    set +e
    calyx_gate_run bash -c '\''printf child >"$1"'\'' _ "$MONITOR_MARKER"
    rc=$?
    set +m
    [[ "$rc" -eq 125 && ! -e "$MONITOR_MARKER" ]]
  ' >"$runner_contract_root/monitor.log" 2>&1
monitor_rc=$?
set -e
[[ "$monitor_rc" -eq 0 ]] \
  && pass "monitor mode is rejected before a stage can launch" \
  || bad "monitor-mode mutation launched or returned rc=$monitor_rc"

set +e
env -u CALYX_GATE_RUN_SLICE -u CALYX_GATE_RUN_SLICE_OWNER_PID \
  -u CALYX_GATE_RUN_SLICE_OWNER_STARTTIME RUNNER_SRC="$runner_src" STOPPED_PGID_MARKER="$runner_contract_root/stopped-pgid-called" \
  timeout --signal=TERM --kill-after=1s 8s bash -c '
    set -uo pipefail
    source "$RUNNER_SRC"
    ps() {
      if [[ "${1:-}" == -o && "${2:-}" == pgid= && "${3:-}" == -p ]]; then
        state="$(awk "/^State:/{print \$2}" "/proc/${4:?}/status" 2>/dev/null)" || return 1
        [[ "$calyx_gate_active_invocation" =~ ^[0-9a-f]{32}$ && ( "$state" == T || "$state" == t ) ]] || return 1; printf "invocation=%s state=%s\n" "$calyx_gate_active_invocation" "$state" >"$STOPPED_PGID_MARKER"
      fi
      command ps "$@"
    }
    set +e
    calyx_gate_run bash -c "sleep 0.05"
    rc=$?
    calyx_gate_finalize || rc=125
    exit "$rc"
  ' >"$runner_contract_root/stopped-pgid.log" 2>&1
stopped_pgid_rc=$?
set -e
[[ "$stopped_pgid_rc" -eq 0 && -s "$runner_contract_root/stopped-pgid-called" ]] \
  && pass "stage PGID is captured while the identity-bound leader is stopped" \
  || bad "post-CONT PGID lookup race returned rc=$stopped_pgid_rc"

fake_systemd_bin="$runner_contract_root/fake-systemd-bin"
mkdir -p "$fake_systemd_bin"
cat >"$fake_systemd_bin/systemd-run" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
while [[ $# -gt 0 && "$1" == --* ]]; do shift; done
raw="$(<"/proc/$BASHPID/stat")"; tail="${raw##*) }"; read -r -a fields <<<"$tail"
printf 'pid=%s starttime=%s\n' "$BASHPID" "${fields[19]}" >"${FAKE_SYSTEMD_PID:?}"
exec "$@"
STUB
chmod +x "$fake_systemd_bin/systemd-run"
publication_marker="$runner_contract_root/fake-systemd-stage-ran"
publication_pid_file="$runner_contract_root/fake-systemd-pid"
set +e
env -u CALYX_GATE_RUN_SLICE -u CALYX_GATE_RUN_SLICE_OWNER_PID -u CALYX_GATE_RUN_SLICE_OWNER_STARTTIME \
  PATH="$fake_systemd_bin:$PATH" RUNNER_SRC="$runner_src" \
  PUBLICATION_MARKER="$publication_marker" FAKE_SYSTEMD_PID="$publication_pid_file" \
  CALYX_GATE_SIGNAL_GRACE_SECONDS=0.1 \
  timeout --signal=TERM --kill-after=1s 5s bash -c '
    set -uo pipefail
    source "$RUNNER_SRC"
    calyx_gate_run bash -c '\''printf ran >"$1"'\'' _ "$PUBLICATION_MARKER"
  ' >"$runner_contract_root/fake-systemd.log" 2>&1
publication_rc=$?
set -e
publication_pid="$(sed -n 's/^pid=\([0-9]*\).*/\1/p' "$publication_pid_file" 2>/dev/null | head -n1 || true)"
publication_starttime="$(sed -n 's/.* starttime=\([0-9]*\)$/\1/p' "$publication_pid_file" 2>/dev/null | head -n1 || true)"
publication_pid_absent=0
[[ "$publication_pid" =~ ^[1-9][0-9]*$ && "$publication_starttime" =~ ^[1-9][0-9]*$ ]] \
  && gate_test_pid_identity_gone "$publication_pid" "$publication_starttime" && publication_pid_absent=1
if [[ "$publication_pid_absent" -eq 0 ]]; then
  gate_test_kill_exact_bounded "$publication_pid" "$publication_starttime" || true
fi
[[ "$publication_rc" -eq 125 && ! -e "$publication_marker" \
  && "$publication_pid_absent" -eq 1 ]] \
  && pass "failed cgroup publication kills its exact probe before user stage execution" \
  || bad "fake systemd-run rc=$publication_rc launched the stage"

no_stop_marker="$runner_contract_root/no-stop-stage-ran"
no_stop_state="$runner_contract_root/no-stop-state"
set +e
env -u CALYX_GATE_RUN_SLICE -u CALYX_GATE_RUN_SLICE_OWNER_PID -u CALYX_GATE_RUN_SLICE_OWNER_STARTTIME \
  RUNNER_SRC="$runner_src" NO_STOP_MARKER="$no_stop_marker" NO_STOP_STATE="$no_stop_state" \
  timeout --signal=TERM --kill-after=2s 8s bash -c '
    set -uo pipefail
    source "$RUNNER_SRC"
    calyx_gate_launch_scope() {
      local scope="$1" launch_read_fd="$3" launch_write_fd="$4" starttime
      calyx_gate_close_numeric_fd "$launch_read_fd" || exit 125
      calyx_gate_pid_starttime starttime "$BASHPID" || exit 125
      printf "CALYX_GATE_LAUNCH_ID_V1 pid=%s starttime=%s\n" \
        "$BASHPID" "$starttime" >&"$launch_write_fd" || exit 125
      calyx_gate_close_numeric_fd "$launch_write_fd" || exit 125
      kill -STOP "$BASHPID" || exit 125
      exec setsid -- systemd-run --user --scope --quiet --collect \
        --slice="$CALYX_GATE_RUN_SLICE" --unit="$scope" \
        --property=KillMode=control-group --property=SendSIGKILL=yes \
        bash -c "sleep 30"
    }
    calyx_gate_run bash -c '\''printf ran >"$1"'\'' _ "$NO_STOP_MARKER"
    rc=$?
    calyx_gate_discover_owned_slice 1 || rc=125
    printf "slice=%s control=%s\n" "$CALYX_GATE_RUN_SLICE" \
      "$calyx_gate_run_slice_control" >"$NO_STOP_STATE"
    calyx_gate_finalize || rc=125
    exit "$rc"
  ' >"$runner_contract_root/no-stop.log" 2>&1
no_stop_rc=$?
set -e
no_stop_control="$(sed -n 's/.* control=\(.*\)$/\1/p' "$no_stop_state" 2>/dev/null | head -n1 || true)"
[[ "$no_stop_rc" -eq 125 && ! -e "$no_stop_marker" \
  && "$no_stop_control" == */calyxgate-u*.slice \
  && ! -e "/sys/fs/cgroup$no_stop_control" ]] \
  && grep -Fq 'stage did not stop for cgroup publication' "$runner_contract_root/no-stop.log" \
  && grep -Fq 'run_slice_released unit=' "$runner_contract_root/no-stop.log" \
  && pass "pre-publication STOP failure kills the scope and releases its run slice" \
  || {
    bad "pre-publication STOP failure rc=$no_stop_rc leaked or launched user code"
    sed -n '1,30p' "$runner_contract_root/no-stop.log" | sed 's/^/    no-stop: /'
    printf '    no-stop-control=%s marker=%s\n' "$no_stop_control" "$no_stop_marker"
  }

nested_record_ready="$runner_contract_root/nested-record-ready"
nested_record_survived="$runner_contract_root/nested-record-survived"
nested_record_state="$runner_contract_root/nested-record-state"
nested_record_script="$runner_contract_root/nested-record.sh"
cat >"$nested_record_script" <<'STUB'
#!/usr/bin/env bash
set -uo pipefail
source "$RUNNER_SRC"
calyx_gate_read_unit_field() {
  local output_var="$1" unit="$2" field="$3" value
  if [[ "$unit" == *.scope && "$field" == InvocationID ]]; then
    calyx_gate_cgroup_error "injected nested scope InvocationID failure"
    return 125
  fi
  value="$(systemctl --user show "$unit" --property="$field" --value 2>/dev/null)" || return 125
  printf -v "$output_var" '%s' "$value"
}
calyx_gate_launch_scope() {
  local scope="$1" launch_read_fd="$3" launch_write_fd="$4" starttime
  calyx_gate_close_numeric_fd "$launch_read_fd" || exit 125
  calyx_gate_pid_starttime starttime "$BASHPID" || exit 125
  printf 'CALYX_GATE_LAUNCH_ID_V1 pid=%s starttime=%s\n' \
    "$BASHPID" "$starttime" >&"$launch_write_fd" || exit 125
  calyx_gate_close_numeric_fd "$launch_write_fd" || exit 125
  kill -STOP "$BASHPID" || exit 125
  exec setsid -- systemd-run --user --scope --quiet --collect \
    --slice="$CALYX_GATE_RUN_SLICE" --unit="$scope" \
    --property=KillMode=control-group --property=SendSIGKILL=yes \
    env --default-signal=HUP,INT,TERM bash -c '
      trap "" HUP INT TERM
      bash -c "trap \"\" HUP INT TERM; while :; do sleep 60; done" &
      child=$!
      control="$(awk -F: '\''$1=="0"{print $3}'\'' /proc/self/cgroup)"
      printf "child=%s control=%s\n" "$child" "$control" >"$NESTED_RECORD_READY"
      kill -STOP "$BASHPID"
      wait
    '
}
set +e
calyx_gate_run bash -c 'exit 0'
inner_rc=$?
set -e
printf 'survived rc=%s\n' "$inner_rc" >"$NESTED_RECORD_SURVIVED"
exit "$inner_rc"
STUB
chmod +x "$nested_record_script"
set +e
env -u CALYX_GATE_RUN_SLICE -u CALYX_GATE_RUN_SLICE_OWNER_PID \
  -u CALYX_GATE_RUN_SLICE_OWNER_STARTTIME RUNNER_SRC="$runner_src" \
  NESTED_RECORD_READY="$nested_record_ready" \
  NESTED_RECORD_SURVIVED="$nested_record_survived" \
  NESTED_RECORD_SCRIPT="$nested_record_script" \
  timeout --signal=TERM --kill-after=2s 12s bash -c '
    set -uo pipefail
    source "$RUNNER_SRC"
    calyx_gate_run bash "$NESTED_RECORD_SCRIPT"
    rc=$?
    printf "slice=%s control=%s\n" "$CALYX_GATE_RUN_SLICE" \
      "$calyx_gate_run_slice_control" >"$1"
    calyx_gate_finalize || rc=125
    exit "$rc"
  ' _ "$nested_record_state" >"$runner_contract_root/nested-record.log" 2>&1
nested_record_rc=$?
set -e
nested_record_child="$(sed -n 's/^child=\([0-9]*\).*/\1/p' "$nested_record_ready" | head -n1)"
nested_record_control="$(sed -n 's/.* control=\(.*\)$/\1/p' "$nested_record_ready" | head -n1)"
nested_record_slice_control="$(sed -n 's/.* control=\(.*\)$/\1/p' "$nested_record_state" | head -n1)"
[[ "$nested_record_rc" -eq 125 && -s "$nested_record_survived" \
  && "$nested_record_child" =~ ^[1-9][0-9]*$ && ! -e "/proc/$nested_record_child" \
  && -n "$nested_record_control" && ! -e "/sys/fs/cgroup$nested_record_control" \
  && -n "$nested_record_slice_control" && ! -e "/sys/fs/cgroup$nested_record_slice_control" ]] \
  && grep -Fq 'injected nested scope InvocationID failure' "$runner_contract_root/nested-record.log" \
  && pass "nested publication failure kills only its exact scope and releases the outer slice" \
  || bad "nested publication failure rc=$nested_record_rc leaked or killed the outer stage"

detached_stage="$runner_contract_root/detached-stage.sh"
detached_ready="$runner_contract_root/detached-ready"
detached_probe="$runner_contract_root/detached.lock"
cat >"$detached_stage" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
setsid bash -c '
  trap "" HUP INT TERM
  exec {probe_fd}>>"$1"
  flock --fcntl --exclusive "$probe_fd"
  cgroup="$(awk -F: '\''$1=="0"{print $3}'\'' /proc/self/cgroup)"
  raw="$(<"/proc/$BASHPID/stat")"; tail="${raw##*) }"; read -r -a fields <<<"$tail"
  printf "pid=%s starttime=%s cgroup=%s cgroup_id=%s\n" "$BASHPID" "${fields[19]}" "$cgroup" \
    "$(stat -c "%d:%i" -- "/sys/fs/cgroup$cgroup")" >"$2"
  while :; do sleep 60; done
' _ "${DETACHED_PROBE:?}" "${DETACHED_READY:?}" &
for _ in {1..100}; do
  [[ -s "$DETACHED_READY" ]] && exit 0
  sleep 0.01
done
exit 124
STUB
chmod +x "$detached_stage"
set +e
env -u CALYX_GATE_RUN_SLICE -u CALYX_GATE_RUN_SLICE_OWNER_PID -u CALYX_GATE_RUN_SLICE_OWNER_STARTTIME \
  RUNNER_SRC="$runner_src" DETACHED_STAGE="$detached_stage" DETACHED_READY="$detached_ready" \
  DETACHED_PROBE="$detached_probe" CALYX_GATE_SIGNAL_GRACE_SECONDS=0.1 \
  timeout --signal=TERM --kill-after=2s 8s bash -c '
     set -uo pipefail
     source "$RUNNER_SRC"
     calyx_gate_run bash "$DETACHED_STAGE"
     rc=$?
     calyx_gate_finalize || rc=125
     exit "$rc"
   ' >"$runner_contract_root/detached.log" 2>&1
detached_rc=$?
set -e
detached_pid="$(sed -n 's/^pid=\([0-9]*\).*/\1/p' "$detached_ready" 2>/dev/null | head -n1 || true)"
detached_starttime="$(sed -n 's/.* starttime=\([0-9]*\).*/\1/p' "$detached_ready" 2>/dev/null | head -n1 || true)"
detached_cgroup="$(sed -n 's/.* cgroup=\([^ ]*\).*/\1/p' "$detached_ready" 2>/dev/null | head -n1 || true)"
detached_pid_absent=0 detached_cgroup_absent=0
[[ "$detached_pid" =~ ^[1-9][0-9]*$ && "$detached_starttime" =~ ^[1-9][0-9]*$ ]] \
  && gate_test_pid_identity_gone "$detached_pid" "$detached_starttime" && detached_pid_absent=1
[[ "$detached_cgroup" == */calyxgate-*.scope && ! -e "/sys/fs/cgroup$detached_cgroup" ]] && detached_cgroup_absent=1
if [[ "$detached_pid_absent" -eq 0 ]]; then
  gate_test_kill_exact_bounded "$detached_pid" "$detached_starttime" || true
fi
exec {detached_probe_fd}>>"$detached_probe"
set +e
flock --fcntl --exclusive --nonblock "$detached_probe_fd"
detached_probe_rc=$?
set -e
eval "exec ${detached_probe_fd}>&-"
[[ "$detached_rc" -eq 125 && "$detached_pid" =~ ^[1-9][0-9]*$ \
  && "$detached_pid_absent" -eq 1 && "$detached_probe_rc" -eq 0 \
  && "$detached_cgroup" == */calyxgate-*.scope \
  && "$detached_cgroup_absent" -eq 1 ]] \
  && grep -Fq 'stage left processes in scope=' "$runner_contract_root/detached.log" \
  && pass "setsid descendant is cgroup-drained and turns a false success into rc 125" \
  || bad "detached descendant rc=$detached_rc pid=${detached_pid:-missing} probe=$detached_probe_rc escaped"

metadata_stage="$runner_contract_root/metadata-stage.sh"
metadata_ready="$runner_contract_root/metadata-ready"
cat >"$metadata_stage" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
setsid bash -c '
  trap "" HUP INT TERM
  control="$(awk -F: '\''$1=="0"{print $3}'\'' /proc/self/cgroup)"
  raw="$(<"/proc/$BASHPID/stat")"; tail="${raw##*) }"; read -r -a fields <<<"$tail"
  printf "pid=%s starttime=%s control=%s identity=%s\n" "$BASHPID" "${fields[19]}" "$control" \
    "$(stat -c "%d:%i" -- "/sys/fs/cgroup$control")" >"$1"
  while :; do sleep 60; done
' _ "$METADATA_READY" &
for _ in {1..100}; do [[ -s "$METADATA_READY" ]] && exit 0; sleep 0.01; done
exit 124
STUB
chmod +x "$metadata_stage"
set +e
env -u CALYX_GATE_RUN_SLICE -u CALYX_GATE_RUN_SLICE_OWNER_PID \
  -u CALYX_GATE_RUN_SLICE_OWNER_STARTTIME RUNNER_SRC="$runner_src" \
  METADATA_STAGE="$metadata_stage" METADATA_READY="$metadata_ready" \
  METADATA_FORBIDDEN="$runner_contract_root/metadata-owned-drain-forbidden" \
  timeout --signal=TERM --kill-after=2s 8s bash -c '
    set -uo pipefail
    source "$RUNNER_SRC"
    calyx_gate_verify_active_unit() {
      calyx_gate_cgroup_error "injected active scope metadata failure"
      return 125
    }
    calyx_gate_drain_owned_slice() {
      printf "called\n" >"$METADATA_FORBIDDEN"
      return 125
    }
    calyx_gate_run bash "$METADATA_STAGE"
    rc=$?
    calyx_gate_finalize || rc=125
    exit "$rc"
  ' >"$runner_contract_root/metadata-failure.log" 2>&1
metadata_rc=$?
set -e
metadata_pid="$(sed -n 's/^pid=\([0-9]*\).*/\1/p' "$metadata_ready" 2>/dev/null | head -n1 || true)"
metadata_starttime="$(sed -n 's/.* starttime=\([0-9]*\).*/\1/p' "$metadata_ready" 2>/dev/null | head -n1 || true)"
metadata_control="$(sed -n 's/.* control=\([^ ]*\).*/\1/p' "$metadata_ready" 2>/dev/null | head -n1 || true)"
metadata_identity="$(sed -n 's/.* identity=\([^ ]*\).*/\1/p' "$metadata_ready" 2>/dev/null | head -n1 || true)"
metadata_pid_absent=0 metadata_cgroup_absent=0
[[ "$metadata_pid" =~ ^[1-9][0-9]*$ && "$metadata_starttime" =~ ^[1-9][0-9]*$ ]] \
  && gate_test_pid_identity_gone "$metadata_pid" "$metadata_starttime" && metadata_pid_absent=1
[[ "$metadata_control" == */calyxgate-*.scope \
  && ! -e "/sys/fs/cgroup$metadata_control" ]] && metadata_cgroup_absent=1
if [[ "$metadata_pid_absent" -eq 0 ]]; then
  gate_test_kill_exact_bounded "$metadata_pid" "$metadata_starttime" || true
fi
[[ "$metadata_rc" -eq 125 && "$metadata_pid" =~ ^[1-9][0-9]*$ \
  && "$metadata_pid_absent" -eq 1 && "$metadata_control" == */calyxgate-*.scope \
  && "$metadata_identity" =~ ^[0-9]+:[0-9]+$ \
  && "$metadata_cgroup_absent" -eq 1 \
  && ! -e "$runner_contract_root/metadata-owned-drain-forbidden" ]] \
  && grep -Fq 'injected active scope metadata failure' "$runner_contract_root/metadata-failure.log" \
  && grep -Fq 'using identity-bound cgroup.kill' "$runner_contract_root/metadata-failure.log" \
  && pass "metadata verification failure still kills the exact bound scope and returns 125" \
  || bad "metadata failure rc=$metadata_rc left pid=${metadata_pid:-missing} control=${metadata_control:-missing}"

owned_metadata_ready="$runner_contract_root/owned-metadata-ready"
owned_metadata_state="$runner_contract_root/owned-metadata-state"
owned_metadata_stage="$runner_contract_root/owned-metadata-stage.sh"
cat >"$owned_metadata_stage" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
trap '' HUP INT TERM
bash -c 'trap "" HUP INT TERM; while :; do sleep 60; done' &
child=$!
control="$(awk -F: '$1=="0"{print $3}' /proc/self/cgroup)"
raw="$(<"/proc/$child/stat")"; tail="${raw##*) }"; read -r -a fields <<<"$tail"
printf 'child=%s starttime=%s control=%s identity=%s\n' "$child" "${fields[19]}" "$control" \
  "$(stat -c '%d:%i' -- "/sys/fs/cgroup$control")" >"${OWNED_METADATA_READY:?}"
wait
STUB
chmod +x "$owned_metadata_stage"
set +e
env -u CALYX_GATE_RUN_SLICE -u CALYX_GATE_RUN_SLICE_OWNER_PID \
  -u CALYX_GATE_RUN_SLICE_OWNER_STARTTIME RUNNER_SRC="$runner_src" \
  OWNED_METADATA_READY="$owned_metadata_ready" OWNED_METADATA_STATE="$owned_metadata_state" \
  OWNED_METADATA_STAGE="$owned_metadata_stage" \
  timeout --signal=TERM --kill-after=2s 8s bash -c '
    set -uo pipefail
    source "$RUNNER_SRC"
    calyx_gate_cgroup_init || exit $?
    calyx_gate_prepare_scope scope
    launch_read_fd="" launch_write_fd="" trusted_starttime="" published_starttime=""
    calyx_gate_open_fd_channel launch_read_fd launch_write_fd "$CALYX_HOME" \
      "owned metadata launch identity" || exit $?
    calyx_gate_launch_scope "$scope" "" "$launch_read_fd" "$launch_write_fd" \
      bash "$OWNED_METADATA_STAGE" &
    pid=$!
    calyx_gate_active_pid="$pid"
    calyx_gate_active_pgid="$pid"
    calyx_gate_active_scope="$scope"
    calyx_gate_child_starttime trusted_starttime "$pid" "$BASHPID" || exit 125
    calyx_gate_active_starttime="$trusted_starttime"
    calyx_gate_close_numeric_fd "$launch_write_fd" || exit 125
    calyx_gate_read_launch_identity "$launch_read_fd" "$pid" published_starttime || exit 125
    calyx_gate_close_numeric_fd "$launch_read_fd" || exit 125
    [[ "$published_starttime" == "$trusted_starttime" ]] || exit 125
    kill -CONT "$pid" || exit 125
    calyx_gate_capture_scope_control "$pid" "$scope" || exit $?
    calyx_gate_record_scope "$pid" "$scope" || exit $?
    pgid="$(ps -o pgid= -p "$pid" 2>/dev/null | tr -d " ")" || exit 125
    [[ "$pgid" == "$pid" ]] || exit 125
    calyx_gate_active_pgid="$pgid"
    kill -CONT "$pid" || exit 125
    for _ in {1..100}; do [[ -s "$OWNED_METADATA_READY" ]] && break; sleep 0.01; done
    [[ -s "$OWNED_METADATA_READY" ]] || exit 124
    eval "$(declare -f calyx_gate_verify_owned_slice_unit | sed '\''1s/calyx_gate_verify_owned_slice_unit/calyx_gate_verify_owned_slice_unit_real/'\'')"
    owned_verify_failed=0
    calyx_gate_verify_owned_slice_unit() {
      if [[ "$owned_verify_failed" -eq 0 ]]; then
        owned_verify_failed=1
        calyx_gate_cgroup_error "injected owned slice metadata failure"
        return 125
      fi
      calyx_gate_verify_owned_slice_unit_real
    }
    slice_control="$calyx_gate_run_slice_control"
    calyx_gate_drain_owned_slice TERM "owned metadata regression"
    drain_rc=$?
    if calyx_gate_pid_starttime actual_starttime "$pid"; then
      state="$(awk "/^State:/{print \$2}" "/proc/$pid/status" 2>/dev/null)" || exit 125
      [[ "$actual_starttime" != "$published_starttime" || "$state" == Z ]] || exit 125
    fi
    wait "$pid" 2>/dev/null || true
    calyx_gate_clear_active_scope
    calyx_gate_finalize || exit 125
    printf "rc=%s control=%s\n" "$drain_rc" "$slice_control" >"$OWNED_METADATA_STATE"
    exit "$drain_rc"
  ' >"$runner_contract_root/owned-metadata.log" 2>&1
owned_metadata_rc=$?
set -e
owned_metadata_child="$(sed -n 's/^child=\([0-9]*\).*/\1/p' "$owned_metadata_ready" 2>/dev/null | head -n1 || true)"
owned_metadata_starttime="$(sed -n 's/.* starttime=\([0-9]*\).*/\1/p' "$owned_metadata_ready" 2>/dev/null | head -n1 || true)"
owned_metadata_control="$(sed -n 's/.* control=\([^ ]*\).*/\1/p' "$owned_metadata_ready" 2>/dev/null | head -n1 || true)"
owned_metadata_slice="$(sed -n 's/.* control=\(.*\)$/\1/p' "$owned_metadata_state" 2>/dev/null | head -n1 || true)"
for _ in {1..100}; do
  [[ ! "$owned_metadata_child" =~ ^[1-9][0-9]*$ || ! -e "/proc/$owned_metadata_child" ]] \
    && [[ -z "$owned_metadata_control" || ! -e "/sys/fs/cgroup$owned_metadata_control" ]] \
    && [[ -z "$owned_metadata_slice" || ! -e "/sys/fs/cgroup$owned_metadata_slice" ]] \
    && break
  sleep 0.01
done
owned_metadata_child_absent=0 owned_metadata_scope_absent=0 owned_metadata_slice_absent=0
[[ "$owned_metadata_child" =~ ^[1-9][0-9]*$ && "$owned_metadata_starttime" =~ ^[1-9][0-9]*$ ]] \
  && gate_test_pid_identity_gone "$owned_metadata_child" "$owned_metadata_starttime" && owned_metadata_child_absent=1
[[ "$owned_metadata_control" == */calyxgate-*.scope && ! -e "/sys/fs/cgroup$owned_metadata_control" ]] && owned_metadata_scope_absent=1
[[ "$owned_metadata_slice" == */calyxgate-u*.slice && ! -e "/sys/fs/cgroup$owned_metadata_slice" ]] && owned_metadata_slice_absent=1
if [[ "$owned_metadata_child_absent" -eq 0 ]]; then
  gate_test_kill_exact_bounded "$owned_metadata_child" "$owned_metadata_starttime" || true
fi
[[ "$owned_metadata_rc" -eq 125 && -s "$owned_metadata_state" \
  && "$owned_metadata_child_absent" -eq 1 && "$owned_metadata_scope_absent" -eq 1 \
  && "$owned_metadata_slice_absent" -eq 1 ]] \
  && grep -Fq 'injected owned slice metadata failure' "$runner_contract_root/owned-metadata.log" \
  && grep -Fq 'using identity-bound cgroup.kill' "$runner_contract_root/owned-metadata.log" \
  && pass "owned-slice metadata failure kills the exact slice, returns 125, and releases it" \
  || {
    bad "owned-slice metadata rc=$owned_metadata_rc left child/control state"
    printf '    owned-metadata: state=%s child=%s child_absent=%s scope=%q scope_absent=%s slice=%q slice_absent=%s\n' \
      "$(test -s "$owned_metadata_state" && cat "$owned_metadata_state" || echo missing)" \
      "${owned_metadata_child:-missing}" "$owned_metadata_child_absent" \
      "$owned_metadata_control" "$owned_metadata_scope_absent" \
      "$owned_metadata_slice" "$owned_metadata_slice_absent"
    tail -25 "$runner_contract_root/owned-metadata.log" | sed 's/^/    owned-metadata-log: /'
  }

runner_handoff_contracts="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)/gate-runner-handoff-regression.sh"
[[ -f "$runner_handoff_contracts" ]] || {
  echo "REGRESSION SETUP ERROR: missing $runner_handoff_contracts" >&2
  return 2
}
# shellcheck source=gate-runner-handoff-regression.sh
source "$runner_handoff_contracts"

invalid_slice_marker="$runner_contract_root/invalid-slice-stage-ran"
set +e
CALYX_GATE_RUN_SLICE=not-a-calyx-slice CALYX_GATE_RUN_SLICE_OWNER_PID=1 \
  RUNNER_SRC="$runner_src" INVALID_SLICE_MARKER="$invalid_slice_marker" \
  timeout --signal=TERM --kill-after=1s 3s bash -c 'source "$RUNNER_SRC"; calyx_gate_run bash -c '\''printf ran >"$1"'\'' _ "$INVALID_SLICE_MARKER"' \
  >"$runner_contract_root/invalid-slice.log" 2>&1
invalid_slice_rc=$?
set -e
[[ "$invalid_slice_rc" -eq 125 && ! -e "$invalid_slice_marker" ]] \
  && grep -Fq 'inherited gate run-slice marker is invalid' "$runner_contract_root/invalid-slice.log" \
  && pass "invalid inherited run-slice identity fails before launch" \
  || bad "invalid run-slice marker rc=$invalid_slice_rc reached the stage"

if [[ "$runner_contract_had_calyx_home" -eq 1 ]]; then
  export CALYX_HOME="$runner_contract_previous_calyx_home"
else
  unset CALYX_HOME
fi
