#!/usr/bin/env bash
# Source-only signal-during-global-flock regression for issue #1421.
[[ "${BASH_SOURCE[0]}" != "$0" ]] || {
  echo "source this module from check-tmpdir-regression.sh" >&2
  exit 64
}

echo
echo "=================================================================="
echo "LOCK WAIT - PID-only TERM cleans the queued gate without a target"
echo "=================================================================="

lock_wait_path="$(dirname "$repo6")/tmp/.cargo-gate.lock"
exec {lock_wait_holder_fd}>>"$lock_wait_path"
flock --fcntl --exclusive "$lock_wait_holder_fd"
lock_wait_log="$work/lock-wait/gate.log"
lock_wait_stages="$work/lock-wait/stages.log"
lock_wait_ready="$work/lock-wait/flock-ready"
lock_wait_real_flock="$(command -v flock)"
mkdir -p "$work/lock-wait"
cat >"$repo6/fakebin/flock" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
[[ " $* " != *" --help "* ]] || exec "${LOCK_WAIT_REAL_FLOCK:?}" "$@"
[[ -z "${LOCK_WAIT_READY:-}" ]] || printf 'pid=%s pgid=%s\n' \
  "$BASHPID" "$(ps -o pgid= -p "$BASHPID" | tr -d ' ')" >"$LOCK_WAIT_READY"
exec "${LOCK_WAIT_REAL_FLOCK:?}" "$@"
STUB
chmod +x "$repo6/fakebin/flock"
setsid env --default-signal=HUP,INT,TERM -u CALYX_FSV_ROOT -u CALYX_GATE_RUN_SLICE -u CALYX_GATE_RUN_SLICE_OWNER_PID \
  HOME="$repo6/home" PATH="$repo6/fakebin:$PATH" \
  CALYX_TMP_ROOT="$repo6/global-tmp" CALYX_TMP_MIN_FREE_INODE_PERCENT=0 \
  CALYX_TMP_MIN_FREE_BLOCK_PERCENT=0 \
  CALYX_TARGET_SCRATCH_ROOT="$(dirname "$repo6")/target-scratch" \
  CALYX_GATE_SIGNAL_GRACE_SECONDS=0.5 \
  FAKE_TMP_LOG="$lock_wait_stages" FAKE_REQUIRE_OWNED_MODE=1 \
  FAKE_FAIL_STAGE= FAKE_DELETE_BASELINE=0 FAKE_BLOCK_STAGE= FAKE_BLOCK_SCOPE=gate \
  FAKE_READY_FILE= FAKE_STUBBORN_CHILD=0 FAKE_LEAK_DESCENDANT=0 FAKE_DELAY_SECONDS= \
  LOCK_WAIT_READY="$lock_wait_ready" LOCK_WAIT_REAL_FLOCK="$lock_wait_real_flock" \
  bash "$repo6/scripts/check.sh" >"$lock_wait_log" 2>&1 &
lock_wait_pid=$!
owned_extra_pgids="$lock_wait_pid"
gate_test_wait_ready "$lock_wait_ready" || true
lock_wait_tmp="$(gate_created_tmp_from_log "$lock_wait_log")"
lock_wait_target="$(gate_target_from_log "$lock_wait_log")"
lock_wait_flock_pid="$(sed -n 's/^pid=\([0-9]*\).*/\1/p' "$lock_wait_ready" 2>/dev/null | head -n1 || true)"
exec {lock_wait_preprobe_fd}>>"$lock_wait_path"
set +e
flock --fcntl --exclusive --nonblock "$lock_wait_preprobe_fd"
lock_wait_preprobe_rc=$?
set -e
eval "exec ${lock_wait_preprobe_fd}>&-"
sleep 0.1
[[ -s "$lock_wait_ready" && -d "$lock_wait_tmp" && -z "$lock_wait_target" \
  && "$lock_wait_flock_pid" =~ ^[1-9][0-9]*$ && -e "/proc/$lock_wait_flock_pid" \
  && "$lock_wait_preprobe_rc" -ne 0 \
  && -z "$(find "$(dirname "$repo6")/target-scratch" -mindepth 1 -print -quit)" ]] \
  && pass "queued gate remains blocked on the real held flock with only its TMPDIR" \
  || bad "queued gate published a target or never reached the held lock"

kill -TERM "$lock_wait_pid"
lock_wait_watchdog_marker="$work/lock-wait/watchdog-fired"
(
  sleep 5
  printf 'fired\n' >"$lock_wait_watchdog_marker"
  kill -TERM -- "-$lock_wait_pid" 2>/dev/null || true
  sleep 0.5
  kill -KILL -- "-$lock_wait_pid" 2>/dev/null || true
) &
lock_wait_watchdog=$!
set +e
wait "$lock_wait_pid"
lock_wait_rc=$?
set -e
kill -KILL "$lock_wait_watchdog" 2>/dev/null || true
wait "$lock_wait_watchdog" 2>/dev/null || true
owned_extra_pgids=""
exec {lock_wait_probe_fd}>>"$lock_wait_path"
set +e
flock --fcntl --exclusive --nonblock "$lock_wait_probe_fd"
lock_wait_probe_rc=$?
set -e
eval "exec ${lock_wait_probe_fd}>&-"
[[ "$lock_wait_rc" -eq 143 && "$lock_wait_probe_rc" -ne 0 \
  && ! -e "$lock_wait_watchdog_marker" \
  && -n "$lock_wait_tmp" && ! -e "$lock_wait_tmp" \
  && -z "$(find "$(dirname "$repo6")/target-scratch" -mindepth 1 -print -quit)" ]] \
  && ! gate_test_group_alive "$lock_wait_pid" \
  && pass "PID-only TERM removes queued TMPDIR while preserving the holder lock" \
  || bad "lock-wait TERM rc=$lock_wait_rc probe=$lock_wait_probe_rc tmp=$lock_wait_tmp leaked, hung, or released another owner's lock"
eval "exec ${lock_wait_holder_fd}>&-"
rm -- "$repo6/fakebin/flock"
