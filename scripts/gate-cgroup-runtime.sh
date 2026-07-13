#!/usr/bin/env bash
# Source-only cgroup-v2 containment for gate stages.

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  echo "gate-cgroup-runtime: ERROR this file must be sourced" >&2
  exit 64
fi

calyx_gate_cgroup_initialized=0
calyx_gate_scope_seq=0
calyx_gate_owns_run_slice=0
calyx_gate_active_pid=""
calyx_gate_active_pgid=""
calyx_gate_active_scope=""
calyx_gate_active_invocation=""
calyx_gate_active_control=""
calyx_gate_active_control_identity=""
calyx_gate_run_slice_control=""
calyx_gate_run_slice_identity=""
calyx_gate_run_slice_invocation=""
calyx_gate_population=""
calyx_gate_finish_status=125

calyx_gate_cgroup_error() {
  echo "${CALYX_GATE_STAGE_OWNER:-gate}: ERROR $*" >&2
}

calyx_gate_pid_starttime() {
  local output_var="$1" pid="$2" value
  value="$(awk '{print $22}' "/proc/$pid/stat" 2>/dev/null)" || return 125
  [[ "$value" =~ ^[1-9][0-9]*$ ]] || return 125
  printf -v "$output_var" '%s' "$value"
}

calyx_gate_now_us() {
  local now="$EPOCHREALTIME" seconds fraction
  seconds="${now%%.*}"
  fraction="${now#*.}000000"
  fraction="${fraction:0:6}"
  printf '%s\n' "$((10#$seconds * 1000000 + 10#$fraction))"
}

calyx_gate_grace_us() {
  local whole="${calyx_gate_signal_grace%%.*}" fraction=0
  [[ "$calyx_gate_signal_grace" != *.* ]] || fraction="${calyx_gate_signal_grace#*.}"
  fraction="${fraction}000000"
  fraction="${fraction:0:6}"
  printf '%s\n' "$((10#$whole * 1000000 + 10#$fraction))"
}

calyx_gate_cgroup_init() {
  [[ "$calyx_gate_cgroup_initialized" -eq 0 ]] || return 0
  local cgroup_type self_group owner uid token command_name
  local owner_starttime actual_starttime slice_control slice_invocation slice_identity active
  for command_name in systemd-run systemctl stat awk setsid; do
    command -v "$command_name" >/dev/null 2>&1 || {
      calyx_gate_cgroup_error "$command_name is required for stage containment"
      return 127
    }
  done
  cgroup_type="$(stat -fc %T /sys/fs/cgroup 2>/dev/null)" || return 125
  [[ "$cgroup_type" == cgroup2fs ]] || {
    calyx_gate_cgroup_error "unified cgroup v2 is required (actual=$cgroup_type)"
    return 125
  }
  self_group="$(awk -F: '$1=="0"{print $3}' /proc/self/cgroup 2>/dev/null)" || return 125
  [[ "$self_group" == /* ]] || {
    calyx_gate_cgroup_error "cannot read unified process cgroup"
    return 125
  }
  systemctl --user is-system-running >/dev/null 2>&1 || {
    calyx_gate_cgroup_error "the user systemd manager is not running"
    return 125
  }
  uid="$(id -u)"
  if [[ -n "${CALYX_GATE_RUN_SLICE:-}" ]]; then
    [[ "$CALYX_GATE_RUN_SLICE" =~ ^calyxgate-u[0-9]+-r[0-9]+\.slice$ \
      && "${CALYX_GATE_RUN_SLICE_OWNER_PID:-}" =~ ^[1-9][0-9]*$ \
      && "${CALYX_GATE_RUN_SLICE_OWNER_STARTTIME:-}" =~ ^[1-9][0-9]*$ ]] || {
      calyx_gate_cgroup_error "inherited gate run-slice marker is invalid"
      return 125
    }
    [[ "$CALYX_GATE_RUN_SLICE" == "calyxgate-u${uid}-"* ]] || {
      calyx_gate_cgroup_error "inherited gate run slice belongs to another uid"
      return 125
    }
    owner="$CALYX_GATE_RUN_SLICE_OWNER_PID"
    [[ "$owner" != "$BASHPID" ]] || {
      calyx_gate_cgroup_error "fresh runner cannot inherit its own run-slice marker"
      return 125
    }
    calyx_gate_pid_starttime actual_starttime "$owner" || {
      calyx_gate_cgroup_error "run-slice owner process is absent: pid=$owner"
      return 125
    }
    [[ "$actual_starttime" == "$CALYX_GATE_RUN_SLICE_OWNER_STARTTIME" ]] || {
      calyx_gate_cgroup_error "run-slice owner identity changed: pid=$owner"
      return 125
    }
    calyx_gate_read_unit_field slice_control "$CALYX_GATE_RUN_SLICE" ControlGroup || return 125
    calyx_gate_read_unit_field slice_invocation "$CALYX_GATE_RUN_SLICE" InvocationID || return 125
    calyx_gate_read_unit_field active "$CALYX_GATE_RUN_SLICE" ActiveState || return 125
    [[ "$slice_control" == /*/"$CALYX_GATE_RUN_SLICE" \
      && "$slice_invocation" =~ ^[0-9a-f]{32}$ && "$active" == active \
      && "$self_group" == "$slice_control"/* ]] || {
      calyx_gate_cgroup_error "inherited run-slice proof failed: control=$slice_control self=$self_group active=$active"
      return 125
    }
    slice_identity="$(stat -c '%d:%i' -- "/sys/fs/cgroup$slice_control")" || return 125
    calyx_gate_run_slice_control="$slice_control"
    calyx_gate_run_slice_identity="$slice_identity"
    calyx_gate_run_slice_invocation="$slice_invocation"
  else
    token="${BASHPID}${RANDOM}${RANDOM}${RANDOM}"
    CALYX_GATE_RUN_SLICE="calyxgate-u${uid}-r${token}.slice"
    CALYX_GATE_RUN_SLICE_OWNER_PID="$BASHPID"
    calyx_gate_pid_starttime owner_starttime "$BASHPID" || return 125
    CALYX_GATE_RUN_SLICE_OWNER_STARTTIME="$owner_starttime"
    calyx_gate_owns_run_slice=1
    export CALYX_GATE_RUN_SLICE CALYX_GATE_RUN_SLICE_OWNER_PID \
      CALYX_GATE_RUN_SLICE_OWNER_STARTTIME
  fi
  calyx_gate_cgroup_initialized=1
}

calyx_gate_prepare_scope() {
  calyx_gate_scope_seq=$((calyx_gate_scope_seq + 1))
  printf -v "$1" 'calyxgate-r%s-p%s-s%06d.scope' \
    "${CALYX_GATE_RUN_SLICE#*r}" "$BASHPID" "$calyx_gate_scope_seq"
}

calyx_gate_launch_scope() {
  local scope="$1" stage_dir="$2"
  shift 2
  exec setsid -- systemd-run --user --scope --quiet --collect \
    --slice="$CALYX_GATE_RUN_SLICE" --unit="$scope" \
    --property=KillMode=control-group --property=SendSIGKILL=yes \
    --property="TimeoutStopSec=${calyx_gate_signal_grace}s" \
    env --default-signal=HUP,INT,TERM bash -c '
      set -euo pipefail
      expected_scope="$1"
      stage_dir="$2"
      shift 2
      kill -STOP "$BASHPID"
      actual="$(awk -F: '\''$1=="0"{print $3}'\'' /proc/self/cgroup)"
      [[ "$actual" == */"$expected_scope" ]] || {
        echo "CALYX_GATE_CGROUP_REFUSED: pid=$BASHPID expected=$expected_scope actual=$actual" >&2
        exit 125
      }
      [[ -z "$stage_dir" ]] || cd -- "$stage_dir"
      exec "$@"
    ' _ "$scope" "$stage_dir" "$@"
}

calyx_gate_read_unit_field() {
  local output_var="$1" unit="$2" field="$3" value
  value="$(systemctl --user show "$unit" --property="$field" --value 2>/dev/null)" || {
    calyx_gate_cgroup_error "cannot read unit field: unit=$unit field=$field"
    return 125
  }
  printf -v "$output_var" '%s' "$value"
}

calyx_gate_read_process_cgroup() {
  local output_var="$1" pid="$2" value
  value="$(awk -F: '$1=="0"{print $3}' "/proc/$pid/cgroup" 2>/dev/null)" || {
    calyx_gate_cgroup_error "cannot read process cgroup: pid=$pid"
    return 125
  }
  [[ "$value" == /* ]] || {
    calyx_gate_cgroup_error "process cgroup is invalid: pid=$pid value=${value:-missing}"
    return 125
  }
  printf -v "$output_var" '%s' "$value"
}

calyx_gate_discover_owned_slice() {
  local required="${1:-1}" load id invocation control active identity
  [[ "$calyx_gate_owns_run_slice" -eq 1 ]] || return 0
  [[ -z "$calyx_gate_run_slice_control" ]] || return 0
  calyx_gate_read_unit_field load "$CALYX_GATE_RUN_SLICE" LoadState || return 125
  if [[ "$load" == not-found ]]; then
    [[ "$required" -eq 0 ]] && return 0
    calyx_gate_cgroup_error "owned run slice was not published: unit=$CALYX_GATE_RUN_SLICE"
    return 125
  fi
  calyx_gate_read_unit_field id "$CALYX_GATE_RUN_SLICE" Id || return 125
  calyx_gate_read_unit_field invocation "$CALYX_GATE_RUN_SLICE" InvocationID || return 125
  calyx_gate_read_unit_field control "$CALYX_GATE_RUN_SLICE" ControlGroup || return 125
  calyx_gate_read_unit_field active "$CALYX_GATE_RUN_SLICE" ActiveState || return 125
  [[ "$load" == loaded && "$id" == "$CALYX_GATE_RUN_SLICE" \
    && "$invocation" =~ ^[0-9a-f]{32}$ && "$control" == /*/"$CALYX_GATE_RUN_SLICE" \
    && "$active" == active ]] || {
    calyx_gate_cgroup_error "owned run slice publication mismatch: load=$load id=$id control=$control active=$active"
    return 125
  }
  identity="$(stat -c '%d:%i' -- "/sys/fs/cgroup$control")" || return 125
  calyx_gate_run_slice_control="$control"
  calyx_gate_run_slice_identity="$identity"
  calyx_gate_run_slice_invocation="$invocation"
}

# Scope publication is split into an early identity-bound capture and a full
# validation pass so every failure after systemd creates the unit can kill the
# exact cgroup, including inherited/nested runners that do not own the slice.
# shellcheck source=gate-cgroup-scope.sh
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)/gate-cgroup-scope.sh"

calyx_gate_control_population() {
  local control="$1" expected_identity="$2" path="/sys/fs/cgroup$1" actual value
  if [[ ! -e "$path" ]]; then
    calyx_gate_population=0
    return 0
  fi
  if [[ ! -d "$path" ]]; then
    [[ -e "$path" ]] || { calyx_gate_population=0; return 0; }
    calyx_gate_cgroup_error "cgroup control is not a directory: control=$control"
    return 125
  fi
  if ! actual="$(stat -c '%d:%i' -- "$path" 2>/dev/null)"; then
    [[ ! -e "$path" ]] || {
      calyx_gate_cgroup_error "cannot read cgroup identity: control=$control"
      return 125
    }
    calyx_gate_population=0
    return 0
  fi
  [[ "$actual" == "$expected_identity" ]] || {
    calyx_gate_cgroup_error "cgroup identity changed: control=$control expected=$expected_identity actual=$actual"
    return 125
  }
  if ! value="$(awk '$1=="populated"{print $2}' "$path/cgroup.events" 2>/dev/null)"; then
    [[ ! -e "$path" ]] || {
      calyx_gate_cgroup_error "cannot read cgroup.events: control=$control"
      return 125
    }
    calyx_gate_population=0
    return 0
  fi
  [[ "$value" == 0 || "$value" == 1 ]] || {
    calyx_gate_cgroup_error "invalid cgroup population: control=$control value=${value:-missing}"
    return 125
  }
  calyx_gate_population="$value"
}

calyx_gate_verify_active_unit() {
  local id invocation control
  calyx_gate_read_unit_field id "$calyx_gate_active_scope" Id || return 125
  calyx_gate_read_unit_field invocation "$calyx_gate_active_scope" InvocationID || return 125
  calyx_gate_read_unit_field control "$calyx_gate_active_scope" ControlGroup || return 125
  [[ "$id" == "$calyx_gate_active_scope" \
    && "$invocation" == "$calyx_gate_active_invocation" \
    && "$control" == "$calyx_gate_active_control" ]] || {
    calyx_gate_cgroup_error "active scope identity changed: scope=$calyx_gate_active_scope"
    return 125
  }
}

calyx_gate_verify_owned_slice_unit() {
  local id invocation control active
  calyx_gate_read_unit_field id "$CALYX_GATE_RUN_SLICE" Id || return 125
  calyx_gate_read_unit_field invocation "$CALYX_GATE_RUN_SLICE" InvocationID || return 125
  calyx_gate_read_unit_field control "$CALYX_GATE_RUN_SLICE" ControlGroup || return 125
  calyx_gate_read_unit_field active "$CALYX_GATE_RUN_SLICE" ActiveState || return 125
  [[ "$id" == "$CALYX_GATE_RUN_SLICE" \
    && "$invocation" == "$calyx_gate_run_slice_invocation" \
    && "$control" == "$calyx_gate_run_slice_control" \
    && "$active" == active ]] || {
    calyx_gate_cgroup_error "owned run slice identity changed: id=$id control=$control active=$active"
    return 125
  }
}

calyx_gate_wait_control_empty() {
  local control="$1" identity="$2" deadline now
  deadline=$(( $(calyx_gate_now_us) + $(calyx_gate_grace_us) ))
  while :; do
    calyx_gate_control_population "$control" "$identity" || return $?
    [[ "$calyx_gate_population" -eq 0 ]] && return 0
    now="$(calyx_gate_now_us)"
    (( now < deadline )) || return 1
    sleep 0.02
  done
}

calyx_gate_kill_control() {
  local control="$1" identity="$2" path="/sys/fs/cgroup$1" actual
  [[ -e "$path" ]] || return 0
  if ! actual="$(stat -c '%d:%i' -- "$path" 2>/dev/null)"; then
    calyx_gate_control_population "$control" "$identity" || return $?
    [[ "$calyx_gate_population" -eq 0 ]] && return 0
    return 125
  fi
  [[ "$actual" == "$identity" ]] || return 125
  printf '1\n' >"$path/cgroup.kill" 2>/dev/null || {
    calyx_gate_control_population "$control" "$identity" || return $?
    [[ "$calyx_gate_population" -eq 0 ]] && return 0
    return 125
  }
}

calyx_gate_wait_control_gone() {
  local control="$1" identity="$2"
  for _ in {1..200}; do
    calyx_gate_control_population "$control" "$identity" || return $?
    [[ "$calyx_gate_population" -eq 0 ]] && return 0
    sleep 0.01
  done
  calyx_gate_cgroup_error "cgroup remains populated after KILL: control=$control"
  return 125
}

calyx_gate_drain_active_scope() {
  local signal_name="$1" label="$2" verify_status
  calyx_gate_control_population "$calyx_gate_active_control" \
    "$calyx_gate_active_control_identity" || return $?
  [[ "$calyx_gate_population" -eq 1 ]] || return 0
  if calyx_gate_verify_active_unit; then
    :
  else
    verify_status=$?
    calyx_gate_cgroup_error "active scope metadata verification failed: scope=$calyx_gate_active_scope status=$verify_status; using identity-bound cgroup.kill"
    calyx_gate_kill_control "$calyx_gate_active_control" \
      "$calyx_gate_active_control_identity" || return $?
    calyx_gate_wait_control_gone "$calyx_gate_active_control" \
      "$calyx_gate_active_control_identity" || return $?
    return 125
  fi
  echo "${CALYX_GATE_STAGE_OWNER:-gate}: forwarding $signal_name to scope=$calyx_gate_active_scope control=$calyx_gate_active_control" >&2
  systemctl --user kill --kill-whom=all --signal="$signal_name" \
    "$calyx_gate_active_scope" >/dev/null 2>&1 || {
    calyx_gate_control_population "$calyx_gate_active_control" \
      "$calyx_gate_active_control_identity" || return $?
    [[ "$calyx_gate_population" -eq 0 ]] || return 125
  }
  if ! calyx_gate_wait_control_empty "$calyx_gate_active_control" \
    "$calyx_gate_active_control_identity"; then
    echo "${CALYX_GATE_STAGE_OWNER:-gate}: $label exceeded signal grace; using cgroup.kill" >&2
    calyx_gate_kill_control "$calyx_gate_active_control" \
      "$calyx_gate_active_control_identity" || return $?
  fi
  calyx_gate_wait_control_gone "$calyx_gate_active_control" \
    "$calyx_gate_active_control_identity"
}

calyx_gate_drain_owned_slice() {
  local signal_name="$1" label="$2" verify_status
  [[ "$calyx_gate_owns_run_slice" -eq 1 && -n "$calyx_gate_run_slice_control" ]] || return 125
  calyx_gate_control_population "$calyx_gate_run_slice_control" \
    "$calyx_gate_run_slice_identity" || return $?
  [[ "$calyx_gate_population" -eq 1 ]] || return 0
  if calyx_gate_verify_owned_slice_unit; then
    :
  else
    verify_status=$?
    calyx_gate_cgroup_error "owned run slice metadata verification failed: slice=$CALYX_GATE_RUN_SLICE status=$verify_status; using identity-bound cgroup.kill"
    calyx_gate_kill_control "$calyx_gate_run_slice_control" \
      "$calyx_gate_run_slice_identity" || return $?
    calyx_gate_wait_control_gone "$calyx_gate_run_slice_control" \
      "$calyx_gate_run_slice_identity" || return $?
    return 125
  fi
  echo "${CALYX_GATE_STAGE_OWNER:-gate}: forwarding $signal_name to run_slice=$CALYX_GATE_RUN_SLICE control=$calyx_gate_run_slice_control" >&2
  systemctl --user kill --kill-whom=cgroup --signal="$signal_name" \
    "$CALYX_GATE_RUN_SLICE" >/dev/null 2>&1 || {
    calyx_gate_control_population "$calyx_gate_run_slice_control" \
      "$calyx_gate_run_slice_identity" || return $?
    [[ "$calyx_gate_population" -eq 0 ]] || return 125
  }
  if ! calyx_gate_wait_control_empty "$calyx_gate_run_slice_control" \
    "$calyx_gate_run_slice_identity"; then
    echo "${CALYX_GATE_STAGE_OWNER:-gate}: $label exceeded signal grace; using cgroup.kill" >&2
    calyx_gate_kill_control "$calyx_gate_run_slice_control" \
      "$calyx_gate_run_slice_identity" || return $?
  fi
  calyx_gate_wait_control_gone "$calyx_gate_run_slice_control" \
    "$calyx_gate_run_slice_identity"
}

calyx_gate_clear_active_scope() {
  calyx_gate_active_pid=""
  calyx_gate_active_pgid=""
  calyx_gate_active_scope=""
  calyx_gate_active_invocation=""
  calyx_gate_active_control=""
  calyx_gate_active_control_identity=""
}

calyx_gate_finish_active_scope() {
  local status="$1" leaked=0
  calyx_gate_control_population "$calyx_gate_active_control" \
    "$calyx_gate_active_control_identity" || {
    calyx_gate_finish_status=125
    return 125
  }
  if [[ "$calyx_gate_population" -eq 1 ]]; then
    leaked=1
    calyx_gate_cgroup_error "stage left processes in scope=$calyx_gate_active_scope; draining"
    calyx_gate_drain_active_scope TERM "stage scope" || {
      calyx_gate_finish_status=125
      return 125
    }
  fi
  calyx_gate_clear_active_scope
  [[ "$leaked" -eq 0 || "$status" -ne 0 ]] || status=125
  calyx_gate_finish_status="$status"
}

calyx_gate_finish_owned_slice() {
  local status="$1" leaked=0
  if [[ "$calyx_gate_owns_run_slice" -eq 0 || -z "$calyx_gate_run_slice_control" ]]; then
    calyx_gate_finish_status="$status"
    return 0
  fi
  calyx_gate_control_population "$calyx_gate_run_slice_control" \
    "$calyx_gate_run_slice_identity" || {
    calyx_gate_finish_status=125
    return 125
  }
  if [[ "$calyx_gate_population" -eq 1 ]]; then
    leaked=1
    calyx_gate_cgroup_error "stage left processes in run slice=$CALYX_GATE_RUN_SLICE; draining"
    calyx_gate_drain_owned_slice TERM "run slice" || {
      calyx_gate_finish_status=125
      return 125
    }
  fi
  [[ "$leaked" -eq 0 || "$status" -ne 0 ]] || status=125
  calyx_gate_finish_status="$status"
}

calyx_gate_assert_owned_slice_empty() {
  [[ "$calyx_gate_owns_run_slice" -eq 1 && -n "$calyx_gate_run_slice_control" ]] || return 0
  calyx_gate_control_population "$calyx_gate_run_slice_control" \
    "$calyx_gate_run_slice_identity" || return $?
  [[ "$calyx_gate_population" -eq 0 ]] || {
    calyx_gate_cgroup_error "run slice remains populated: $calyx_gate_run_slice_control"
    return 125
  }
}

calyx_gate_release_owned_slice() {
  local path stop_status=0
  [[ "$calyx_gate_owns_run_slice" -eq 1 ]] || return 0
  calyx_gate_discover_owned_slice 0 || return $?
  [[ -n "$calyx_gate_run_slice_control" ]] || return 0
  path="/sys/fs/cgroup$calyx_gate_run_slice_control"
  if [[ ! -e "$path" ]]; then
    calyx_gate_run_slice_control=""
    calyx_gate_run_slice_identity=""
    calyx_gate_run_slice_invocation=""
    return 0
  fi
  calyx_gate_assert_owned_slice_empty || return $?
  calyx_gate_verify_owned_slice_unit || return $?
  systemctl --user stop "$CALYX_GATE_RUN_SLICE" >/dev/null 2>&1 || stop_status=$?
  for _ in {1..200}; do
    [[ -e "$path" ]] || break
    calyx_gate_control_population "$calyx_gate_run_slice_control" \
      "$calyx_gate_run_slice_identity" || return $?
    [[ "$calyx_gate_population" -eq 0 ]] || {
      calyx_gate_cgroup_error "run slice repopulated during release: $calyx_gate_run_slice_control"
      return 125
    }
    sleep 0.01
  done
  [[ ! -e "$path" ]] || {
    calyx_gate_cgroup_error "run slice cgroup remains after stop: control=$calyx_gate_run_slice_control status=$stop_status"
    return 125
  }
  echo "${CALYX_GATE_STAGE_OWNER:-gate}: run_slice_released unit=$CALYX_GATE_RUN_SLICE" >&2
  calyx_gate_run_slice_control=""
  calyx_gate_run_slice_identity=""
  calyx_gate_run_slice_invocation=""
}
