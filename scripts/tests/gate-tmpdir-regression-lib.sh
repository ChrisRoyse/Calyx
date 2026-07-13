#!/usr/bin/env bash
# Shared bounded process helpers for gate TMPDIR regression batteries.

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  echo "gate-tmpdir-regression-lib: ERROR this file must be sourced" >&2
  exit 64
fi

gate_test_group_alive() {
  local pgid="$1"
  [[ "$pgid" =~ ^[1-9][0-9]*$ ]] && kill -0 -- "-$pgid" 2>/dev/null
}

gate_test_wait_group_absent() {
  local pgid="$1" attempts="${2:-40}"
  local i
  for ((i = 0; i < attempts; i++)); do
    gate_test_group_alive "$pgid" || return 0
    sleep 0.05
  done
  return 1
}

gate_test_stop_group_bounded() {
  local pgid="$1"
  gate_test_group_alive "$pgid" || return 0
  kill -TERM -- "-$pgid" 2>/dev/null || true
  gate_test_wait_group_absent "$pgid" 10 && return 0
  kill -KILL -- "-$pgid" 2>/dev/null || true
  gate_test_wait_group_absent "$pgid" 40
}

gate_test_wait_ready() {
  local path="$1" attempts="${2:-100}"
  local i
  for ((i = 0; i < attempts; i++)); do
    [[ -s "$path" ]] && return 0
    sleep 0.05
  done
  return 1
}

gate_test_pid_starttime() {
  local output_var="$1" pid="$2" raw tail value
  local -a fields=()
  IFS= read -r raw 2>/dev/null <"/proc/$pid/stat" || return 1
  tail="${raw##*) }"
  [[ "$tail" != "$raw" ]] || return 1
  read -r -a fields <<<"$tail"
  value="${fields[19]:-}"
  [[ "$value" =~ ^[1-9][0-9]*$ ]] || return 1
  printf -v "$output_var" '%s' "$value"
}

gate_test_pid_identity_gone() {
  local pid="$1" expected="$2" actual=""
  [[ "$pid" =~ ^[1-9][0-9]*$ && "$expected" =~ ^[1-9][0-9]*$ ]] || return 1
  gate_test_pid_starttime actual "$pid" || return 0
  [[ "$actual" != "$expected" ]]
}

gate_test_signal_exact_bounded() {
  local pid="$1" expected="$2" signal_name="$3" wait_ms="$4"
  [[ "$pid" =~ ^[1-9][0-9]*$ && "$expected" =~ ^[1-9][0-9]*$ \
    && "$signal_name" =~ ^(HUP|INT|TERM|KILL|STOP)$ && "$wait_ms" =~ ^[0-9]+$ ]] || return 1
  python3 - "$pid" "$expected" "$signal_name" "$wait_ms" <<'PY'
import os
import select
import signal
import sys

pid = int(sys.argv[1])
expected = sys.argv[2].encode("ascii")
requested = getattr(signal, f"SIG{sys.argv[3]}")
wait_ms = int(sys.argv[4])
try:
    pidfd = os.pidfd_open(pid)
except ProcessLookupError:
    raise SystemExit(0)
try:
    try:
        raw = open(f"/proc/{pid}/stat", "rb").read(8192)
    except FileNotFoundError:
        raise SystemExit(0)
    end = raw.rfind(b") ")
    fields = raw[end + 2 :].split() if end >= 0 else []
    if len(fields) <= 19 or fields[19] != expected:
        raise SystemExit(3)
    signal.pidfd_send_signal(pidfd, requested)
    if wait_ms == 0:
        raise SystemExit(0)
    poller = select.poll()
    poller.register(pidfd, select.POLLIN)
    raise SystemExit(0 if poller.poll(wait_ms) else 125)
finally:
    os.close(pidfd)
PY
}

gate_test_kill_exact_bounded() {
  gate_test_signal_exact_bounded "$1" "$2" KILL 2000
}

gate_test_cgroup_absent() {
  local control="$1" identity="$2"
  [[ "$control" == /*/calyxgate-*.scope && "$identity" =~ ^[0-9]+:[0-9]+$ \
    && ! -e "/sys/fs/cgroup$control" ]]
}

gate_test_log_groups_owned() {
  local log="$1" expected_target="$2" line stage tmp target pid pgid cgroup cgroup_id
  local control parent expected_parent="" count=0
  local -A seen_controls=()
  while IFS= read -r line; do
    read -r stage tmp target pid pgid cgroup cgroup_id <<<"$line"
    [[ "$stage" == stage=* && "$tmp" == tmp=* && "$target" == target=* \
      && "$pid" == pid=* && "$pgid" == pgid=* && "$cgroup" == cgroup=* \
      && "$cgroup_id" == cgroup_id=* ]] || return 1
    [[ -n "${target#target=}" && "${target#target=}" == "$expected_target" ]] || return 1
    control="${cgroup#cgroup=}"
    parent="${control%/*}"
    [[ "${pid#pid=}" == "${pgid#pgid=}" \
      && "$control" == */calyxgate-*.scope \
      && "${cgroup_id#cgroup_id=}" =~ ^[0-9]+:[0-9]+$ \
      && ! -e "/sys/fs/cgroup$control" \
      && -z "${seen_controls[$control]+present}" ]] || return 1
    [[ -z "$expected_parent" || "$parent" == "$expected_parent" ]] || return 1
    expected_parent="$parent"
    seen_controls["$control"]=1
    count=$((count + 1))
  done <"$log"
  [[ "$count" -gt 0 ]]
}

gate_test_exact_stage_manifest() {
  local log="$1"
  shift
  local actual expected="" stage
  actual="$(sed -n 's/^stage=\([^ ]*\).*/\1/p' "$log")"
  for stage in "$@"; do
    expected+="${expected:+$'\n'}$stage"
  done
  [[ "$actual" == "$expected" ]]
}

gate_test_cleanup_readback_bounded() {
  local log="$1" all valid
  all="$(grep -Ec '^calyx_check_tmpdir_cleanup_before ' "$log" 2>/dev/null || true)"
  valid="$(grep -Ec '^calyx_check_tmpdir_cleanup_before path=[^ ]+ entries_sampled=[01] dir_blocks=[0-9]+$' "$log" 2>/dev/null || true)"
  [[ "$all" -eq 1 && "$valid" -eq 1 ]]
}

gate_test_tmp_guard_readbacks_bounded() {
  local log="$1" expected="$2" inode_all inode_valid block_all block_valid
  inode_all="$(grep -Ec '^calyx_tmp_guard_inodes ' "$log" 2>/dev/null || true)"
  inode_valid="$(grep -Ec '^calyx_tmp_guard_inodes total=([0-9]+|unavailable) used=([0-9]+|unavailable) free=([0-9]+|unavailable) used_percent=([0-9]+|unavailable)$' "$log" 2>/dev/null || true)"
  block_all="$(grep -Ec '^calyx_tmp_guard_blocks ' "$log" 2>/dev/null || true)"
  block_valid="$(grep -Ec '^calyx_tmp_guard_blocks total=([0-9]+|unavailable) used=([0-9]+|unavailable) free=([0-9]+|unavailable) used_percent=([0-9]+|unavailable)$' "$log" 2>/dev/null || true)"
  [[ "$inode_all" -eq "$expected" && "$inode_valid" -eq "$expected" \
    && "$block_all" -eq "$expected" && "$block_valid" -eq "$expected" ]]
}
