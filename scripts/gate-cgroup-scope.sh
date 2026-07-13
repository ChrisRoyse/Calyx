#!/usr/bin/env bash
# Source-only exact-scope capture and publication validation.

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  echo "gate-cgroup-scope: ERROR this file must be sourced" >&2
  exit 64
fi

calyx_gate_capture_scope_control() {
  local pid="$1" scope="$2" control="" path identity
  [[ "$scope" =~ ^calyxgate-r[[:alnum:]._-]+-p[0-9]+-s[0-9]{6}\.scope$ ]] || return 125
  for _ in {1..300}; do
    control="$(awk -F: '$1=="0"{print $3}' "/proc/$pid/cgroup" 2>/dev/null || true)"
    if [[ "$control" == /*/"$CALYX_GATE_RUN_SLICE"/"$scope" ]]; then
      path="/sys/fs/cgroup$control"
      if [[ -d "$path" && -f "$path/cgroup.events" && -w "$path/cgroup.kill" ]]; then
        identity="$(stat -c '%d:%i' -- "$path" 2>/dev/null)" || identity=""
        if [[ "$identity" =~ ^[0-9]+:[0-9]+$ ]]; then
          calyx_gate_active_pid="$pid"
          calyx_gate_active_pgid="$pid"
          calyx_gate_active_scope="$scope"
          calyx_gate_active_invocation=""
          calyx_gate_active_control="$control"
          calyx_gate_active_control_identity="$identity"
          return 0
        fi
      fi
    fi
    kill -0 "$pid" 2>/dev/null || break
    sleep 0.01
  done
  calyx_gate_cgroup_error "cannot identity-bind launched scope: pid=$pid scope=$scope control=${control:-missing}"
  return 125
}

calyx_gate_record_scope() {
  local pid="$1" scope="$2" state="" id invocation control active proc_control
  local actual_identity slice_control
  [[ "$calyx_gate_active_scope" == "$scope" && -n "$calyx_gate_active_control" ]] ||
    calyx_gate_capture_scope_control "$pid" "$scope" || return 125
  for _ in {1..300}; do
    state="$(awk '/^State:/{print $2}' "/proc/$pid/status" 2>/dev/null || true)"
    [[ "$state" == T ]] && break
    kill -0 "$pid" 2>/dev/null || break
    sleep 0.01
  done
  [[ "$state" == T ]] || {
    calyx_gate_cgroup_error "stage did not stop for cgroup publication: pid=$pid scope=$scope"
    return 125
  }
  calyx_gate_discover_owned_slice 1 || return 125
  slice_control="$calyx_gate_run_slice_control"
  calyx_gate_read_unit_field id "$scope" Id || return 125
  calyx_gate_read_unit_field invocation "$scope" InvocationID || return 125
  calyx_gate_read_unit_field control "$scope" ControlGroup || return 125
  calyx_gate_read_unit_field active "$scope" ActiveState || return 125
  [[ "$id" == "$scope" && "$invocation" =~ ^[0-9a-f]{32}$ \
    && "$control" == "$calyx_gate_active_control" && "$active" == active ]] || {
    calyx_gate_cgroup_error "scope publication mismatch: id=$id invocation=$invocation control=$control active=$active"
    return 125
  }
  calyx_gate_read_process_cgroup proc_control "$pid" || return 125
  [[ "$proc_control" == "$control" && "$control" == "$slice_control"/* ]] || {
    calyx_gate_cgroup_error "stage cgroup mismatch: pid=$pid slice=$slice_control expected=$control actual=$proc_control"
    return 125
  }
  actual_identity="$(stat -c '%d:%i' -- "/sys/fs/cgroup$control")" || return 125
  [[ "$actual_identity" == "$calyx_gate_active_control_identity" ]] || {
    calyx_gate_cgroup_error "scope cgroup identity changed during publication: scope=$scope"
    return 125
  }
  calyx_gate_active_invocation="$invocation"
}
