#!/usr/bin/env bash
# Source-only creator ownership transport and recovery for Calyx gates.

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  echo "gate-identity-capture: ERROR this file must be sourced" >&2
  exit 64
fi

calyx_gate_prepared_root_identity=""
calyx_gate_prepared_entry_identity=""
calyx_gate_prepared_staging_name=""
calyx_gate_record_timeout_seconds=5
calyx_gate_identity_max_bytes=1024

calyx_gate_close_numeric_fd() {
  local fd="$1"
  [[ "$fd" =~ ^[0-9]+$ ]] || return 125
  eval "exec ${fd}>&-"
}

calyx_gate_discard_fd_channel() {
  local status=0 fd
  for fd in "$@"; do
    [[ -z "$fd" ]] || calyx_gate_close_numeric_fd "$fd" || status=125
  done
  return "$status"
}

calyx_gate_open_identity_helper_fd() {
  local output_var="$1" helper_path="" before="" after="" opened_fd=""
  local before_identity before_uid before_mode before_type
  local after_identity after_uid after_mode after_type
  helper_path="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)/gate_identity_record_exec.py"
  [[ -f "$helper_path" && ! -L "$helper_path" ]] || return 125
  before="$(stat -c '%d:%i|%u|%a|%F' -- "$helper_path")" || return 125
  IFS='|' read -r before_identity before_uid before_mode before_type <<<"$before"
  [[ "$before_identity" =~ ^[0-9]+:[0-9]+$ && "$before_uid" == "$(id -u)" \
    && $((8#$before_mode & 0022)) -eq 0 && "$before_type" == "regular file" ]] || return 125
  exec {opened_fd}<"$helper_path" || return 125
  after="$(stat -Lc '%d:%i|%u|%a|%F' -- "/proc/$BASHPID/fd/$opened_fd")" || {
    calyx_gate_close_numeric_fd "$opened_fd" || true
    return 125
  }
  IFS='|' read -r after_identity after_uid after_mode after_type <<<"$after"
  [[ "$after_identity" == "$before_identity" && "$after_uid" == "$before_uid" \
    && "$after_mode" == "$before_mode" && "$after_type" == "$before_type" ]] || {
    calyx_gate_close_numeric_fd "$opened_fd" || true
    return 125
  }
  printf -v "$output_var" '%s' "$opened_fd"
}

calyx_gate_open_fd_channel() {
  local read_var="$1" write_var="$2" parent="$3" label="${4:-gate channel}"
  local channel_dir="" channel_path="" anchor_fd="" channel_read_fd="" channel_write_fd=""
  local parent_before="" parent_after="" read_meta="" write_meta=""
  local read_identity read_uid read_mode read_type write_identity write_uid write_mode write_type
  local status=0
  calyx_gate_assert_stable_owned_dir "$label parent" "$parent" || return $?
  parent_before="$(stat -c '%d:%i' -- "$parent")" || return 125
  channel_dir="$(umask 077; mktemp -d -- "$parent/.calyx-fd-channel.XXXXXXXXXXXXXXXX")" \
    || return 125
  channel_path="$channel_dir/channel"
  chmod 700 -- "$channel_dir" || status=125
  [[ "$status" -ne 0 ]] || mkfifo -m 600 -- "$channel_path" || status=125
  [[ "$status" -ne 0 ]] || exec {anchor_fd}<>"$channel_path" || status=125
  [[ "$status" -ne 0 ]] || exec {channel_read_fd}<"$channel_path" || status=125
  [[ "$status" -ne 0 ]] || exec {channel_write_fd}>"$channel_path" || status=125
  [[ -z "$anchor_fd" ]] || calyx_gate_close_numeric_fd "$anchor_fd" || status=125
  if [[ -e "$channel_path" || -L "$channel_path" ]]; then rm -- "$channel_path" || status=125; fi
  rmdir -- "$channel_dir" 2>/dev/null || status=125
  parent_after="$(stat -c '%d:%i' -- "$parent" 2>/dev/null)" || status=125
  [[ "$parent_after" == "$parent_before" ]] || status=125
  if [[ "$status" -eq 0 ]]; then
    read_meta="$(stat -Lc '%d:%i|%u|%a|%F' -- "/proc/$BASHPID/fd/$channel_read_fd")" || status=125
    write_meta="$(stat -Lc '%d:%i|%u|%a|%F' -- "/proc/$BASHPID/fd/$channel_write_fd")" || status=125
  fi
  if [[ "$status" -eq 0 ]]; then
    IFS='|' read -r read_identity read_uid read_mode read_type <<<"$read_meta"
    IFS='|' read -r write_identity write_uid write_mode write_type <<<"$write_meta"
    [[ "$read_identity" =~ ^[0-9]+:[0-9]+$ && "$write_identity" == "$read_identity" \
      && "$read_uid" == "$(id -u)" && "$write_uid" == "$read_uid" \
      && $((8#$read_mode & 0077)) -eq 0 && "$write_mode" == "$read_mode" \
      && "$read_type" == fifo && "$write_type" == fifo ]] || status=125
  fi
  if [[ "$status" -ne 0 ]]; then
    calyx_gate_discard_fd_channel "$channel_write_fd" "$channel_read_fd" || true
    echo "${CALYX_GATE_STAGE_OWNER:-gate}: ERROR could not create private $label" >&2
    return 125
  fi
  printf -v "$read_var" '%s' "$channel_read_fd"
  printf -v "$write_var" '%s' "$channel_write_fd"
}

calyx_gate_open_identity_capture() {
  local read_var="$1" write_var="$2" parent="$3"
  local record_path="" record_read_fd="" record_write_fd=""
  local parent_before="" parent_after="" read_meta="" write_meta=""
  local read_identity read_uid read_mode read_type write_identity write_uid write_mode write_type
  local status=0
  calyx_gate_assert_stable_owned_dir "identity capture parent" "$parent" || return $?
  parent_before="$(stat -c '%d:%i' -- "$parent")" || return 125
  record_path="$(umask 077; mktemp -- "$parent/.calyx-identity.XXXXXXXXXXXXXXXX")" \
    || return 125
  chmod 600 -- "$record_path" || status=125
  [[ "$status" -ne 0 ]] || exec {record_write_fd}>"$record_path" || status=125
  [[ "$status" -ne 0 ]] || exec {record_read_fd}<"$record_path" || status=125
  if [[ -e "$record_path" || -L "$record_path" ]]; then rm -- "$record_path" || status=125; fi
  parent_after="$(stat -c '%d:%i' -- "$parent" 2>/dev/null)" || status=125
  [[ "$parent_after" == "$parent_before" ]] || status=125
  if [[ "$status" -eq 0 ]]; then
    read_meta="$(stat -Lc '%d:%i|%u|%a|%F' -- "/proc/$BASHPID/fd/$record_read_fd")" \
      || status=125
    write_meta="$(stat -Lc '%d:%i|%u|%a|%F' -- "/proc/$BASHPID/fd/$record_write_fd")" \
      || status=125
  fi
  if [[ "$status" -eq 0 ]]; then
    IFS='|' read -r read_identity read_uid read_mode read_type <<<"$read_meta"
    IFS='|' read -r write_identity write_uid write_mode write_type <<<"$write_meta"
    [[ "$read_identity" =~ ^[0-9]+:[0-9]+$ && "$write_identity" == "$read_identity" \
      && "$read_uid" == "$(id -u)" && "$write_uid" == "$read_uid" \
      && $((8#$read_mode & 0077)) -eq 0 && "$write_mode" == "$read_mode" \
      && ( "$read_type" == "regular file" || "$read_type" == "regular empty file" ) \
      && "$write_type" == "$read_type" ]] || status=125
  fi
  if [[ "$status" -ne 0 ]]; then
    calyx_gate_discard_fd_channel "$record_write_fd" "$record_read_fd" || true
    echo "${CALYX_GATE_STAGE_OWNER:-gate}: ERROR could not create private identity capture" >&2
    return 125
  fi
  printf -v "$read_var" '%s' "$record_read_fd"
  printf -v "$write_var" '%s' "$record_write_fd"
}

calyx_gate_read_create_identity() {
  local read_fd="$1" root_var="$2" entry_var="$3" parsed="" parse_status=0
  local helper_fd="${4:-}" close_status=0 owns_helper=0
  calyx_gate_prepared_root_identity=""
  calyx_gate_prepared_entry_identity=""
  calyx_gate_prepared_staging_name=""
  if [[ -z "$helper_fd" ]]; then
    calyx_gate_open_identity_helper_fd helper_fd || return 125
    owns_helper=1
  fi
  if parsed="$(python3 /proc/self/fd/4 read-create 3 "$calyx_gate_identity_max_bytes" \
    "$calyx_gate_record_timeout_seconds" 3<&"$read_fd" 4<&"$helper_fd")"; then
    parse_status=0
  else
    parse_status=$?
  fi
  [[ "$owns_helper" -eq 0 ]] || calyx_gate_close_numeric_fd "$helper_fd" || close_status=125
  if [[ "$parsed" =~ ^CALYX_CREATE_PREPARED_V1\ root=([0-9]+:[0-9]+)\ entry=([0-9]+:[0-9]+)\ staging=(\.calyx-create-[0-9a-f]{64})$ ]]; then
    calyx_gate_prepared_root_identity="${BASH_REMATCH[1]}"
    calyx_gate_prepared_entry_identity="${BASH_REMATCH[2]}"
    calyx_gate_prepared_staging_name="${BASH_REMATCH[3]}"
  fi
  [[ "$parse_status" -eq 0 && "$close_status" -eq 0 ]] || return 125
  if [[ "$parsed" =~ ^CALYX_CREATE_CAPTURE_V1\ root=([0-9]+:[0-9]+)\ entry=([0-9]+:[0-9]+)(\ staging=(\.calyx-create-[0-9a-f]{64}))?$ ]]; then
    printf -v "$root_var" '%s' "${BASH_REMATCH[1]}"
    printf -v "$entry_var" '%s' "${BASH_REMATCH[2]}"
    if [[ -n "${BASH_REMATCH[4]:-}" ]]; then
      calyx_gate_prepared_root_identity="${BASH_REMATCH[1]}"
      calyx_gate_prepared_entry_identity="${BASH_REMATCH[2]}"
      calyx_gate_prepared_staging_name="${BASH_REMATCH[4]}"
    fi
    return 0
  fi
  echo "${CALYX_GATE_STAGE_OWNER:-gate}: ERROR creator identity parser returned malformed output" >&2
  return 125
}

calyx_gate_recover_prepared_create() {
  local root="$1" final_path="$2" allow_final="$3"
  local root_physical final_parent final_name selected="" name path identity
  local -a names=()
  [[ "$calyx_gate_prepared_root_identity" =~ ^[0-9]+:[0-9]+$ \
    && "$calyx_gate_prepared_entry_identity" =~ ^[0-9]+:[0-9]+$ \
    && "$calyx_gate_prepared_staging_name" =~ ^\.calyx-create-[0-9a-f]{64}$ ]] || return 125
  root_physical="$(cd "$root" && pwd -P)" || return 125
  final_parent="$(cd "$(dirname -- "$final_path")" && pwd -P)" || return 125
  final_name="$(basename -- "$final_path")"
  [[ "$final_parent" == "$root_physical" && -n "$final_name" ]] || return 125
  [[ "$(stat -c '%d:%i' -- "$root_physical")" == "$calyx_gate_prepared_root_identity" ]] || return 125
  names=("$calyx_gate_prepared_staging_name")
  [[ "$allow_final" -eq 0 ]] || names+=("$final_name")
  for name in "${names[@]}"; do
    path="$root_physical/$name"
    [[ -e "$path" || -L "$path" ]] || continue
    identity="$(stat -c '%d:%i' -- "$path")" || return 125
    [[ "$identity" == "$calyx_gate_prepared_entry_identity" && -z "$selected" ]] || {
      echo "${CALYX_GATE_STAGE_OWNER:-gate}: ERROR prepared create recovery identity mismatch: path=$path" >&2
      return 125
    }
    selected="$name"
  done
  [[ -z "$selected" ]] || calyx_gate_safe_remove_entry "$root_physical" \
    "$calyx_gate_prepared_root_identity" "$selected" \
    "$calyx_gate_prepared_entry_identity" directory prepared-create \
    "$(id -u)" "$(id -u)" 0022 0022 || return 125
  echo "${CALYX_GATE_STAGE_OWNER:-gate}: prepared_create_recovered root=$root_physical final=$final_name" >&2
}

calyx_gate_create_owned_dir() {
  local ready_var="$1" root_var="$2" entry_var="$3" capture_parent="$4"
  local recovery_root="$5" recovery_final="$6"
  shift 6
  local read_fd="" write_fd="" helper_fd="" create_status=0 record_status=0 close_status=0
  local captured_root="" captured_entry="" allow_final=1
  printf -v "$ready_var" '%s' 0
  printf -v "$root_var" '%s' ""
  printf -v "$entry_var" '%s' ""
  calyx_gate_open_identity_capture read_fd write_fd "$capture_parent" || return $?
  if ! calyx_gate_open_identity_helper_fd helper_fd; then
    create_status=125
  elif calyx_gate_run python3 /proc/self/fd/4 exec 3 "$calyx_gate_identity_max_bytes" \
    "$calyx_gate_record_timeout_seconds" "$@" 3>&"$write_fd" 4<&"$helper_fd"; then
    create_status=0
  else
    create_status=$?
  fi
  # Both atomic ownership records now reside in parent-owned unlinked bounded file bytes.
  calyx_gate_close_numeric_fd "$write_fd" || close_status=125
  if [[ -n "$helper_fd" ]] && calyx_gate_read_create_identity \
    "$read_fd" captured_root captured_entry "$helper_fd"; then
    record_status=0
  else
    record_status=$?
    [[ "$record_status" -ne 0 ]] || record_status=125
  fi
  [[ -z "$helper_fd" ]] || calyx_gate_close_numeric_fd "$helper_fd" || close_status=125
  calyx_gate_close_numeric_fd "$read_fd" || close_status=125
  if [[ "$record_status" -ne 0 && -n "$calyx_gate_prepared_staging_name" ]]; then
    [[ "$create_status" -ne 75 ]] || allow_final=0
    calyx_gate_recover_prepared_create "$recovery_root" "$recovery_final" "$allow_final" || return 125
  fi
  if [[ "$record_status" -eq 0 ]]; then
    printf -v "$root_var" '%s' "$captured_root"
    printf -v "$entry_var" '%s' "$captured_entry"
    printf -v "$ready_var" '%s' 1
  fi
  [[ "$close_status" -eq 0 ]] || return "$close_status"
  [[ "$create_status" -eq 0 ]] || return "$create_status"
  [[ "$record_status" -eq 0 ]] || return "$record_status"
}
