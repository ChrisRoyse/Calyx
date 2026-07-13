#!/usr/bin/env bash
# Source-only path, mount, identity, and persistent-lock contracts for gates.

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  echo "gate-runtime-state: ERROR this file must be sourced" >&2
  exit 64
fi

calyx_gate_assert_path_text() {
  local label="$1" path="$2"
  [[ "$path" == /* ]] || {
    echo "${CALYX_GATE_STAGE_OWNER:-gate}: ERROR $label must be absolute: $path" >&2
    return 125
  }
  [[ "$path" != *$'\n'* && "$path" != *$'\r'* ]] || {
    echo "${CALYX_GATE_STAGE_OWNER:-gate}: ERROR $label must be single-line" >&2
    return 125
  }
}

calyx_gate_lexical_path() {
  local output_var="$1" label="$2" raw="$3" normalized
  calyx_gate_assert_path_text "$label" "$raw" || return $?
  command -v realpath >/dev/null 2>&1 || {
    echo "${CALYX_GATE_STAGE_OWNER:-gate}: ERROR realpath is required" >&2
    return 127
  }
  normalized="$(realpath -ms -- "$raw")" || return 125
  calyx_gate_assert_path_text "$label" "$normalized" || return $?
  printf -v "$output_var" '%s' "$normalized"
}

calyx_gate_assert_stable_owned_dir() {
  local label="$1" path="$2" owner_uid mode mode_value physical
  [[ -d "$path" && ! -L "$path" ]] || {
    echo "${CALYX_GATE_STAGE_OWNER:-gate}: ERROR $label is not a stable directory: $path" >&2
    return 125
  }
  owner_uid="$(stat -c '%u' -- "$path")" || return 125
  mode="$(stat -c '%a' -- "$path")" || return 125
  mode_value=$((8#$mode))
  [[ "$owner_uid" == "$(id -u)" && $((mode_value & 0022)) -eq 0 ]] || {
    echo "${CALYX_GATE_STAGE_OWNER:-gate}: ERROR $label ownership/mode is unsafe: path=$path uid=$owner_uid mode=$mode" >&2
    return 125
  }
  physical="$(cd -- "$path" && pwd -P)" || return 125
  [[ "$physical" == "$path" ]] || {
    echo "${CALYX_GATE_STAGE_OWNER:-gate}: ERROR $label contains a symlink component: path=$path physical=$physical" >&2
    return 125
  }
}

calyx_gate_resolve_home() {
  local output_var="$1" home_raw="${CALYX_HOME:-}" home
  [[ -n "$home_raw" ]] || {
    echo "${CALYX_GATE_STAGE_OWNER:-gate}: ERROR CALYX_HOME is required for global gate ownership" >&2
    return 125
  }
  calyx_gate_lexical_path home "CALYX_HOME" "$home_raw" || return $?
  calyx_gate_assert_stable_owned_dir "CALYX_HOME" "$home" || return $?
  export CALYX_HOME="$home"
  printf -v "$output_var" '%s' "$home"
}

calyx_gate_resolve_target_root() {
  local output_var="$1" home="$2" raw
  raw="${CALYX_TARGET_SCRATCH_ROOT:-$home/target-scratch}"
  local root expected="$home/target-scratch"
  calyx_gate_lexical_path root "Cargo target scratch root" "$raw" || return $?
  [[ "$root" == "$expected" ]] || {
    echo "${CALYX_GATE_STAGE_OWNER:-gate}: ERROR Cargo target scratch root must be canonical: expected=$expected actual=$root" >&2
    return 125
  }
  if [[ -e "$root" || -L "$root" ]]; then
    calyx_gate_assert_stable_owned_dir "Cargo target scratch root" "$root" || return $?
  fi
  export CALYX_TARGET_SCRATCH_ROOT="$root"
  printf -v "$output_var" '%s' "$root"
}

calyx_gate_mount_snapshot() {
  local output_var="$1" mount_output
  command -v findmnt >/dev/null 2>&1 || {
    echo "${CALYX_GATE_STAGE_OWNER:-gate}: ERROR findmnt is required for mount-safe cleanup" >&2
    return 127
  }
  if ! mount_output="$(findmnt -rn -o TARGET)"; then
    echo "${CALYX_GATE_STAGE_OWNER:-gate}: ERROR findmnt failed while proving cleanup boundaries" >&2
    return 125
  fi
  printf -v "$output_var" '%s' "$mount_output"
}

calyx_gate_assert_no_mounts_at_or_below() {
  local path="$1" label="${2:-path}" snapshot mount_target
  calyx_gate_mount_snapshot snapshot || return $?
  while IFS= read -r mount_target; do
    [[ -n "$mount_target" ]] || continue
    case "$mount_target" in
      "$path"|"$path"/*)
        echo "${CALYX_GATE_STAGE_OWNER:-gate}: ERROR mountpoint at or below $label: path=$path mount=$mount_target" >&2
        return 125
        ;;
    esac
  done <<<"$snapshot"
}

calyx_gate_plan_owned_dir() {
  local path_var="$1" root="$2" prefix="$3"
  local candidate="$root/$prefix.${BASHPID}.${RANDOM}${RANDOM}${RANDOM}"
  printf -v "$path_var" '%s' "$candidate"
}

calyx_gate_read_path_identity() {
  local output_var="$1" path="$2" identity
  identity="$(stat -c '%d:%i' -- "$path" 2>/dev/null)" || {
    echo "${CALYX_GATE_STAGE_OWNER:-gate}: ERROR cannot read path identity: $path" >&2
    return 125
  }
  [[ "$identity" =~ ^[0-9]+:[0-9]+$ ]] || return 125
  printf -v "$output_var" '%s' "$identity"
}

calyx_gate_close_numeric_fd() {
  local fd="$1"
  [[ "$fd" =~ ^[0-9]+$ ]] || return 125
  eval "exec ${fd}>&-"
}

calyx_gate_discard_identity_capture() {
  local read_fd="$1" write_fd="$2" pid="$3"
  [[ -z "$write_fd" ]] || calyx_gate_close_numeric_fd "$write_fd" || true
  [[ -z "$read_fd" ]] || calyx_gate_close_numeric_fd "$read_fd" || true
  [[ ! "$pid" =~ ^[1-9][0-9]*$ ]] || kill -KILL "$pid" 2>/dev/null || true
  [[ ! "$pid" =~ ^[1-9][0-9]*$ ]] || wait "$pid" 2>/dev/null || true
}

calyx_gate_open_identity_capture() {
  local read_var="$1" write_var="$2" parent="$3"
  local capture_read_fd capture_write_fd capture_pid read_identity write_identity
  local read_uid write_uid read_mode write_mode read_type write_type read_meta write_meta owner_pid
  calyx_gate_assert_stable_owned_dir "identity capture parent" "$parent" || return $?
  [[ -x /usr/bin/cat ]] || return 127
  coproc CALYX_GATE_ID_CAPTURE { exec /usr/bin/cat; }
  capture_pid="$CALYX_GATE_ID_CAPTURE_PID"
  exec {capture_read_fd}<&"${CALYX_GATE_ID_CAPTURE[0]}" || {
    calyx_gate_close_numeric_fd "${CALYX_GATE_ID_CAPTURE[0]}" || true
    calyx_gate_close_numeric_fd "${CALYX_GATE_ID_CAPTURE[1]}" || true
    kill -KILL "$capture_pid" 2>/dev/null || true
    wait "$capture_pid" 2>/dev/null || true
    return 125
  }
  exec {capture_write_fd}>&"${CALYX_GATE_ID_CAPTURE[1]}" || {
    calyx_gate_close_numeric_fd "$capture_read_fd" || true
    calyx_gate_close_numeric_fd "${CALYX_GATE_ID_CAPTURE[0]}" || true
    calyx_gate_close_numeric_fd "${CALYX_GATE_ID_CAPTURE[1]}" || true
    kill -KILL "$capture_pid" 2>/dev/null || true
    wait "$capture_pid" 2>/dev/null || true
    return 125
  }
  calyx_gate_close_numeric_fd "${CALYX_GATE_ID_CAPTURE[0]}" || {
    calyx_gate_discard_identity_capture "$capture_read_fd" "$capture_write_fd" "$capture_pid"
    return 125
  }
  calyx_gate_close_numeric_fd "${CALYX_GATE_ID_CAPTURE[1]}" || {
    calyx_gate_discard_identity_capture "$capture_read_fd" "$capture_write_fd" "$capture_pid"
    return 125
  }
  unset CALYX_GATE_ID_CAPTURE CALYX_GATE_ID_CAPTURE_PID
  owner_pid="$BASHPID"
  read_meta="$(stat -Lc '%d:%i|%u|%a|%F' -- "/proc/$owner_pid/fd/$capture_read_fd")" || {
    calyx_gate_discard_identity_capture "$capture_read_fd" "$capture_write_fd" "$capture_pid"
    return 125
  }
  write_meta="$(stat -Lc '%d:%i|%u|%a|%F' -- "/proc/$owner_pid/fd/$capture_write_fd")" || {
    calyx_gate_discard_identity_capture "$capture_read_fd" "$capture_write_fd" "$capture_pid"
    return 125
  }
  IFS='|' read -r read_identity read_uid read_mode read_type <<<"$read_meta"
  IFS='|' read -r write_identity write_uid write_mode write_type <<<"$write_meta"
  if [[ ! "$read_identity" =~ ^[0-9]+:[0-9]+$ || ! "$write_identity" =~ ^[0-9]+:[0-9]+$ \
    || "$read_uid" != "$(id -u)" || "$write_uid" != "$read_uid" \
    || $((8#$read_mode & 0077)) -ne 0 \
    || "$write_mode" != "$read_mode" || "$read_type" != fifo || "$write_type" != fifo ]]; then
    calyx_gate_discard_identity_capture "$capture_read_fd" "$capture_write_fd" "$capture_pid"
    return 125
  fi
  printf -v "$read_var" '%s' "$capture_read_fd"
  printf -v "$write_var" '%s' "$capture_write_fd"
  calyx_gate_identity_capture_pid="$capture_pid"
}

calyx_gate_read_create_identity() {
  local read_fd="$1" root_var="$2" entry_var="$3" record extra
  IFS= read -r record <&"$read_fd" || {
    echo "${CALYX_GATE_STAGE_OWNER:-gate}: ERROR creator did not publish a complete identity record" >&2
    return 125
  }
  if IFS= read -r extra <&"$read_fd"; then
    echo "${CALYX_GATE_STAGE_OWNER:-gate}: ERROR creator published more than one identity record" >&2
    return 125
  fi
  if [[ ! "$record" =~ ^CALYX_CREATE_ID_V1\ root=([0-9]+:[0-9]+)\ entry=([0-9]+:[0-9]+)$ ]]; then
    echo "${CALYX_GATE_STAGE_OWNER:-gate}: ERROR creator identity record is malformed" >&2
    return 125
  fi
  printf -v "$root_var" '%s' "${BASH_REMATCH[1]}"
  printf -v "$entry_var" '%s' "${BASH_REMATCH[2]}"
}

calyx_gate_create_owned_dir() {
  local ready_var="$1" root_var="$2" entry_var="$3" capture_parent="$4"
  shift 4
  local read_fd="" write_fd="" capture_pid="" create_status=0 record_status=0 capture_status=0
  local captured_root="" captured_entry=""
  printf -v "$ready_var" '%s' 0
  printf -v "$root_var" '%s' ""
  printf -v "$entry_var" '%s' ""
  calyx_gate_identity_capture_pid=""
  calyx_gate_open_identity_capture read_fd write_fd "$capture_parent" || return $?
  capture_pid="$calyx_gate_identity_capture_pid"
  if calyx_gate_run env CALYX_CREATE_ID_FD=1 "$@" >&"$write_fd"; then
    create_status=0
  else
    create_status=$?
  fi
  calyx_gate_close_numeric_fd "$write_fd" || return 125
  if calyx_gate_read_create_identity "$read_fd" captured_root captured_entry; then
    record_status=0
  else
    record_status=$?
  fi
  calyx_gate_close_numeric_fd "$read_fd" || return 125
  if wait "$capture_pid"; then capture_status=0; else capture_status=$?; fi
  [[ "$capture_status" -eq 0 ]] || return 125
  if [[ "$record_status" -eq 0 ]]; then
    printf -v "$root_var" '%s' "$captured_root"
    printf -v "$entry_var" '%s' "$captured_entry"
    printf -v "$ready_var" '%s' 1
  fi
  [[ "$create_status" -eq 0 ]] || return "$create_status"
  [[ "$record_status" -eq 0 ]] || return "$record_status"
}

calyx_gate_safe_remove_entry() {
  local root="$1" root_id="$2" relative="$3" entry_id="$4" kind="$5" tag="$6"
  local entry_uid root_uid entry_mask root_mask cargo_policy
  entry_uid="${7:-$(id -u)}"
  root_uid="${8:-$entry_uid}"
  entry_mask="${9-}"
  if [[ $# -ge 10 ]]; then root_mask="${10}"; else root_mask="$entry_mask"; fi
  cargo_policy="${11:-0}"
  local runtime_dir helper helper_fd="" helper_uid helper_mode helper_identity path_identity
  local status lock_identity=""
  runtime_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)" || return 125
  helper="$runtime_dir/gate_safe_rmtree.py"
  [[ -f "$helper" && ! -L "$helper" ]] || {
    echo "${CALYX_GATE_STAGE_OWNER:-gate}: ERROR safe removal helper is unavailable: $helper" >&2
    return 125
  }
  exec {helper_fd}<"$helper" || return 125
  helper_uid="$(stat -Lc '%u' -- "/proc/$BASHPID/fd/$helper_fd")" || {
    eval "exec ${helper_fd}>&-"; return 125;
  }
  helper_mode="$(stat -Lc '%a' -- "/proc/$BASHPID/fd/$helper_fd")" || {
    eval "exec ${helper_fd}>&-"; return 125;
  }
  helper_identity="$(stat -Lc '%d:%i' -- "/proc/$BASHPID/fd/$helper_fd")" || {
    eval "exec ${helper_fd}>&-"; return 125;
  }
  path_identity="$(stat -c '%d:%i' -- "$helper")" || {
    eval "exec ${helper_fd}>&-"; return 125;
  }
  [[ "$helper_uid" == "$(id -u)" && $((8#$helper_mode & 0022)) -eq 0 \
    && "$helper_identity" == "$path_identity" ]] || {
    eval "exec ${helper_fd}>&-"; return 125;
  }
  command -v python3 >/dev/null 2>&1 || {
    eval "exec ${helper_fd}>&-"; return 127;
  }
  local -a args=(--root "$root" --root-id "$root_id" --relative "$relative"
    --entry-id "$entry_id" --kind "$kind" --uid "$entry_uid" --root-uid "$root_uid" --tag "$tag")
  [[ -z "$entry_mask" ]] || args+=(--forbid-mode-mask "$entry_mask")
  [[ -z "$root_mask" ]] || args+=(--root-forbid-mode-mask "$root_mask")
  [[ "$cargo_policy" -eq 0 ]] || args+=(--require-cargo-marker-or-empty)
  if [[ -n "${CALYX_GATE_LOCK_PATH:-}" || -n "${CALYX_GATE_LOCK_FD:-}" ]]; then
    [[ -n "${CALYX_GATE_LOCK_PATH:-}" && -n "${CALYX_GATE_LOCK_FD:-}" ]] || {
      eval "exec ${helper_fd}>&-"; return 125;
    }
    calyx_gate_validate_lock "$CALYX_GATE_LOCK_PATH" "$CALYX_GATE_LOCK_FD" || {
      status=$?; eval "exec ${helper_fd}>&-"; return "$status";
    }
    lock_identity="$(stat -Lc '%d:%i' -- "/proc/$BASHPID/fd/$CALYX_GATE_LOCK_FD")" || {
      eval "exec ${helper_fd}>&-"; return 125;
    }
    args+=(--gate-lock-fd "$CALYX_GATE_LOCK_FD" --gate-lock-id "$lock_identity")
  fi
  if python3 "/proc/self/fd/$helper_fd" "${args[@]}"; then status=0; else status=$?; fi
  eval "exec ${helper_fd}>&-"
  return "$status"
}

calyx_gate_validate_lock_path() {
  local lock_path="$1" parent owner_uid mode mode_value
  calyx_gate_assert_path_text "gate lock path" "$lock_path" || return $?
  parent="$(dirname -- "$lock_path")"
  calyx_gate_assert_stable_owned_dir "gate lock parent" "$parent" || return $?
  if [[ -e "$lock_path" || -L "$lock_path" ]]; then
    [[ -f "$lock_path" && ! -L "$lock_path" ]] || {
      echo "${CALYX_GATE_STAGE_OWNER:-gate}: ERROR gate lock is not a regular file: $lock_path" >&2
      return 125
    }
    owner_uid="$(stat -c '%u' -- "$lock_path")" || return 125
    mode="$(stat -c '%a' -- "$lock_path")" || return 125
    mode_value=$((8#$mode))
    [[ "$owner_uid" == "$(id -u)" && $((mode_value & 0022)) -eq 0 ]] || {
      echo "${CALYX_GATE_STAGE_OWNER:-gate}: ERROR gate lock ownership/mode is unsafe: path=$lock_path uid=$owner_uid mode=$mode" >&2
      return 125
    }
  fi
}

calyx_gate_validate_open_lock() {
  local lock_path="$1" lock_fd="$2" fd_path lock_identity fd_identity
  [[ "$lock_fd" =~ ^[0-9]+$ && -e "/proc/$BASHPID/fd/$lock_fd" ]] || return 125
  fd_path="$(readlink -f -- "/proc/$BASHPID/fd/$lock_fd")" || return 125
  [[ "$fd_path" == "$lock_path" ]] || {
    echo "${CALYX_GATE_STAGE_OWNER:-gate}: ERROR inherited Cargo gate lock path mismatch: expected=$lock_path actual=$fd_path" >&2
    return 125
  }
  lock_identity="$(stat -c '%d:%i' -- "$lock_path")" || return 125
  fd_identity="$(stat -Lc '%d:%i' -- "/proc/$BASHPID/fd/$lock_fd")" || return 125
  [[ "$lock_identity" == "$fd_identity" ]] || return 125
}

calyx_gate_require_ofd_flock() {
  command -v flock >/dev/null 2>&1 || {
    echo "${CALYX_GATE_STAGE_OWNER:-gate}: ERROR flock is required for Cargo gate serialization" >&2
    return 127
  }
  flock --help 2>&1 | grep -q -- '--fcntl' || {
    echo "${CALYX_GATE_STAGE_OWNER:-gate}: ERROR flock must support Linux open-file-description locks" >&2
    return 127
  }
}

calyx_gate_query_ofd_lock() {
  local output_var="$1" lock_fd="$2" runtime_dir helper helper_fd="" result status
  runtime_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)" || return 125
  helper="$runtime_dir/gate_ofd_lock_query.py"
  [[ -f "$helper" && ! -L "$helper" ]] || return 125
  exec {helper_fd}<"$helper" || return 125
  result="$(python3 "/proc/self/fd/$helper_fd" "$lock_fd")" || status=$?
  eval "exec ${helper_fd}>&-"
  [[ "${status:-0}" -eq 0 && ( "$result" == unlocked || "$result" == conflict ) ]] || return 125
  printf -v "$output_var" '%s' "$result"
}

calyx_gate_prove_lock_held() {
  local lock_path="$1" lock_fd="$2" probe_fd="" probe_state candidate_state stable_state
  exec {probe_fd}>>"$lock_path" || return 125
  calyx_gate_query_ofd_lock probe_state "$probe_fd" || {
    eval "exec ${probe_fd}>&-"; return 125;
  }
  calyx_gate_query_ofd_lock candidate_state "$lock_fd" || {
    eval "exec ${probe_fd}>&-"; return 125;
  }
  calyx_gate_query_ofd_lock stable_state "$probe_fd" || {
    eval "exec ${probe_fd}>&-"; return 125;
  }
  eval "exec ${probe_fd}>&-"
  if [[ "$probe_state" == unlocked && "$candidate_state" == unlocked ]]; then
    echo "${CALYX_GATE_STAGE_OWNER:-gate}: ERROR inherited Cargo gate lock is not held: $lock_path" >&2
    return 125
  fi
  [[ "$probe_state" == conflict && "$stable_state" == conflict \
    && "$candidate_state" == unlocked ]] || {
    echo "${CALYX_GATE_STAGE_OWNER:-gate}: ERROR inherited Cargo gate fd is not the locked open description: $lock_path" >&2
    return 125
  }
}

calyx_gate_acquire_lock() {
  local output_var="$1" lock_path="$2" lock_fd old_umask status
  calyx_gate_require_ofd_flock || return $?
  calyx_gate_validate_lock_path "$lock_path" || return $?
  old_umask="$(umask)"
  umask 077
  exec {lock_fd}>>"$lock_path" || {
    umask "$old_umask"
    return 125
  }
  umask "$old_umask"
  calyx_gate_validate_lock_path "$lock_path" || {
    eval "exec ${lock_fd}>&-"
    return 125
  }
  calyx_gate_validate_open_lock "$lock_path" "$lock_fd" || {
    eval "exec ${lock_fd}>&-"
    return 125
  }
  if calyx_gate_run flock --fcntl --exclusive "$lock_fd"; then status=0; else status=$?; fi
  if [[ "$status" -ne 0 ]]; then
    eval "exec ${lock_fd}>&-"
    echo "${CALYX_GATE_STAGE_OWNER:-gate}: ERROR cannot acquire Cargo gate lock: path=$lock_path status=$status" >&2
    return "$status"
  fi
  printf -v "$output_var" '%s' "$lock_fd"
  echo "${CALYX_GATE_STAGE_OWNER:-gate}: cargo_gate_lock_acquired path=$lock_path fd=$lock_fd" >&2
}

calyx_gate_validate_lock() {
  local lock_path="$1" lock_fd="$2"
  [[ "$lock_fd" =~ ^[0-9]+$ && -e "/proc/$BASHPID/fd/$lock_fd" ]] || {
    echo "${CALYX_GATE_STAGE_OWNER:-gate}: ERROR inherited Cargo gate lock fd is unavailable: $lock_fd" >&2
    return 125
  }
  calyx_gate_require_ofd_flock || return $?
  calyx_gate_validate_lock_path "$lock_path" || return $?
  calyx_gate_validate_open_lock "$lock_path" "$lock_fd" || return $?
  calyx_gate_prove_lock_held "$lock_path" "$lock_fd"
}

calyx_gate_close_lock() {
  local fd_var="$1" lock_fd=""
  lock_fd="${!fd_var:-}"
  [[ -n "$lock_fd" ]] || return 0
  [[ "$lock_fd" =~ ^[0-9]+$ ]] || return 125
  # Never issue LOCK_UN: a live inherited descriptor must retain ownership.
  eval "exec ${lock_fd}>&-"
  printf -v "$fd_var" '%s' ""
}
