#!/usr/bin/env bash
# Source-only canonical-lock convergence test for check/fuzz/public wrappers.
[[ "${BASH_SOURCE[0]}" != "$0" ]] || {
  echo "source this module from gate-wrapper-tmpdir-regression.sh" >&2
  exit 64
}

echo
echo "=================================================================="
echo "CROSS-WRAPPER LOCK - check, fuzz, and public converge on CALYX_HOME"
echo "=================================================================="

cross_home="$work/cross-wrapper/calyx"
cross_check="$cross_home/repo"
cross_fuzz="$cross_home/fuzz-repo"
cross_dev="$cross_home/dev"
cross_public="$cross_home/public-mirror"
mkdir -p "$cross_check/scripts" "$cross_check/home/.cargo" \
  "$cross_fuzz/scripts" "$cross_fuzz/fuzz" "$cross_fuzz/home/.cargo"
chmod 755 "$cross_home" "$cross_check" "$cross_fuzz"
for root in "$cross_check" "$cross_fuzz"; do
  cp "$helper_src" "$root/scripts/check-tmpdir.sh"
  cp "$runner_src" "$root/scripts/gate-stage-runner.sh"
  cp "$runtime_src" "$root/scripts/gate-runtime-state.sh"
  cp "$capture_src" "$root/scripts/gate-identity-capture.sh"
  cp "$safe_helper_src" "$creator_src" "$record_exec_src" "$root/scripts/"
  cp "$cgroup_src" "$root/scripts/gate-cgroup-runtime.sh"
  cp "$cgroup_scope_src" "$root/scripts/gate-cgroup-scope.sh"
  cp "$ofd_query_src" "$root/scripts/gate_ofd_lock_query.py"
  : >"$root/home/.cargo/env"
  printf 'export CALYX_HOME=%q\n' "$cross_home" >"$root/env.sh"
done
cp "$repo_scripts/check.sh" "$cross_check/scripts/check.sh"
cp "$guard_src" "$cross_check/scripts/tmp_scratch_guard.sh"
cp "$fuzz_src" "$cross_fuzz/scripts/fuzz-gate.sh"
write_fake_cargo "$cross_fuzz"
printf '[workspace]\nmembers=[]\n' >"$cross_fuzz/fuzz/Cargo.toml"
setup_public_fixture "$cross_dev" "$cross_public"

cross_lock="$cross_home/tmp/.cargo-gate.lock"
mkdir -m 700 "$cross_home/tmp"
(umask 077; : >"$cross_lock")
exec {cross_holder_fd}>>"$cross_lock"
flock --fcntl --exclusive "$cross_holder_fd"
cross_real_flock="$(command -v flock)"
cat >"$cross_fuzz/fakebin/flock" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
[[ " $* " != *" --help "* ]] || exec "${CROSS_REAL_FLOCK:?}" "$@"
[[ -z "${CROSS_FLOCK_READY:-}" ]] || printf 'pid=%s pgid=%s\n' \
  "$BASHPID" "$(ps -o pgid= -p "$BASHPID" | tr -d ' ')" >"$CROSS_FLOCK_READY"
exec "${CROSS_REAL_FLOCK:?}" "$@"
STUB
chmod +x "$cross_fuzz/fakebin/flock"
cp "$cross_fuzz/fakebin/flock" "$cross_dev/fakebin/flock"
cross_check_ready="$work/cross-wrapper/check-flock-ready"
cross_fuzz_ready="$work/cross-wrapper/fuzz-flock-ready"
cross_public_ready="$work/cross-wrapper/public-flock-ready"

cross_check_log="$work/cross-wrapper/check.log"
cross_fuzz_log="$work/cross-wrapper/fuzz.log"
cross_public_log="$work/cross-wrapper/public.log"
setsid env --default-signal=HUP,INT,TERM -u CALYX_FSV_ROOT -u CALYX_GATE_RUN_SLICE -u CALYX_GATE_RUN_SLICE_OWNER_PID \
  -u CALYX_GATE_RUN_SLICE_OWNER_STARTTIME \
  HOME="$cross_check/home" PATH="$cross_fuzz/fakebin:$PATH" \
  CALYX_TMP_ROOT="$cross_check/global-tmp" CALYX_TMP_MIN_FREE_INODE_PERCENT=0 \
  CALYX_TMP_MIN_FREE_BLOCK_PERCENT=0 \
  CROSS_FLOCK_READY="$cross_check_ready" CROSS_REAL_FLOCK="$cross_real_flock" \
  bash "$cross_check/scripts/check.sh" >"$cross_check_log" 2>&1 &
cross_check_pid=$!
setsid env --default-signal=HUP,INT,TERM -u CALYX_FSV_ROOT -u CALYX_GATE_RUN_SLICE -u CALYX_GATE_RUN_SLICE_OWNER_PID \
  -u CALYX_GATE_RUN_SLICE_OWNER_STARTTIME \
  -u CALYX_GATE_TMPDIR -u CALYX_GATE_TMP_ROOT -u CARGO_TARGET_DIR -u TMPDIR \
  HOME="$cross_fuzz/home" PATH="$cross_fuzz/fakebin:$PATH" \
  CROSS_FLOCK_READY="$cross_fuzz_ready" CROSS_REAL_FLOCK="$cross_real_flock" \
  bash "$cross_fuzz/scripts/fuzz-gate.sh" >"$cross_fuzz_log" 2>&1 &
cross_fuzz_pid=$!
setsid env --default-signal=HUP,INT,TERM -u CALYX_FSV_ROOT -u CALYX_GATE_RUN_SLICE -u CALYX_GATE_RUN_SLICE_OWNER_PID \
  -u CALYX_GATE_RUN_SLICE_OWNER_STARTTIME \
  HOME="$cross_dev/home" PATH="$cross_dev/fakebin:$PATH" \
  CALYX_TMP_ROOT="$cross_dev/global-tmp" CALYX_TMP_MIN_FREE_INODE_PERCENT=0 \
  CALYX_TMP_MIN_FREE_BLOCK_PERCENT=0 \
  CROSS_FLOCK_READY="$cross_public_ready" CROSS_REAL_FLOCK="$cross_real_flock" \
  bash "$cross_dev/scripts/public-mirror-gate.sh" --public-dir "$cross_public" \
  >"$cross_public_log" 2>&1 &
cross_public_pid=$!
owned_pids="$cross_check_pid $cross_fuzz_pid $cross_public_pid"
owned_pgids="$owned_pids"

gate_test_wait_ready "$cross_check_ready" || true
gate_test_wait_ready "$cross_fuzz_ready" || true
gate_test_wait_ready "$cross_public_ready" || true
cross_check_tmp="$(sed -n 's/.*calyx_check_tmpdir_created root=[^ ]* run=\([^ ]*\).*/\1/p' "$cross_check_log" | head -n1)"
cross_fuzz_tmp="$(sed -n 's/.*calyx_check_tmpdir_created root=[^ ]* run=\([^ ]*\).*/\1/p' "$cross_fuzz_log" | head -n1)"
cross_public_tmp="$(sed -n 's/.*calyx_check_tmpdir_created root=[^ ]* run=\([^ ]*\).*/\1/p' "$cross_public_log" | head -n1)"
cross_unique_tmp_count="$(printf '%s\n' "$cross_check_tmp" "$cross_fuzz_tmp" "$cross_public_tmp" | sort -u | wc -l | tr -d ' ')"
cross_check_flock_pid="$(sed -n 's/^pid=\([0-9]*\).*/\1/p' "$cross_check_ready" 2>/dev/null | head -n1 || true)"
cross_fuzz_flock_pid="$(sed -n 's/^pid=\([0-9]*\).*/\1/p' "$cross_fuzz_ready" 2>/dev/null | head -n1 || true)"
cross_public_flock_pid="$(sed -n 's/^pid=\([0-9]*\).*/\1/p' "$cross_public_ready" 2>/dev/null | head -n1 || true)"
sleep 0.1
[[ "$cross_unique_tmp_count" -eq 3 && -d "$cross_check_tmp" \
  && -d "$cross_fuzz_tmp" && -d "$cross_public_tmp" \
  && "$cross_check_flock_pid" =~ ^[1-9][0-9]*$ \
  && "$cross_fuzz_flock_pid" =~ ^[1-9][0-9]*$ \
  && "$cross_public_flock_pid" =~ ^[1-9][0-9]*$ \
  && -e "/proc/$cross_check_flock_pid" && -e "/proc/$cross_fuzz_flock_pid" \
  && -e "/proc/$cross_public_flock_pid" \
  && -z "$(find "$cross_home/target-scratch" -mindepth 1 -print -quit 2>/dev/null)" ]] \
  && pass "all three wrappers queue at one lock with distinct live TMPDIRs and no target" \
  || bad "cross-wrapper lock state did not converge before target publication"

kill -TERM "$cross_check_pid" "$cross_fuzz_pid" "$cross_public_pid"
cross_watchdog_marker="$work/cross-wrapper/watchdog-fired"
(
  sleep 8
  printf 'fired\n' >"$cross_watchdog_marker"
  kill -TERM -- "-$cross_check_pid" "-$cross_fuzz_pid" "-$cross_public_pid" 2>/dev/null || true
  sleep 0.5
  kill -KILL -- "-$cross_check_pid" "-$cross_fuzz_pid" "-$cross_public_pid" 2>/dev/null || true
) &
cross_watchdog=$!
set +e
wait "$cross_check_pid"; cross_check_rc=$?
wait "$cross_fuzz_pid"; cross_fuzz_rc=$?
wait "$cross_public_pid"; cross_public_rc=$?
set -e
kill -KILL "$cross_watchdog" 2>/dev/null || true
wait "$cross_watchdog" 2>/dev/null || true
owned_pids="" owned_pgids=""
exec {cross_probe_fd}>>"$cross_lock"
set +e
flock --fcntl --exclusive --nonblock "$cross_probe_fd"
cross_probe_rc=$?
set -e
eval "exec ${cross_probe_fd}>&-"
[[ "$cross_check_rc" -eq 143 && "$cross_fuzz_rc" -eq 143 && "$cross_public_rc" -eq 143 \
  && "$cross_probe_rc" -ne 0 && ! -e "$cross_watchdog_marker" && ! -e "$cross_check_tmp" \
  && ! -e "$cross_fuzz_tmp" && ! -e "$cross_public_tmp" \
  && -z "$(find "$cross_home/target-scratch" -mindepth 1 -print -quit 2>/dev/null)" ]] \
  && pass "PID-only TERM cleans all queued wrappers without releasing the holder lock" \
  || bad "cross-wrapper queued TERM leaked scratch or lock ownership"
eval "exec ${cross_holder_fd}>&-"
rm -- "$cross_fuzz/fakebin/flock" "$cross_dev/fakebin/flock"
