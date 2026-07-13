#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
# shellcheck source=gate-runtime-state.sh
source "$script_dir/gate-runtime-state.sh"

usage() {
  echo "usage: check-tmpdir.sh create <calyx-tmp-root>" >&2
  echo "       check-tmpdir.sh create-at <calyx-tmp-root> <exact-check-path>" >&2
  echo "       check-tmpdir.sh create-target <target-scratch-root> <check|fuzz-gate|public-mirror-gate>" >&2
  echo "       check-tmpdir.sh create-target-at <target-scratch-root> <prefix> <exact-target-path>" >&2
  echo "       check-tmpdir.sh create-target-path-at <target-scratch-root> <exact-target-path>" >&2
  echo "       check-tmpdir.sh validate <calyx-tmp-root> <run-dir> [<root-dev:ino> <run-dev:ino>]" >&2
  echo "       check-tmpdir.sh validate-target <target-scratch-root> <target-dir> <root-dev:ino> <target-dev:ino>" >&2
  echo "       check-tmpdir.sh cleanup <calyx-tmp-root> <run-dir> [<root-dev:ino> <run-dev:ino>]" >&2
}

fail() {
  local code="$1"
  shift
  echo "CALYX_CHECK_TMPDIR_REFUSED: $*" >&2
  exit "$code"
}

current_uid="$(id -u)"

assert_absolute_path() {
  local label="$1" path="$2"
  [[ "$path" == /* ]] || fail 1 "$label is not absolute: $path"
  [[ "$path" != *$'\n'* ]] || fail 1 "$label contains a newline"
}

assert_owned_directory() {
  local label="$1" path="$2" path_uid
  [[ ! -L "$path" ]] || fail 1 "$label is a symlink: $path"
  [[ -d "$path" ]] || fail 1 "$label is not a directory: $path"
  path_uid="$(stat -c '%u' -- "$path" 2>/dev/null)" ||
    fail 1 "cannot stat $label: $path"
  [[ "$path_uid" == "$current_uid" ]] ||
    fail 1 "$label owner uid $path_uid does not match current uid $current_uid: $path"
}

assert_stable_permissions() {
  local label="$1" path="$2" mode mode_value
  mode="$(stat -c '%a' -- "$path" 2>/dev/null)" ||
    fail 1 "cannot read permissions for $label: $path"
  mode_value=$((8#$mode))
  (( (mode_value & 0022) == 0 )) ||
    fail 1 "$label is group/world writable (mode=$mode): $path"
}

path_identity() {
  stat -c '%d:%i' -- "$1" 2>/dev/null || fail 1 "cannot read path identity: $1"
}

assert_expected_identity() {
  local label="$1" path="$2" expected="$3" actual
  [[ -z "$expected" ]] && return 0
  [[ "$expected" =~ ^[0-9]+:[0-9]+$ ]] || fail 1 "invalid expected identity for $label: $expected"
  actual="$(path_identity "$path")"
  [[ "$actual" == "$expected" ]] ||
    fail 1 "$label identity changed: expected=$expected actual=$actual path=$path"
}

assert_no_mounts_at_or_below() {
  local path="$1"
  calyx_gate_assert_no_mounts_at_or_below "$path" "run dir"
}

resolve_directory() {
  (cd -- "$1" && pwd -P)
}

prepare_root() {
  local root="$1" create_missing="$2" expected_name="$3" parent root_name
  assert_absolute_path "tmp root" "$root"
  root_name="$(basename -- "$root")"
  [[ "$root_name" == "$expected_name" ]] ||
    fail 1 "root basename must be '$expected_name': $root"

  parent="$(dirname -- "$root")"
  assert_owned_directory "tmp root parent" "$parent"
  assert_stable_permissions "tmp root parent" "$parent"
  [[ ! -L "$root" ]] || fail 1 "tmp root is a symlink: $root"
  if [[ ! -e "$root" ]]; then
    [[ "$create_missing" -eq 1 ]] || fail 1 "tmp root does not exist: $root"
    umask 077
    mkdir -- "$root" 2>/dev/null || [[ -d "$root" ]] ||
      fail 1 "cannot create tmp root: $root"
  fi
  assert_owned_directory "tmp root" "$root"
  assert_stable_permissions "tmp root" "$root"
  resolve_directory "$root"
}

assert_direct_prefixed_path() {
  local root_resolved="$1" path="$2" prefix="$3" label="$4"
  local path_parent path_name
  assert_absolute_path "$label" "$path"
  path_name="$(basename -- "$path")"
  [[ "$path_name" == "$prefix".* && "$path_name" != "$prefix." ]] ||
    fail 1 "$label basename is not $prefix.<suffix>: $path"
  path_parent="$(dirname -- "$path")"
  assert_owned_directory "$label parent" "$path_parent"
  assert_stable_permissions "$label parent" "$path_parent"
  path_parent="$(resolve_directory "$path_parent")"
  [[ "$path_parent" == "$root_resolved" ]] ||
    fail 1 "$label is not a direct child of $root_resolved: $path"
}

assert_direct_child_path() {
  local root_resolved="$1" path="$2" label="$3" path_parent path_name
  assert_absolute_path "$label" "$path"
  path_name="$(basename -- "$path")"
  [[ -n "$path_name" && "$path_name" != "." && "$path_name" != ".." ]] ||
    fail 1 "$label has an invalid basename: $path"
  path_parent="$(dirname -- "$path")"
  assert_owned_directory "$label parent" "$path_parent"
  assert_stable_permissions "$label parent" "$path_parent"
  path_parent="$(resolve_directory "$path_parent")"
  [[ "$path_parent" == "$root_resolved" ]] ||
    fail 1 "$label is not a direct child of $root_resolved: $path"
}

arm_partial_create_traps() {
  created_path=""
  created_root_path=""
  created_root_identity=""
  created_identity=""
  created_by_helper=0
  pending_signal_status=0
  cleanup_partial_create() {
    local status=$?
    trap - EXIT HUP INT TERM
    if [[ "$status" -ne 0 && "$created_by_helper" -eq 1 ]]; then
      if [[ -z "$created_path" || -z "$created_root_path" \
        || ! "$created_root_identity" =~ ^[0-9]+:[0-9]+$ \
        || ! "$created_identity" =~ ^[0-9]+:[0-9]+$ ]] \
        || ! calyx_gate_safe_remove_entry "$created_root_path" \
          "$created_root_identity" "$(basename -- "$created_path")" \
          "$created_identity" directory partial-create "$current_uid" \
          "$current_uid" 0022 0022; then
        echo "CALYX_CHECK_TMPDIR_PARTIAL_CREATE_RECOVERY_REQUIRED: path=$created_path" >&2
        status=125
      fi
    fi
    exit "$status"
  }
  trap cleanup_partial_create EXIT
  trap 'pending_signal_status=129' HUP
  trap 'pending_signal_status=130' INT
  trap 'pending_signal_status=143' TERM
}

create_exact_directory() {
  local label="$1" path="$2" root_path="$3" expected_root="$4"
  local creator="$script_dir/gate_owned_dir_create.py" creator_fd=""
  local creator_uid creator_mode creator_identity path_identity record create_status
  if [[ -e "$path" || -L "$path" ]]; then
    echo "CALYX_CHECK_TMPDIR_COLLISION: $label already exists: $path" >&2
    return 75
  fi
  created_path="$path"
  [[ -f "$creator" && ! -L "$creator" ]] || fail 125 "owned-directory creator is unavailable"
  exec {creator_fd}<"$creator" || fail 125 "cannot open owned-directory creator"
  creator_uid="$(stat -Lc '%u' -- "/proc/$BASHPID/fd/$creator_fd")" || create_status=125
  creator_mode="$(stat -Lc '%a' -- "/proc/$BASHPID/fd/$creator_fd")" || create_status=125
  creator_identity="$(stat -Lc '%d:%i' -- "/proc/$BASHPID/fd/$creator_fd")" || create_status=125
  path_identity="$(stat -c '%d:%i' -- "$creator")" || create_status=125
  if [[ "${create_status:-0}" -ne 0 || "$creator_uid" != "$current_uid" \
    || $((8#$creator_mode & 0022)) -ne 0 || "$creator_identity" != "$path_identity" ]]; then
    exec {creator_fd}>&-
    fail 125 "owned-directory creator identity or policy check failed"
  fi
  if record="$(python3 "/proc/$BASHPID/fd/$creator_fd" --root "$root_path" \
    --root-id "$expected_root" --name "$(basename -- "$path")" --uid "$current_uid")"; then
    create_status=0
  else
    create_status=$?
  fi
  exec {creator_fd}>&-
  if [[ "$create_status" -eq 75 ]]; then
    echo "CALYX_CHECK_TMPDIR_COLLISION: $label appeared during create: $path" >&2
    return 75
  fi
  [[ "$create_status" -eq 0 ]] || return "$create_status"
  if [[ ! "$record" =~ ^CALYX_CREATED_V1\ root=([0-9]+:[0-9]+)\ entry=([0-9]+:[0-9]+)$ \
    || "${BASH_REMATCH[1]:-}" != "$expected_root" ]]; then
    echo "CALYX_CHECK_TMPDIR_PARTIAL_CREATE_RECOVERY_REQUIRED: path=$path" >&2
    return 125
  fi
  created_root_path="$root_path"
  created_root_identity="${BASH_REMATCH[1]}"
  created_identity="${BASH_REMATCH[2]}"
  created_by_helper=1
  trap 'exit 129' HUP
  trap 'exit 130' INT
  trap 'exit 143' TERM
  [[ "$pending_signal_status" -eq 0 ]] || exit "$pending_signal_status"
}

publish_create_identity() {
  local root_path="$1" entry_path="$2" publish_fd="${CALYX_CREATE_ID_FD:-}"
  local root_fd="" entry_fd="" root_id entry_id
  [[ -n "$publish_fd" ]] || return 0
  [[ "$publish_fd" =~ ^[0-9]+$ && -e "/proc/$BASHPID/fd/$publish_fd" ]] ||
    fail 125 "creator identity publication fd is unavailable"
  exec {root_fd}<"$root_path" || fail 125 "cannot open created root for identity publication: $root_path"
  exec {entry_fd}<"$entry_path" || {
    eval "exec ${root_fd}>&-"
    fail 125 "cannot open created entry for identity publication: $entry_path"
  }
  root_id="$(stat -Lc '%d:%i' -- "/proc/$BASHPID/fd/$root_fd")" || fail 125 "cannot read opened root identity"
  entry_id="$(stat -Lc '%d:%i' -- "/proc/$BASHPID/fd/$entry_fd")" || fail 125 "cannot read opened entry identity"
  [[ "$root_id" == "$created_root_identity" && "$entry_id" == "$created_identity" ]] ||
    fail 125 "created identities changed before publication"
  printf 'CALYX_CREATE_ID_V1 root=%s entry=%s\n' "$root_id" "$entry_id" >&"$publish_fd" ||
    fail 125 "cannot publish created identities"
  eval "exec ${entry_fd}>&-"
  eval "exec ${root_fd}>&-"
}

create_run_dir() {
  local root="$1" requested_path="${2:-}" root_resolved root_identity run_dir=""
  local run_resolved attempt create_status
  local created_path="" created_root_path="" created_root_identity=""
  local created_identity="" created_by_helper=0 pending_signal_status=0
  root_resolved="$(prepare_root "$root" 1 tmp)"
  root_identity="$(path_identity "$root_resolved")"
  arm_partial_create_traps

  for attempt in {1..64}; do
    if [[ -n "$requested_path" ]]; then
      run_dir="$requested_path"
    else
      run_dir="$root_resolved/check.${BASHPID}.${RANDOM}${RANDOM}${RANDOM}"
    fi
    assert_direct_prefixed_path "$root_resolved" "$run_dir" check "run dir"
    if create_exact_directory "run dir" "$run_dir" "$root_resolved" "$root_identity"; then
      break
    else
      create_status=$?
    fi
    [[ "$pending_signal_status" -eq 0 ]] || exit "$pending_signal_status"
    if [[ -n "$requested_path" || "$create_status" -ne 75 ]]; then
      trap - EXIT HUP INT TERM
      return "$create_status"
    fi
  done
  [[ "$created_by_helper" -eq 1 ]] || fail 1 "exhausted run-dir collision budget"

  assert_owned_directory "run dir" "$run_dir"
  assert_stable_permissions "run dir" "$run_dir"
  run_resolved="$(resolve_directory "$run_dir")"
  [[ "$run_resolved" == "$run_dir" ]] ||
    fail 1 "run dir did not resolve to the created path: path=$run_dir resolved=$run_resolved"
  [[ "$(path_identity "$root_resolved")" == "$root_identity" ]] ||
    fail 1 "tmp root identity changed during create: $root_resolved"

  publish_create_identity "$root_resolved" "$run_resolved"
  [[ "$pending_signal_status" -eq 0 ]] || exit "$pending_signal_status"
  echo "calyx_check_tmpdir_created root=$root_resolved run=$run_resolved uid=$current_uid" >&2
  [[ "${CALYX_CREATE_ID_FD:-}" == 1 ]] || printf '%s\n' "$run_resolved"
  trap - EXIT HUP INT TERM
}

create_target_dir() {
  local root="$1" prefix="$2" requested_path="${3:-}"
  local allow_unprefixed="${4:-0}"
  local root_resolved root_identity target_dir="" target_resolved attempt create_status
  local created_path="" created_root_path="" created_root_identity=""
  local created_identity="" created_by_helper=0 pending_signal_status=0
  case "$prefix" in
    check | fuzz-gate | public-mirror-gate) ;;
    *) fail 1 "unsupported Cargo target prefix: $prefix" ;;
  esac
  root_resolved="$(prepare_root "$root" 1 target-scratch)"
  root_identity="$(path_identity "$root_resolved")"
  arm_partial_create_traps

  for attempt in {1..64}; do
    if [[ -n "$requested_path" ]]; then
      target_dir="$requested_path"
    else
      target_dir="$root_resolved/$prefix.${BASHPID}.${RANDOM}${RANDOM}${RANDOM}"
    fi
    if [[ "$allow_unprefixed" -eq 1 ]]; then
      assert_direct_child_path "$root_resolved" "$target_dir" "Cargo target dir"
    else
      assert_direct_prefixed_path "$root_resolved" "$target_dir" "$prefix" "Cargo target dir"
    fi
    if create_exact_directory "Cargo target dir" "$target_dir" "$root_resolved" "$root_identity"; then
      break
    else
      create_status=$?
    fi
    [[ "$pending_signal_status" -eq 0 ]] || exit "$pending_signal_status"
    if [[ -n "$requested_path" || "$create_status" -ne 75 ]]; then
      trap - EXIT HUP INT TERM
      return "$create_status"
    fi
  done
  [[ "$created_by_helper" -eq 1 ]] || fail 1 "exhausted Cargo target collision budget"

  assert_owned_directory "Cargo target dir" "$target_dir"
  assert_stable_permissions "Cargo target dir" "$target_dir"
  target_resolved="$(resolve_directory "$target_dir")"
  [[ "$(dirname -- "$target_resolved")" == "$root_resolved" ]] ||
    fail 1 "Cargo target dir escaped scratch root: root=$root_resolved target=$target_resolved"
  [[ "$(path_identity "$root_resolved")" == "$root_identity" ]] ||
    fail 1 "Cargo target root identity changed during create: $root_resolved"
  publish_create_identity "$root_resolved" "$target_resolved"
  [[ "$pending_signal_status" -eq 0 ]] || exit "$pending_signal_status"
  echo "calyx_gate_target_created root=$root_resolved target=$target_resolved uid=$current_uid" >&2
  [[ "${CALYX_CREATE_ID_FD:-}" == 1 ]] || printf '%s\n' "$target_resolved"
  trap - EXIT HUP INT TERM
}

validate_run_dir() {
  local root="$1" run_dir="$2" expected_root="${3:-}" expected_run="${4:-}"
  local root_resolved run_resolved
  root_resolved="$(prepare_root "$root" 0 tmp)"
  assert_expected_identity "tmp root" "$root_resolved" "$expected_root"
  assert_direct_prefixed_path "$root_resolved" "$run_dir" check "run dir"
  assert_owned_directory "run dir" "$run_dir"
  assert_stable_permissions "run dir" "$run_dir"
  assert_expected_identity "run dir" "$run_dir" "$expected_run"
  run_resolved="$(resolve_directory "$run_dir")"
  [[ "$(dirname -- "$run_resolved")" == "$root_resolved" ]] ||
    fail 1 "resolved run dir escaped tmp root: root=$root_resolved run=$run_resolved"
  echo "calyx_check_tmpdir_valid root=$root_resolved run=$run_resolved uid=$current_uid" >&2
}

validate_target_dir() {
  local root="$1" target="$2" expected_root="$3" expected_target="$4"
  local root_resolved target_resolved
  [[ -n "$expected_root" && -n "$expected_target" ]] ||
    fail 1 "target validation requires root and target identities"
  root_resolved="$(prepare_root "$root" 0 target-scratch)"
  assert_expected_identity "Cargo target root" "$root_resolved" "$expected_root"
  assert_direct_child_path "$root_resolved" "$target" "Cargo target dir"
  assert_owned_directory "Cargo target dir" "$target"
  assert_stable_permissions "Cargo target dir" "$target"
  assert_expected_identity "Cargo target dir" "$target" "$expected_target"
  target_resolved="$(resolve_directory "$target")"
  [[ "$(dirname -- "$target_resolved")" == "$root_resolved" ]] ||
    fail 1 "resolved Cargo target escaped scratch root: root=$root_resolved target=$target_resolved"
  echo "calyx_gate_target_valid root=$root_resolved target=$target_resolved uid=$current_uid" >&2
}

cleanup_run_dir() {
  local root="$1" run_dir="$2" expected_root="${3:-}" expected_run="${4:-}"
  local root_resolved root_identity run_resolved run_identity entries first_entry dir_blocks run_name
  root_resolved="$(prepare_root "$root" 0 tmp)"
  assert_expected_identity "tmp root" "$root_resolved" "$expected_root"
  root_identity="$(path_identity "$root_resolved")"
  assert_direct_prefixed_path "$root_resolved" "$run_dir" check "run dir"

  if [[ ! -e "$run_dir" && ! -L "$run_dir" ]]; then
    [[ -z "$expected_run" ]] || fail 1 "expected run dir is absent: identity=$expected_run path=$run_dir"
    echo "calyx_check_tmpdir_absent path=$run_dir"
    return 0
  fi

  assert_owned_directory "run dir" "$run_dir"
  assert_stable_permissions "run dir" "$run_dir"
  [[ -n "$expected_root" && -n "$expected_run" ]] ||
    fail 1 "existing run-dir cleanup requires root and run identities: $run_dir"
  assert_expected_identity "run dir" "$run_dir" "$expected_run"
  run_resolved="$(resolve_directory "$run_dir")"
  [[ "$(dirname -- "$run_resolved")" == "$root_resolved" ]] ||
    fail 1 "resolved run dir escaped tmp root: root=$root_resolved run=$run_resolved"
  run_identity="$(path_identity "$run_resolved")"
  assert_no_mounts_at_or_below "$run_resolved" ||
    fail $? "cannot prove run-dir mount boundary: $run_resolved"

  first_entry="$(find "$run_resolved" -xdev -mindepth 1 -maxdepth 1 -print -quit)" ||
    fail 125 "cannot sample run-dir contents before cleanup: $run_resolved"
  entries=0
  [[ -z "$first_entry" ]] || entries=1
  dir_blocks="$(stat -c '%b' -- "$run_resolved")" ||
    fail 125 "cannot read run-dir allocation before cleanup: $run_resolved"
  echo "calyx_check_tmpdir_cleanup_before path=$run_resolved entries_sampled=$entries dir_blocks=$dir_blocks"
  [[ "$(path_identity "$root_resolved")" == "$root_identity" ]] ||
    fail 1 "tmp root identity changed before cleanup: $root_resolved"
  [[ "$(path_identity "$run_resolved")" == "$run_identity" ]] ||
    fail 1 "run dir identity changed before cleanup: $run_resolved"
  assert_expected_identity "tmp root" "$root_resolved" "$expected_root"
  assert_expected_identity "run dir" "$run_resolved" "$expected_run"
  assert_no_mounts_at_or_below "$run_resolved" ||
    fail $? "cannot prove run-dir mount boundary: $run_resolved"
  run_name="$(basename -- "$run_resolved")"
  calyx_gate_safe_remove_entry "$root_resolved" "$root_identity" "$run_name" \
    "$run_identity" directory check-tmpdir "$current_uid" "$current_uid" 0022 0022 ||
    fail $? "safe removal failed for run dir: $run_resolved"
  [[ ! -e "$run_resolved" && ! -L "$run_resolved" ]] ||
    fail 1 "run dir remains after cleanup: run=$run_resolved"
  echo "calyx_check_tmpdir_absent path=$run_resolved"
}

case "${1:-}" in
create)
  [[ $# -eq 2 ]] || { usage; exit 64; }
  create_run_dir "$2"
  ;;
create-at)
  [[ $# -eq 3 ]] || { usage; exit 64; }
  create_run_dir "$2" "$3"
  ;;
create-target)
  [[ $# -eq 3 ]] || { usage; exit 64; }
  create_target_dir "$2" "$3"
  ;;
create-target-at)
  [[ $# -eq 4 ]] || { usage; exit 64; }
  create_target_dir "$2" "$3" "$4"
  ;;
create-target-path-at)
  [[ $# -eq 3 ]] || { usage; exit 64; }
  create_target_dir "$2" public-mirror-gate "$3" 1
  ;;
validate)
  [[ $# -eq 3 || $# -eq 5 ]] || { usage; exit 64; }
  validate_run_dir "$2" "$3" "${4:-}" "${5:-}"
  ;;
validate-target)
  [[ $# -eq 5 ]] || { usage; exit 64; }
  validate_target_dir "$2" "$3" "$4" "$5"
  ;;
cleanup)
  [[ $# -eq 3 || $# -eq 5 ]] || { usage; exit 64; }
  cleanup_run_dir "$2" "$3" "${4:-}" "${5:-}"
  ;;
*)
  usage
  exit 64
  ;;
esac
