#!/usr/bin/env bash
# Source-only helpers for cgroup-contained gate stages and signal-atomic setup.

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  echo "gate-stage-runner: ERROR this file must be sourced" >&2
  exit 64
fi

gate_runner_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
# shellcheck source=gate-runtime-state.sh
source "$gate_runner_dir/gate-runtime-state.sh"
# shellcheck source=gate-cgroup-runtime.sh
source "$gate_runner_dir/gate-cgroup-runtime.sh"

calyx_gate_signals_deferred=0
calyx_gate_pending_signal=""
calyx_gate_saved_hup_trap=""
calyx_gate_saved_int_trap=""
calyx_gate_saved_term_trap=""
calyx_gate_signal_grace="${CALYX_GATE_SIGNAL_GRACE_SECONDS:-5}"
calyx_gate_kill_reap_ms=2000
if [[ ! "$calyx_gate_signal_grace" =~ ^(([0-9]|[12][0-9])(\.[0-9]+)?|30(\.0+)?)$ ]]; then
  echo "${CALYX_GATE_STAGE_OWNER:-gate}: ERROR CALYX_GATE_SIGNAL_GRACE_SECONDS must be between 0 and 30 seconds: $calyx_gate_signal_grace" >&2
  return 64
fi

calyx_gate_capture_deferred_signal() {
  local signal_name="$1"
  [[ -n "$calyx_gate_pending_signal" ]] || calyx_gate_pending_signal="$signal_name"
}

calyx_gate_restore_trap() {
  local saved="$1" signal_name="$2"
  if [[ -n "$saved" ]]; then eval "$saved"; else trap - "$signal_name"; fi
}

calyx_gate_signals_defer() {
  [[ "$calyx_gate_signals_deferred" -eq 0 ]] || {
    echo "${CALYX_GATE_STAGE_OWNER:-gate}: ERROR nested signal-deferred section" >&2
    return 125
  }
  calyx_gate_saved_hup_trap="$(trap -p HUP)"
  calyx_gate_saved_int_trap="$(trap -p INT)"
  calyx_gate_saved_term_trap="$(trap -p TERM)"
  calyx_gate_pending_signal=""
  calyx_gate_signals_deferred=1
  trap 'calyx_gate_capture_deferred_signal HUP' HUP
  trap 'calyx_gate_capture_deferred_signal INT' INT
  trap 'calyx_gate_capture_deferred_signal TERM' TERM
}

calyx_gate_signals_publish() {
  local pending_signal=""
  [[ "$calyx_gate_signals_deferred" -eq 1 ]] || {
    echo "${CALYX_GATE_STAGE_OWNER:-gate}: ERROR signal publish without defer" >&2
    return 125
  }
  calyx_gate_restore_trap "$calyx_gate_saved_hup_trap" HUP
  calyx_gate_restore_trap "$calyx_gate_saved_int_trap" INT
  calyx_gate_restore_trap "$calyx_gate_saved_term_trap" TERM
  pending_signal="$calyx_gate_pending_signal"
  calyx_gate_signals_deferred=0
  calyx_gate_pending_signal=""
  [[ -z "$pending_signal" ]] || kill -s "$pending_signal" "$BASHPID"
}

calyx_gate_signal_exact_leader_bounded() {
  local pid="$1" expected_starttime="$2" signal_name="$3" timeout_ms="$4"
  local absent_status="$5"
  [[ "$pid" =~ ^[1-9][0-9]*$ \
    && "$expected_starttime" =~ ^[1-9][0-9]*$ \
    && "$signal_name" =~ ^(CONT|KILL)$ && "$timeout_ms" =~ ^[0-9]+$ \
    && "$absent_status" =~ ^(0|125)$ ]] || return 125
  command -v python3 >/dev/null 2>&1 || return 127
  env --ignore-signal=HUP,INT,TERM python3 - "$pid" "$expected_starttime" \
    "$signal_name" "$timeout_ms" "$absent_status" <<'PY'
import os
import select
import signal
import sys

pid = int(sys.argv[1])
expected = sys.argv[2].encode("ascii")
requested = getattr(signal, f"SIG{sys.argv[3]}")
timeout_ms = int(sys.argv[4])
absent_status = int(sys.argv[5])
try:
    pidfd = os.pidfd_open(pid, 0)
except ProcessLookupError:
    raise SystemExit(absent_status)
except OSError:
    raise SystemExit(125)
try:
    try:
        with open(f"/proc/{pid}/stat", "rb") as source:
            raw = source.read(8192)
    except FileNotFoundError:
        raise SystemExit(absent_status)
    except OSError:
        raise SystemExit(125)
    end = raw.rfind(b") ")
    fields = raw[end + 2 :].split() if end >= 0 else []
    if len(fields) <= 19:
        raise SystemExit(125)
    if fields[19] != expected:
        raise SystemExit(3)
    try:
        signal.pidfd_send_signal(pidfd, requested)
    except ProcessLookupError:
        raise SystemExit(absent_status)
    except OSError:
        raise SystemExit(125)
    if timeout_ms:
        poller = select.poll()
        poller.register(pidfd, select.POLLIN)
        if not poller.poll(timeout_ms):
            raise SystemExit(125)
finally:
    os.close(pidfd)
PY
}

calyx_gate_kill_exact_leader_bounded() {
  calyx_gate_signal_exact_leader_bounded "$1" "$2" KILL \
    "$calyx_gate_kill_reap_ms" 0
}

calyx_gate_continue_exact_leader() {
  calyx_gate_signal_exact_leader_bounded "$1" "$2" CONT 0 125
}

calyx_gate_terminate_bound_stage() {
  local pid="$1" expected_starttime="$2" control="$3" identity="$4" reason="$5"
  local cleanup_status=0 leader_status=0
  calyx_gate_cgroup_error "identity-bound stage termination: reason=$reason pid=$pid control=${control:-missing}"
  if [[ -n "$control" && -n "$identity" ]]; then
    calyx_gate_kill_control "$control" "$identity" || cleanup_status=125
    calyx_gate_wait_control_gone "$control" "$identity" || cleanup_status=125
  fi
  if calyx_gate_kill_exact_leader_bounded "$pid" "$expected_starttime"; then
    wait "$pid" 2>/dev/null || true
  else
    leader_status=$?
    if [[ "$leader_status" -ne 3 ]]; then
      cleanup_status=125
    else
      calyx_gate_cgroup_error "stage leader identity changed before exact kill: pid=$pid"
    fi
  fi
  return "$cleanup_status"
}

calyx_gate_abort_deferred_handoff() {
  local pid="$1" expected_starttime="$2" reason="$3"
  calyx_gate_cgroup_error "deferred ownership proof failed: reason=$reason pid=$pid scope=$calyx_gate_active_scope"
  calyx_gate_terminate_bound_stage "$pid" "$expected_starttime" \
    "$calyx_gate_active_control" "$calyx_gate_active_control_identity" \
    "deferred-handoff-$reason"
}

calyx_gate_abort_unpublished_scope() {
  local pid="$1" scope="${2:-$calyx_gate_active_scope}" cleanup_status=0 control="" identity=""
  if [[ -n "$calyx_gate_active_control" ]]; then
    control="$calyx_gate_active_control"
    identity="$calyx_gate_active_control_identity"
  else
    if [[ "$calyx_gate_owns_run_slice" -eq 1 ]]; then
      calyx_gate_discover_owned_slice 0 || cleanup_status=125
      control="$calyx_gate_run_slice_control"
      identity="$calyx_gate_run_slice_identity"
    fi
  fi
  calyx_gate_terminate_bound_stage "$pid" "$calyx_gate_active_starttime" \
    "$control" "$identity" "unpublished-scope" || cleanup_status=125
  if [[ "$cleanup_status" -eq 0 ]]; then
    calyx_gate_clear_active_scope
  else
    echo "${CALYX_GATE_STAGE_OWNER:-gate}: ERROR unpublished stage cleanup was not proven: pid=$pid scope=$calyx_gate_active_scope" >&2
  fi
  return "$cleanup_status"
}

calyx_gate_read_launch_identity() {
  local read_fd="$1" expected_pid="$2" starttime_var="$3"
  local helper_fd="" matched_starttime="" read_status=0 close_status=0
  calyx_gate_open_identity_helper_fd helper_fd || return 125
  if matched_starttime="$(python3 /proc/self/fd/4 read-launch 3 1024 \
    "$calyx_gate_record_timeout_seconds" "$expected_pid" 3<&"$read_fd" 4<&"$helper_fd")"; then
    read_status=0
  else
    read_status=$?
  fi
  calyx_gate_close_numeric_fd "$helper_fd" || close_status=125
  if [[ "$matched_starttime" =~ ^[1-9][0-9]*$ ]]; then
    printf -v "$starttime_var" '%s' "$matched_starttime"
  else
    read_status=125
  fi
  [[ "$close_status" -eq 0 ]] || read_status=125
  return "$read_status"
}

calyx_gate_child_starttime() {
  local output_var="$1" pid="$2" expected_parent="$3" raw tail value
  local -a fields=()
  IFS= read -r raw 2>/dev/null <"/proc/$pid/stat" || return 125
  tail="${raw##*) }"
  [[ "$tail" != "$raw" ]] || return 125
  read -r -a fields <<<"$tail"
  value="${fields[19]:-}"
  [[ "${fields[0]:-}" =~ ^[RSDZTtXxKWPI]$ \
    && "${fields[1]:-}" == "$expected_parent" \
    && "$value" =~ ^[1-9][0-9]*$ ]] || return 125
  printf -v "$output_var" '%s' "$value"
}

calyx_gate_wait_stopped_child() {
  local pid="$1" expected_starttime="$2" expected_parent="$3" raw="" tail="" state=""
  local -a fields=()
  for _ in {1..300}; do
    if IFS= read -r raw 2>/dev/null <"/proc/$pid/stat"; then
      tail="${raw##*) }"
      [[ "$tail" != "$raw" ]] || return 125
      read -r -a fields <<<"$tail"
      state="${fields[0]:-}"
      [[ "${fields[1]:-}" == "$expected_parent" \
        && "${fields[19]:-}" == "$expected_starttime" ]] || {
          calyx_gate_cgroup_error "direct child identity changed before STOP: pid=$pid"
          return 125
        }
      [[ "$state" != T && "$state" != t ]] || return 0
    elif [[ ! -e "/proc/$pid" ]]; then
      break
    fi
    sleep 0.01
  done
  calyx_gate_cgroup_error "direct child did not reach STOP with exact identity: pid=$pid"
  return 125
}

calyx_gate_abort_stopped_launcher() {
  local pid="$1" expected_starttime="${2:-}" reason="$3"
  local kill_status=0
  if [[ ! "$expected_starttime" =~ ^[1-9][0-9]*$ ]]; then
    if [[ ! -e "/proc/$pid" ]]; then
      wait "$pid" 2>/dev/null || true
      calyx_gate_clear_active_scope
      return 0
    fi
    calyx_gate_cgroup_error "refusing to signal stopped launcher without its published identity: reason=$reason pid=$pid"
    return 125
  fi
  if calyx_gate_kill_exact_leader_bounded "$pid" "$expected_starttime"; then
    wait "$pid" 2>/dev/null || true
  else
    kill_status=$?
    if [[ "$kill_status" -ne 3 ]]; then
      calyx_gate_cgroup_error "cannot boundedly reap stopped launcher: reason=$reason pid=$pid"
      return 125
    fi
    calyx_gate_cgroup_error "stopped launcher identity changed before exact kill: reason=$reason pid=$pid"
  fi
  calyx_gate_clear_active_scope
}

calyx_gate_reset_unstarted_owned_slice() {
  [[ "$calyx_gate_owns_run_slice" -eq 1 ]] || return 0
  calyx_gate_cgroup_initialized=0
  calyx_gate_owns_run_slice=0
  calyx_gate_run_slice_control=""
  calyx_gate_run_slice_identity=""
  calyx_gate_run_slice_invocation=""
  unset CALYX_GATE_RUN_SLICE CALYX_GATE_RUN_SLICE_OWNER_PID
  unset CALYX_GATE_RUN_SLICE_OWNER_STARTTIME
}

calyx_gate_run_in() {
  local stage_dir="$1"
  shift
  local command_name="${1##*/}" publish_launch=0 scope pid status pgid
  local probe_status=0 record_status=0 pid_starttime="" trusted_starttime=""
  local cleanup_starttime=""
  local actual_starttime="" actual_control="" state=""
  local launch_read_fd="" launch_write_fd="" launch_parent="${CALYX_HOME:-}"
  local handoff_failed=0 handoff_reason="" reap_safe=0
  local deadline_us=0 now_us=0
  calyx_gate_handoff_timed_out=0
  [[ -z "$calyx_gate_active_pid" ]] || {
    echo "${CALYX_GATE_STAGE_OWNER:-gate}: ERROR nested stage while pid $calyx_gate_active_pid is active" >&2
    return 125
  }
  [[ "$-" != *m* ]] || {
    echo "${CALYX_GATE_STAGE_OWNER:-gate}: ERROR monitor mode is incompatible with stable stage ownership" >&2
    return 125
  }
  calyx_gate_cgroup_init || return $?
  if [[ "$calyx_gate_signals_deferred" -eq 0 ]]; then
    calyx_gate_signals_defer || return $?
    publish_launch=1
  fi
  calyx_gate_prepare_scope scope
  [[ -n "$launch_parent" ]] || {
    calyx_gate_cgroup_error "CALYX_HOME is required for the stage launch identity channel"
    [[ "$publish_launch" -eq 0 ]] || calyx_gate_signals_publish
    return 125
  }
  calyx_gate_open_fd_channel launch_read_fd launch_write_fd "$launch_parent" \
    "stage launch identity" || {
      [[ "$publish_launch" -eq 0 ]] || calyx_gate_signals_publish
      return 125
    }
  calyx_gate_launch_scope "$scope" "$stage_dir" "$launch_read_fd" "$launch_write_fd" "$@" &
  pid=$!
  calyx_gate_active_pid="$pid"
  calyx_gate_active_pgid="$pid"
  calyx_gate_active_scope="$scope"
  if calyx_gate_child_starttime trusted_starttime "$pid" "$BASHPID"; then
    calyx_gate_active_starttime="$trusted_starttime"
  else
    probe_status=125
  fi
  calyx_gate_close_numeric_fd "$launch_write_fd" || record_status=125
  calyx_gate_read_launch_identity "$launch_read_fd" "$pid" pid_starttime \
    || record_status=125
  calyx_gate_close_numeric_fd "$launch_read_fd" || record_status=125
  if [[ "$record_status" -eq 0 ]]; then
    calyx_gate_wait_stopped_child "$pid" "$pid_starttime" "$BASHPID" \
      || record_status=125
  fi
  if [[ "$record_status" -eq 0 ]]; then
    calyx_gate_pid_starttime actual_starttime "$pid" || record_status=125
  fi
  [[ "$record_status" -ne 0 || ( "$actual_starttime" == "$pid_starttime" \
    && ( -z "$trusted_starttime" || "$pid_starttime" == "$trusted_starttime" ) ) ]] \
      || record_status=125
  if [[ "$record_status" -eq 0 && "$probe_status" -ne 0 ]]; then
    calyx_gate_cgroup_error "direct child starttime probe failed before exact publication: pid=$pid"
    record_status=125
  fi
  if [[ "$record_status" -ne 0 ]]; then
    cleanup_starttime="$trusted_starttime"
    [[ "$cleanup_starttime" =~ ^[1-9][0-9]*$ ]] \
      || cleanup_starttime="$pid_starttime"
    if calyx_gate_abort_stopped_launcher "$pid" "$cleanup_starttime" launch-handshake; then
      calyx_gate_reset_unstarted_owned_slice
    else
      record_status=125
    fi
    [[ "$publish_launch" -eq 0 ]] || calyx_gate_signals_publish
    return 125
  fi
  calyx_gate_active_starttime="$pid_starttime"
  calyx_gate_continue_exact_leader "$pid" "$pid_starttime" || {
    if calyx_gate_abort_stopped_launcher "$pid" "$pid_starttime" launch-continue; then
      calyx_gate_reset_unstarted_owned_slice
    fi
    [[ "$publish_launch" -eq 0 ]] || calyx_gate_signals_publish
    return 125
  }
  calyx_gate_capture_scope_control "$pid" "$scope" || record_status=$?
  if [[ "$record_status" -eq 0 ]]; then
    calyx_gate_record_scope "$pid" "$scope" || record_status=$?
  fi
  if [[ "$record_status" -eq 0 ]]; then
    pgid="$(ps -o pgid= -p "$pid" 2>/dev/null | tr -d ' ')" || record_status=125
    [[ "$pgid" == "$pid" ]] || record_status=125
  fi
  if [[ "$record_status" -ne 0 ]]; then
    calyx_gate_abort_unpublished_scope "$pid" "$scope" || record_status=125
    [[ "$publish_launch" -eq 0 ]] || calyx_gate_signals_publish
    return "$record_status"
  fi
  calyx_gate_continue_exact_leader "$pid" "$pid_starttime" || {
    calyx_gate_abort_unpublished_scope "$pid" "$scope"
    [[ "$publish_launch" -eq 0 ]] || calyx_gate_signals_publish
    return 125
  }
  if [[ "$publish_launch" -eq 1 ]]; then calyx_gate_signals_publish; fi
  echo "${CALYX_GATE_STAGE_OWNER:-gate}: owned_stage_started pid=$pid pgid=${pgid:-missing} scope=$scope cgroup=$calyx_gate_active_control command=$command_name" >&2

  # A caller-deferred setup stage owns the signal replay boundary. A trapped
  # signal interrupts Bash wait even while the exact child remains alive, so
  # re-wait until that child is actually reaped. The caller can then consume the
  # ownership record before publishing its pending signal.
  if wait "$pid"; then status=0; else status=$?; fi
  if [[ "$calyx_gate_signals_deferred" -eq 1 && -n "$calyx_gate_pending_signal" \
    && -e "/proc/$pid" ]]; then
    deadline_us=$(( $(calyx_gate_now_us) + $(calyx_gate_grace_us) ))
    while [[ -e "/proc/$pid" ]]; do
      if ! calyx_gate_pid_starttime actual_starttime "$pid"; then
        [[ ! -e "/proc/$pid" ]] && { reap_safe=1; break; }
        handoff_failed=1; handoff_reason=starttime-unreadable; status=125
        break
      fi
      if [[ "$actual_starttime" != "$pid_starttime" ]]; then
        handoff_failed=1; handoff_reason=starttime-mismatch; status=125
        break
      fi
      calyx_gate_read_process_cgroup actual_control "$pid" || {
        [[ ! -e "/proc/$pid" ]] && break
        handoff_failed=1; handoff_reason=cgroup-unreadable; status=125
        break
      }
      if [[ "$actual_control" != "$calyx_gate_active_control" ]]; then
        handoff_failed=1; handoff_reason=cgroup-mismatch; status=125
        break
      fi
      if ! state="$(awk '/^State:/{print $2}' "/proc/$pid/status" 2>/dev/null)"; then
        [[ ! -e "/proc/$pid" ]] && { reap_safe=1; break; }
        handoff_failed=1; handoff_reason=state-unreadable; status=125
        break
      fi
      [[ "$state" != Z ]] || { reap_safe=1; break; }
      now_us="$(calyx_gate_now_us)"
      if ((now_us >= deadline_us)); then
        calyx_gate_handoff_timed_out=1
        calyx_gate_cgroup_error "deferred ownership handoff exceeded signal grace: pid=$pid scope=$scope"
        handoff_failed=1; handoff_reason=deadline; status=125
        break
      fi
      sleep 0.01
    done
    [[ -e "/proc/$pid" ]] || reap_safe=1
    if [[ "$handoff_failed" -eq 1 ]]; then
      calyx_gate_abort_deferred_handoff "$pid" "$pid_starttime" "$handoff_reason" || status=125
    elif [[ "$reap_safe" -eq 1 ]]; then
      if wait "$pid"; then status=0; else status=$?; fi
    else
      calyx_gate_cgroup_error "deferred ownership handoff reached an unproven reap state: pid=$pid scope=$scope"
      status=125
    fi
  fi
  calyx_gate_finish_active_scope "$status" || true
  status="$calyx_gate_finish_status"
  calyx_gate_finish_owned_slice "$status" || true
  status="$calyx_gate_finish_status"
  echo "${CALYX_GATE_STAGE_OWNER:-gate}: owned_stage_finished pid=$pid pgid=${pgid:-missing} scope=$scope status=$status command=$command_name" >&2
  return "$status"
}

calyx_gate_run() {
  calyx_gate_run_in "" "$@"
}

calyx_gate_stop_active() {
  local signal_name="$1" pid="$calyx_gate_active_pid" cleanup_status=0
  [[ -n "$pid" ]] || return 0
  if [[ "$calyx_gate_owns_run_slice" -eq 1 && -n "$calyx_gate_run_slice_control" ]]; then
    calyx_gate_drain_owned_slice "$signal_name" "run slice" || cleanup_status=125
  else
    calyx_gate_drain_active_scope "$signal_name" "stage scope" || cleanup_status=125
  fi
  calyx_gate_terminate_bound_stage "$pid" "$calyx_gate_active_starttime" \
    "$calyx_gate_active_control" "$calyx_gate_active_control_identity" \
    "signal-$signal_name" || cleanup_status=125
  [[ "$cleanup_status" -eq 0 ]] || return 125
  calyx_gate_clear_active_scope
  calyx_gate_assert_owned_slice_empty
}

calyx_gate_cleanup_safe() {
  if [[ -n "$calyx_gate_active_scope" && -z "$calyx_gate_active_invocation" ]]; then
    calyx_gate_abort_unpublished_scope "$calyx_gate_active_pid" || return $?
  fi
  if [[ -n "$calyx_gate_active_scope" ]]; then
    calyx_gate_control_population "$calyx_gate_active_control" \
      "$calyx_gate_active_control_identity" || return $?
    [[ "$calyx_gate_population" -eq 0 ]] || {
      echo "${CALYX_GATE_STAGE_OWNER:-gate}: ERROR refusing scratch cleanup while scope=$calyx_gate_active_scope remains populated" >&2
      return 125
    }
    calyx_gate_clear_active_scope
  fi
  calyx_gate_assert_owned_slice_empty
}

calyx_gate_finalize() {
  calyx_gate_cleanup_safe || return $?
  calyx_gate_release_owned_slice
}
