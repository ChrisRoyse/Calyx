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
  local output_var="$1" raw_output raw_target decoded_target mount_output=""
  command -v findmnt >/dev/null 2>&1 || {
    echo "${CALYX_GATE_STAGE_OWNER:-gate}: ERROR findmnt is required for mount-safe cleanup" >&2
    return 127
  }
  if ! raw_output="$(findmnt -rn -o TARGET)"; then
    echo "${CALYX_GATE_STAGE_OWNER:-gate}: ERROR findmnt failed while proving cleanup boundaries" >&2
    return 125
  fi
  while IFS= read -r raw_target; do
    [[ -n "$raw_target" ]] || continue
    decoded_target="$(printf '%b' "$raw_target")" || return 125
    mount_output+="${mount_output:+$'\n'}$decoded_target"
  done <<<"$raw_output"
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

gate_runtime_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
# shellcheck source=gate-identity-capture.sh
source "$gate_runtime_dir/gate-identity-capture.sh"

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
