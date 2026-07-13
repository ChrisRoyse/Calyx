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
  local pending_signal="$calyx_gate_pending_signal"
  [[ "$calyx_gate_signals_deferred" -eq 1 ]] || {
    echo "${CALYX_GATE_STAGE_OWNER:-gate}: ERROR signal publish without defer" >&2
    return 125
  }
  calyx_gate_restore_trap "$calyx_gate_saved_hup_trap" HUP
  calyx_gate_restore_trap "$calyx_gate_saved_int_trap" INT
  calyx_gate_restore_trap "$calyx_gate_saved_term_trap" TERM
  calyx_gate_signals_deferred=0
  calyx_gate_pending_signal=""
  [[ -z "$pending_signal" ]] || kill -s "$pending_signal" "$BASHPID"
}

calyx_gate_abort_unpublished_scope() {
  local pid="$1" scope="${2:-$calyx_gate_active_scope}" cleanup_status=0 control="" identity=""
  if [[ -n "$calyx_gate_active_control" ]]; then
    control="$calyx_gate_active_control"
    identity="$calyx_gate_active_control_identity"
    calyx_gate_kill_control "$control" "$identity" || cleanup_status=125
  else
    kill -KILL "$pid" 2>/dev/null || [[ ! -e "/proc/$pid" ]] || cleanup_status=125
    wait "$pid" 2>/dev/null || true
    if [[ "$calyx_gate_owns_run_slice" -eq 1 ]]; then
      calyx_gate_discover_owned_slice 0 || cleanup_status=125
      control="$calyx_gate_run_slice_control"
      identity="$calyx_gate_run_slice_identity"
      if [[ -n "$control" ]]; then
        calyx_gate_kill_control "$control" "$identity" || cleanup_status=125
      fi
    fi
  fi
  wait "$pid" 2>/dev/null || true
  [[ ! -e "/proc/$pid" ]] || cleanup_status=125
  if [[ -n "$control" ]]; then
    calyx_gate_wait_control_gone "$control" "$identity" || cleanup_status=125
  fi
  if [[ "$cleanup_status" -eq 0 ]]; then
    calyx_gate_clear_active_scope
  else
    echo "${CALYX_GATE_STAGE_OWNER:-gate}: ERROR unpublished stage cleanup was not proven: pid=$pid scope=$calyx_gate_active_scope" >&2
  fi
  return "$cleanup_status"
}

calyx_gate_run_in() {
  local stage_dir="$1"
  shift
  local command_name="${1##*/}" publish_launch=0 scope pid status pgid
  local record_status=0
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
  calyx_gate_launch_scope "$scope" "$stage_dir" "$@" &
  pid=$!
  calyx_gate_active_pid="$pid"
  calyx_gate_active_pgid="$pid"
  calyx_gate_active_scope="$scope"
  calyx_gate_capture_scope_control "$pid" "$scope" || record_status=$?
  if [[ "$record_status" -eq 0 ]]; then
    calyx_gate_record_scope "$pid" "$scope" || record_status=$?
  fi
  if [[ "$record_status" -ne 0 ]]; then
    calyx_gate_abort_unpublished_scope "$pid" "$scope" || record_status=125
    [[ "$publish_launch" -eq 0 ]] || calyx_gate_signals_publish
    return "$record_status"
  fi
  kill -CONT "$pid" || {
    calyx_gate_abort_unpublished_scope "$pid" "$scope"
    [[ "$publish_launch" -eq 0 ]] || calyx_gate_signals_publish
    return 125
  }
  if [[ "$publish_launch" -eq 1 ]]; then calyx_gate_signals_publish; fi
  pgid="$(ps -o pgid= -p "$pid" 2>/dev/null | tr -d ' ')"
  echo "${CALYX_GATE_STAGE_OWNER:-gate}: owned_stage_started pid=$pid pgid=${pgid:-missing} scope=$scope cgroup=$calyx_gate_active_control command=$command_name" >&2

  while :; do
    if wait "$pid"; then status=0; else status=$?; fi
    if [[ "$calyx_gate_signals_deferred" -eq 1 && -n "$calyx_gate_pending_signal" ]]; then
      case "$calyx_gate_pending_signal:$status" in
        HUP:129)
          calyx_gate_drain_active_scope HUP "deferred-signal stage scope" || status=125
          [[ "$status" -eq 125 ]] || status=129
          break
          ;;
        INT:130)
          calyx_gate_drain_active_scope INT "deferred-signal stage scope" || status=125
          [[ "$status" -eq 125 ]] || status=130
          break
          ;;
        TERM:143)
          calyx_gate_drain_active_scope TERM "deferred-signal stage scope" || status=125
          [[ "$status" -eq 125 ]] || status=143
          break
          ;;
      esac
    fi
    break
  done
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
  local signal_name="$1" pid="$calyx_gate_active_pid"
  [[ -n "$pid" ]] || return 0
  if [[ "$calyx_gate_owns_run_slice" -eq 1 && -n "$calyx_gate_run_slice_control" ]]; then
    calyx_gate_drain_owned_slice "$signal_name" "run slice" || return 125
  else
    calyx_gate_drain_active_scope "$signal_name" "stage scope" || return 125
  fi
  wait "$pid" 2>/dev/null || true
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
