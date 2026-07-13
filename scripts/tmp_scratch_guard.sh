#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
CALYX_GATE_STAGE_OWNER="tmp-scratch-guard"
# shellcheck source=gate-runtime-state.sh
source "$script_dir/gate-runtime-state.sh"

usage() {
  echo "usage: tmp_scratch_guard.sh pre <baseline-file>" >&2
  echo "       tmp_scratch_guard.sh post <baseline-file> <dev:ino> <bytes> <sha256>" >&2
}

if [[ $# -lt 2 ]]; then
  usage
  exit 64
fi

mode="$1"
baseline_file="$2"
baseline_expected_identity="${3:-}"
baseline_expected_bytes="${4:-}"
baseline_expected_sha="${5:-}"
if [[ "$mode" == pre && $# -ne 2 ]] || [[ "$mode" == post && $# -ne 5 ]]; then
  usage
  exit 64
fi
tmp_root="${CALYX_TMP_ROOT:-/tmp}"
stale_minutes="${CALYX_TMP_STALE_MINUTES:-360}"
min_free_inode_percent="${CALYX_TMP_MIN_FREE_INODE_PERCENT:-5}"
min_free_block_percent="${CALYX_TMP_MIN_FREE_BLOCK_PERCENT:-1}"
current_uid="$(id -u)"
current_user="$(id -un 2>/dev/null || printf '%s' "$current_uid")"
if [[ -n "${CALYX_GATE_LOCK_PATH:-}" || -n "${CALYX_GATE_LOCK_FD:-}" ]]; then
  [[ -n "${CALYX_GATE_LOCK_PATH:-}" && -n "${CALYX_GATE_LOCK_FD:-}" ]] || exit 125
  calyx_gate_validate_lock "$CALYX_GATE_LOCK_PATH" "$CALYX_GATE_LOCK_FD" || exit $?
fi
snapshot_dir="$(cd "$(dirname "$baseline_file")" && pwd -P)" || exit 1
snapshot_dir_uid="$(stat -c '%u' -- "$snapshot_dir")" || exit 1
snapshot_dir_mode="$(stat -c '%a' -- "$snapshot_dir")" || exit 1
if [[ -L "$snapshot_dir" || "$snapshot_dir_uid" != "$current_uid" \
  || $((8#$snapshot_dir_mode & 0077)) -ne 0 ]]; then
  echo "CALYX_TMP_GUARD_UNSAFE_SNAPSHOT_DIR: path=$snapshot_dir uid=$snapshot_dir_uid mode=$snapshot_dir_mode" >&2
  exit 1
fi
snapshot_files=()
baseline_fd=""
baseline_read_file="$baseline_file"
cleanup_snapshot_files() {
  local snapshot
  for snapshot in "${snapshot_files[@]}"; do rm -f -- "$snapshot"; done
  [[ -z "$baseline_fd" ]] || eval "exec ${baseline_fd}>&-"
}
trap cleanup_snapshot_files EXIT

if [[ ! -d "$tmp_root" || -L "$tmp_root" ]]; then
  echo "CALYX_TMP_GUARD_NO_TMP_ROOT: $tmp_root is not a directory" >&2
  exit 1
fi
tmp_root_identity="$(stat -c '%d:%i' -- "$tmp_root")" || exit 1
tmp_root_uid="$(stat -c '%u' -- "$tmp_root")" || exit 1

owned_find_expr=(
  \( -name 'calyx-*'
  -o -name 'calyx_*'
  -o -name 'calyxd_mcp*'
  -o -name 'metadata-no-artifact-read-*'
  -o -name 'issue[0-9]*-fsv-*'
  -o -name 'fsv-issue*' \)
)

owned_name() {
  case "$1" in
  calyx-* | calyx_* | calyxd_mcp* | metadata-no-artifact-read-* | issue[0-9]*-fsv-* | fsv-issue*)
    return 0
    ;;
  *)
    return 1
    ;;
  esac
}

path_stat_summary() {
  local path="$1"
  stat -c 'uid=%u user=%U gid=%g group=%G mode=%A mtime=%y' -- "$path" 2>/dev/null ||
    printf 'stat=unavailable'
}

assert_current_user_owns_path() {
  local path="$1"
  local path_uid
  if ! path_uid="$(stat -c '%u' -- "$path" 2>/dev/null)"; then
    echo "CALYX_TMP_GUARD_STAT_FAILED: path=$path" >&2
    return 1
  fi

  if [[ "$path_uid" != "$current_uid" ]]; then
    echo "CALYX_TMP_GUARD_OWNER_MISMATCH: path=$path current_uid=$current_uid current_user=$current_user $(path_stat_summary "$path")" >&2
    return 1
  fi
}

current_user_owns_path() {
  local path="$1"
  local path_uid
  path_uid="$(stat -c '%u' -- "$path" 2>/dev/null)" || return 1
  [[ "$path_uid" == "$current_uid" ]]
}

snapshot_owned() {
  local outvar="$1" label="$2" snapshot_path status
  shift 2
  snapshot_path="$(mktemp -- "$snapshot_dir/.tmp-guard-enum.XXXXXXXX")" || return 1
  snapshot_files+=("$snapshot_path")
  if find "$tmp_root" -mindepth 1 -maxdepth 1 "${owned_find_expr[@]}" \
    "$@" -print0 | sort -z >"$snapshot_path"; then
    :
  else
    status=$?
    echo "CALYX_TMP_GUARD_ENUMERATION_FAILED: label=$label status=$status root=$tmp_root" >&2
    return "$status"
  fi
  printf -v "$outvar" '%s' "$snapshot_path"
}

remove_owned_path() {
  local path="$1" identity name
  case "$path" in
  "$tmp_root"/*) ;;
  *)
    echo "CALYX_TMP_GUARD_REFUSE_OUTSIDE_TMP: $path" >&2
    return 1
    ;;
  esac

  name="$(basename -- "$path")"
  if ! owned_name "$name"; then
    echo "CALYX_TMP_GUARD_REFUSE_UNOWNED: $path" >&2
    return 1
  fi

  assert_current_user_owns_path "$path"
  [[ "$path" != *$'\n'* && "$path" != *$'\r'* ]] || {
    echo "CALYX_TMP_GUARD_REFUSE_MULTILINE: $path" >&2
    return 1
  }
  identity="$(stat -c '%d:%i' -- "$path")" || return 1
  if [[ -d "$path" && ! -L "$path" ]]; then
    calyx_gate_assert_no_mounts_at_or_below "$path" "tmp-guard path" || return $?
  fi
  if ! calyx_gate_safe_remove_entry "$tmp_root" "$tmp_root_identity" "$name" \
    "$identity" any tmp-guard "$current_uid" "$tmp_root_uid" "" ""; then
    echo "CALYX_TMP_GUARD_REMOVE_FAILED: path=$path current_uid=$current_uid current_user=$current_user $(path_stat_summary "$path")" >&2
    return 1
  fi
  [[ ! -e "$path" && ! -L "$path" ]]
}

cleanup_stale_owned() {
  local removed=0 skipped_unowned=0 snapshot
  snapshot_owned snapshot stale -mmin "+$stale_minutes" || return $?
  while IFS= read -r -d '' path; do
    if ! current_user_owns_path "$path"; then
      echo "calyx_tmp_guard_stale_unowned_skipped: path=$path $(path_stat_summary "$path")" >&2
      skipped_unowned=$((skipped_unowned + 1))
      continue
    fi
    remove_owned_path "$path"
    removed=$((removed + 1))
  done <"$snapshot"
  echo "calyx_tmp_guard_stale_removed=$removed"
  echo "calyx_tmp_guard_stale_unowned_skipped=$skipped_unowned"
}

cleanup_all_owned() {
  local removed=0 skipped_unowned=0 snapshot
  snapshot_owned snapshot all || return $?
  while IFS= read -r -d '' path; do
    if ! current_user_owns_path "$path"; then
      echo "calyx_tmp_guard_all_unowned_skipped: path=$path $(path_stat_summary "$path")" >&2
      skipped_unowned=$((skipped_unowned + 1))
      continue
    fi
    remove_owned_path "$path"
    removed=$((removed + 1))
  done <"$snapshot"
  echo "calyx_tmp_guard_all_removed=$removed"
  echo "calyx_tmp_guard_all_unowned_skipped=$skipped_unowned"
}

cleanup_new_owned() {
  # Snapshot the baseline into memory first. Some direct/legacy callers keep
  # it under $tmp_root with an owned-pattern name; removing entries while
  # re-reading such a baseline can delete the reference mid-sweep and make
  # every remaining entry look "new". Full gates now keep it in private TMPDIR.
  local removed=0 skipped_unowned=0 snapshot grep_status
  snapshot_owned snapshot new || return $?
  while IFS= read -r -d '' path; do
    grep_status=0
    grep -Fzxq -- "$path" "$baseline_read_file" || grep_status=$?
    if [[ "$grep_status" -eq 1 ]]; then
      if ! current_user_owns_path "$path"; then
        echo "calyx_tmp_guard_new_unowned_skipped: path=$path $(path_stat_summary "$path")" >&2
        skipped_unowned=$((skipped_unowned + 1))
        continue
      fi
      remove_owned_path "$path"
      removed=$((removed + 1))
    elif [[ "$grep_status" -ne 0 ]]; then
      echo "CALYX_TMP_GUARD_BASELINE_READ_FAILED: status=$grep_status path=$baseline_file" >&2
      return "$grep_status"
    fi
  done <"$snapshot"
  echo "calyx_tmp_guard_new_removed=$removed"
  echo "calyx_tmp_guard_new_unowned_skipped=$skipped_unowned"
}

baseline_actual_state() {
  local path="$1" identity bytes sha
  identity="$(stat -Lc '%d:%i' -- "$path" 2>/dev/null)" || return 1
  bytes="$(stat -Lc '%s' -- "$path" 2>/dev/null)" || return 1
  sha="$(sha256sum -- "$path" 2>/dev/null | awk '{print $1}')" || return 1
  printf '%s %s %s\n' "$identity" "$bytes" "$sha"
}

assert_bound_baseline() {
  local label="$1" actual identity bytes sha
  actual="$(baseline_actual_state "$baseline_read_file")" || {
    echo "CALYX_TMP_GUARD_BASELINE_READ_FAILED: label=$label path=$baseline_file" >&2
    return 1
  }
  read -r identity bytes sha <<<"$actual"
  if [[ "$identity" != "$baseline_expected_identity" \
    || "$bytes" != "$baseline_expected_bytes" || "$sha" != "$baseline_expected_sha" ]]; then
    echo "CALYX_TMP_GUARD_BASELINE_CHANGED: label=$label expected_identity=$baseline_expected_identity actual_identity=$identity expected_bytes=$baseline_expected_bytes actual_bytes=$bytes expected_sha=$baseline_expected_sha actual_sha=$sha" >&2
    return 1
  fi
}

open_bound_baseline() {
  [[ "$baseline_expected_identity" =~ ^[0-9]+:[0-9]+$ \
    && "$baseline_expected_bytes" =~ ^[0-9]+$ \
    && "$baseline_expected_sha" =~ ^[0-9a-f]{64}$ ]] || {
    echo "CALYX_TMP_GUARD_INVALID_BASELINE_PROOF" >&2
    return 1
  }
  exec {baseline_fd}<"$baseline_file" || {
    echo "CALYX_TMP_GUARD_MISSING_BASELINE: $baseline_file" >&2
    return 1
  }
  baseline_read_file="/proc/$BASHPID/fd/$baseline_fd"
  [[ "$(stat -Lc '%u:%a' -- "$baseline_read_file")" == "$current_uid:600" ]] || {
    echo "CALYX_TMP_GUARD_UNSAFE_BASELINE: $baseline_file" >&2
    return 1
  }
  assert_bound_baseline open
}

inode_snapshot() {
  df -Pi "$tmp_root" | awk 'NR == 2 {
    if ($2 !~ /^[0-9]+$/ || $2 == 0 || $3 !~ /^[0-9]+$/ || $4 !~ /^[0-9]+$/) {
      printf "calyx_tmp_guard_inodes total=unavailable used=unavailable free=unavailable used_percent=unavailable\n"
      exit
    }
    used_percent = $5
    sub(/%$/, "", used_percent)
    printf "calyx_tmp_guard_inodes total=%s used=%s free=%s used_percent=%s\n",
      $2, $3, $4, used_percent
  }'
}

block_snapshot() {
  df -Pk "$tmp_root" | awk 'NR == 2 {
    if ($2 !~ /^[0-9]+$/ || $2 == 0 || $3 !~ /^[0-9]+$/ || $4 !~ /^[0-9]+$/) {
      printf "calyx_tmp_guard_blocks total=unavailable used=unavailable free=unavailable used_percent=unavailable\n"
      exit
    }
    used_percent = $5
    sub(/%$/, "", used_percent)
    printf "calyx_tmp_guard_blocks total=%s used=%s free=%s used_percent=%s\n",
      $2, $3, $4, used_percent
  }'
}

free_inode_percent() {
  df -Pi "$tmp_root" | awk 'NR == 2 {
    if ($2 !~ /^[0-9]+$/ || $2 == 0 || $4 !~ /^[0-9]+$/) {
      exit 1
    }
    printf "%.0f\n", ($4 * 100) / $2
  }'
}

free_block_percent() {
  df -Pk "$tmp_root" | awk 'NR == 2 {
    if ($2 !~ /^[0-9]+$/ || $2 == 0 || $4 !~ /^[0-9]+$/) {
      exit 1
    }
    printf "%.0f\n", ($4 * 100) / $2
  }'
}

free_tmp_percent() {
  free_inode_percent 2>/dev/null || free_block_percent
}

min_tmp_free_percent() {
  if free_inode_percent >/dev/null 2>&1; then
    printf '%s\n' "$min_free_inode_percent"
  else
    printf '%s\n' "$min_free_block_percent"
  fi
}

inode_headroom_ok() {
  local free_percent min_free_percent
  free_percent="$(free_tmp_percent)"
  min_free_percent="$(min_tmp_free_percent)"
  ((free_percent >= min_free_percent))
}

assert_inode_headroom() {
  local free_percent min_free_percent
  free_percent="$(free_tmp_percent)"
  min_free_percent="$(min_tmp_free_percent)"
  if ! inode_headroom_ok; then
    echo "CALYX_TMP_GUARD_LOW_TMP_HEADROOM: $tmp_root free inode/block percent $free_percent < $min_free_percent" >&2
    exit 1
  fi
}

case "$mode" in
pre)
  snapshot_owned baseline_snapshot baseline || exit $?
  mv -T -- "$baseline_snapshot" "$baseline_file"
  echo "calyx_tmp_guard_mode=pre"
  baseline_count=0
  while IFS= read -r -d '' _; do baseline_count=$((baseline_count + 1)); done <"$baseline_file"
  echo "calyx_tmp_guard_existing_owned=$baseline_count"
  inode_snapshot
  block_snapshot
  cleanup_stale_owned
  inode_snapshot
  block_snapshot
  if ! inode_headroom_ok; then
    cleanup_all_owned
    inode_snapshot
    block_snapshot
  fi
  assert_inode_headroom
  ;;
post)
  open_bound_baseline || exit $?
  echo "calyx_tmp_guard_mode=post"
  cleanup_new_owned
  cleanup_stale_owned
  assert_bound_baseline final
  inode_snapshot
  block_snapshot
  assert_inode_headroom
  ;;
*)
  usage
  exit 64
  ;;
esac
