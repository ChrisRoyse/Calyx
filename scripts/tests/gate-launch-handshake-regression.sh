#!/usr/bin/env bash
# Source-only launcher identity and STOP/CONT ordering regressions.
[[ "${BASH_SOURCE[0]}" != "$0" ]] || {
  echo "source this module from gate-final-contracts-regression.sh" >&2
  exit 64
}

echo
echo "=================================================================="
echo "LAUNCH HANDSHAKE - fallback identity and observed STOP before CONT"
echo "=================================================================="

probe_home="$work/launch-probe-failure/calyx"
probe_marker="$work/launch-probe-failure/launcher"
probe_log="$work/launch-probe-failure/action.log"
mkdir -p "$probe_home"
chmod 700 "$probe_home"
set +e
RUNNER_SRC="$runner_src" CASE_HOME="$probe_home" PROBE_MARKER="$probe_marker" \
  timeout --signal=KILL 8s bash -c '
    set -uo pipefail
    export CALYX_HOME="$CASE_HOME"
    source "$RUNNER_SRC"
    calyx_gate_child_starttime() {
      local output_var="$1" pid="$2" expected_parent="$3" actual="" state="" ppid=""
      for _ in {1..200}; do
        if calyx_gate_pid_starttime actual "$pid"; then
          state="$(awk "/^State:/{print \$2}" "/proc/$pid/status" 2>/dev/null || true)"
          ppid="$(awk "/^PPid:/{print \$2}" "/proc/$pid/status" 2>/dev/null || true)"
          if [[ "$ppid" == "$expected_parent" && ( "$state" == T || "$state" == t ) ]]; then
            printf "pid=%s starttime=%s state=%s ppid=%s\n" \
              "$pid" "$actual" "$state" "$ppid" >"$PROBE_MARKER"
            return 125
          fi
        fi
        sleep 0.01
      done
      return 125
    }
    set +e
    calyx_gate_run /bin/true
    run_rc=$?
    calyx_gate_finalize
    finalize_rc=$?
    set -e
    read -r pid_field start_field _ _ <"$PROBE_MARKER" || exit 125
    pid="${pid_field#pid=}"
    starttime="${start_field#starttime=}"
    launcher_gone=0
    calyx_gate_pid_starttime actual "$pid" || launcher_gone=1
    active_empty=0
    [[ -z "$calyx_gate_active_pid$calyx_gate_active_scope$calyx_gate_active_starttime" ]] \
      && active_empty=1
    printf "run_rc=%s finalize_rc=%s launcher_gone=%s active_empty=%s pid=%s starttime=%s\n" \
      "$run_rc" "$finalize_rc" "$launcher_gone" "$active_empty" "$pid" "$starttime"
    if [[ "$launcher_gone" -eq 0 ]]; then
      calyx_gate_kill_exact_leader_bounded "$pid" "$starttime" || true
      wait "$pid" 2>/dev/null || true
    fi
    calyx_gate_clear_active_scope
    calyx_gate_release_owned_slice >/dev/null 2>&1 || true
    [[ "$run_rc" -eq 125 && "$finalize_rc" -eq 0 \
      && "$launcher_gone" -eq 1 && "$active_empty" -eq 1 ]]
  ' >"$probe_log" 2>&1
probe_rc=$?
set -e
[[ "$probe_rc" -eq 0 ]] \
  && pass "failed parent starttime probe reaps the capability-identified stopped launcher" \
  || bad "parent starttime probe failure left a stopped launcher or active state rc=$probe_rc"

stop_fixture="$work/launch-stop-order/fixture"
stop_home="$work/launch-stop-order/calyx"
stop_ready="$work/launch-stop-order/closed"
stop_release="$work/launch-stop-order/release"
stop_cont="$work/launch-stop-order/cont"
stop_order="$work/launch-stop-order/order"
stop_stage="$work/launch-stop-order/stage"
stop_log="$work/launch-stop-order/action.log"
mkdir -p "$stop_fixture" "$stop_home"
chmod 700 "$stop_home"
cp "$runner_src" "$runtime_src" "$capture_src" "$record_exec_src" \
  "$cgroup_src" "$cgroup_scope_src" "$safe_helper_src" "$stop_fixture/"
stop_hook="$work/launch-stop-order/hook"
cat >"$stop_hook" <<'HOOK'
  printf 'CLOSED\n' >>"${STOP_ORDER:?}"
  printf 'pid=%s phase=writer-closed\n' "$BASHPID" >"${STOP_READY:?}"
  while [[ ! -e "${STOP_RELEASE:?}" ]]; do sleep 0.01; done
HOOK
awk -v hook="$stop_hook" '
  { print }
  $0 == "  calyx_gate_close_numeric_fd \"$launch_write_fd\" || exit 125" {
    while ((getline line < hook) > 0) print line
    close(hook)
  }
' "$cgroup_src" >"$stop_fixture/gate-cgroup-runtime.sh"
chmod 755 "$stop_fixture/gate-cgroup-runtime.sh"

setsid env RUNNER_SRC="$stop_fixture/gate-stage-runner.sh" CASE_HOME="$stop_home" \
  STOP_READY="$stop_ready" STOP_RELEASE="$stop_release" STOP_CONT="$stop_cont" \
  STOP_ORDER="$stop_order" STOP_STAGE="$stop_stage" bash -c '
    set -uo pipefail
    export CALYX_HOME="$CASE_HOME"
    source "$RUNNER_SRC"
    record_stop_cont() {
      local pid="$1" phase=LATE_CONT
      [[ -e "$STOP_RELEASE" ]] || phase=EARLY_CONT
      printf "pid=%s phase=%s\n" "$pid" "$phase" >>"$STOP_CONT"
      printf "%s\n" "$phase" >>"$STOP_ORDER"
    }
    if declare -F calyx_gate_continue_exact_leader >/dev/null; then
      calyx_gate_continue_exact_leader() {
        record_stop_cont "$1"
        calyx_gate_signal_exact_leader_bounded "$1" "$2" CONT 0 125
      }
    else
      kill() {
        [[ "${1:-}" != -CONT ]] || record_stop_cont "${2:-}"
        command kill "$@"
      }
    fi
    set +e
    calyx_gate_run /usr/bin/touch "$STOP_STAGE"
    run_rc=$?
    calyx_gate_finalize
    finalize_rc=$?
    set -e
    printf "run_rc=%s finalize_rc=%s\n" "$run_rc" "$finalize_rc"
    [[ "$run_rc" -eq 0 && "$finalize_rc" -eq 0 ]]
  ' >"$stop_log" 2>&1 &
stop_pid=$!
owned_extra_pgids="$stop_pid"
gate_test_wait_ready "$stop_ready" || true
stop_pre_cont=0
for _ in {1..200}; do
  [[ ! -s "$stop_cont" ]] || { stop_pre_cont=1; break; }
  sleep 0.01
done
printf 'RELEASE\n' >>"$stop_order"
printf 'release\n' >"$stop_release"
stop_finished=0
for _ in {1..500}; do
  kill -0 "$stop_pid" 2>/dev/null || { stop_finished=1; break; }
  sleep 0.02
done
if [[ "$stop_finished" -eq 0 ]]; then gate_test_stop_group_bounded "$stop_pid" || true; fi
set +e
wait "$stop_pid"
stop_rc=$?
set -e
owned_extra_pgids=""
stop_order_prefix="$(sed -n '1,3p' "$stop_order" 2>/dev/null || true)"
if [[ -s "$stop_ready" && "$stop_pre_cont" -eq 0 && "$stop_finished" -eq 1 \
  && "$stop_rc" -eq 0 && -s "$stop_cont" && -e "$stop_stage" \
  && "$stop_order_prefix" == $'CLOSED\nRELEASE\nLATE_CONT' \
  && ! "$(<"$stop_order")" == *EARLY_CONT* ]]; then
  pass "launcher CONT occurs only after the exact child is observed stopped"
else
  stop_order_readback="$(tr '\n' ',' <"$stop_order" 2>/dev/null || true)"
  bad "launcher close-to-STOP ordering rc=$stop_rc pre_cont=$stop_pre_cont finished=$stop_finished stage=$([[ -e "$stop_stage" ]] && echo yes || echo no) order=$stop_order_readback"
fi
