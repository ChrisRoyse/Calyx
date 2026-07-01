#!/usr/bin/env bash
set -euo pipefail

usage() {
  echo "usage: tmp_scratch_guard.sh pre|post <baseline-file>" >&2
}

if [[ $# -ne 2 ]]; then
  usage
  exit 64
fi

mode="$1"
baseline_file="$2"
tmp_root="${CALYX_TMP_ROOT:-/tmp}"
stale_minutes="${CALYX_TMP_STALE_MINUTES:-360}"
min_free_percent="${CALYX_TMP_MIN_FREE_INODE_PERCENT:-5}"
current_uid="$(id -u)"
current_user="$(id -un 2>/dev/null || printf '%s' "$current_uid")"

if [[ ! -d "$tmp_root" ]]; then
  echo "CALYX_TMP_GUARD_NO_TMP_ROOT: $tmp_root is not a directory" >&2
  exit 1
fi

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

list_owned() {
  find "$tmp_root" -mindepth 1 -maxdepth 1 "${owned_find_expr[@]}" -print | sort
}

remove_owned_path() {
  local path="$1"
  case "$path" in
  "$tmp_root"/*) ;;
  *)
    echo "CALYX_TMP_GUARD_REFUSE_OUTSIDE_TMP: $path" >&2
    return 1
    ;;
  esac

  local name
  name="$(basename -- "$path")"
  if ! owned_name "$name"; then
    echo "CALYX_TMP_GUARD_REFUSE_UNOWNED: $path" >&2
    return 1
  fi

  assert_current_user_owns_path "$path"

  if ! rm -rf -- "$path"; then
    echo "CALYX_TMP_GUARD_REMOVE_FAILED: path=$path current_uid=$current_uid current_user=$current_user $(path_stat_summary "$path")" >&2
    return 1
  fi
}

cleanup_stale_owned() {
  local removed=0
  while IFS= read -r path; do
    [[ -n "$path" ]] || continue
    remove_owned_path "$path"
    removed=$((removed + 1))
  done < <(
    find "$tmp_root" -mindepth 1 -maxdepth 1 "${owned_find_expr[@]}" \
      -mmin "+$stale_minutes" -print
  )
  echo "calyx_tmp_guard_stale_removed=$removed"
}

cleanup_all_owned() {
  local removed=0
  while IFS= read -r path; do
    [[ -n "$path" ]] || continue
    remove_owned_path "$path"
    removed=$((removed + 1))
  done < <(list_owned)
  echo "calyx_tmp_guard_all_removed=$removed"
}

cleanup_new_owned() {
  # Snapshot the baseline into memory first: the baseline file itself lives
  # under $tmp_root and matches the owned patterns, so removing entries while
  # re-reading it from disk can delete the reference mid-sweep and make every
  # remaining entry look "new".
  local removed=0 baseline_content
  baseline_content="$(cat -- "$baseline_file")"
  while IFS= read -r path; do
    [[ -n "$path" ]] || continue
    if ! grep -Fxq "$path" <<<"$baseline_content"; then
      remove_owned_path "$path"
      removed=$((removed + 1))
    fi
  done < <(list_owned)
  echo "calyx_tmp_guard_new_removed=$removed"
}

inode_snapshot() {
  df -Pi "$tmp_root" | awk 'NR == 2 {
    used_percent = $5
    sub(/%$/, "", used_percent)
    printf "calyx_tmp_guard_inodes total=%s used=%s free=%s used_percent=%s\n",
      $2, $3, $4, used_percent
  }'
}

free_inode_percent() {
  df -Pi "$tmp_root" | awk 'NR == 2 { printf "%.0f\n", ($4 * 100) / $2 }'
}

inode_headroom_ok() {
  local free_percent
  free_percent="$(free_inode_percent)"
  ((free_percent >= min_free_percent))
}

assert_inode_headroom() {
  local free_percent
  free_percent="$(free_inode_percent)"
  if ! inode_headroom_ok; then
    echo "CALYX_TMP_GUARD_LOW_INODES: $tmp_root free inode percent $free_percent < $min_free_percent" >&2
    exit 1
  fi
}

case "$mode" in
pre)
  list_owned >"$baseline_file"
  echo "calyx_tmp_guard_mode=pre"
  echo "calyx_tmp_guard_existing_owned=$(wc -l <"$baseline_file")"
  inode_snapshot
  cleanup_stale_owned
  inode_snapshot
  if ! inode_headroom_ok; then
    cleanup_all_owned
    inode_snapshot
  fi
  assert_inode_headroom
  ;;
post)
  if [[ ! -f "$baseline_file" ]]; then
    echo "CALYX_TMP_GUARD_MISSING_BASELINE: $baseline_file" >&2
    exit 1
  fi
  echo "calyx_tmp_guard_mode=post"
  cleanup_new_owned
  cleanup_stale_owned
  inode_snapshot
  assert_inode_headroom
  ;;
*)
  usage
  exit 64
  ;;
esac
