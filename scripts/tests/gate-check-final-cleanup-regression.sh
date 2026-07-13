#!/usr/bin/env bash
# Source-only final target-clean failure propagation for check.sh.
[[ "${BASH_SOURCE[0]}" != "$0" ]] || {
  echo "source this module from gate-final-contracts-regression.sh" >&2
  exit 64
}

echo
echo "=================================================================="
echo "CHECK FINAL CLEANUP FAILURE - rc 73 overrides a green workload"
echo "=================================================================="
check_final_repo="$work/check-final/calyx/repo"
mkdir -p "$check_final_repo"
setup_repo "$check_final_repo"
cat >"$check_final_repo/scripts/cargo-target-clean.sh" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
target="" expected="" lock_fd=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --target-dir) target="$2"; shift 2 ;;
    --expected-target-identity) expected="$2"; shift 2 ;;
    --gate-lock-fd) lock_fd="$2"; shift 2 ;;
    --repo-root) shift 2 ;;
    --also-workspace-target | --also-repo-targets | --also-calyx-home-target) shift ;;
    *) exit 96 ;;
  esac
done
[[ -n "$target" && -n "$expected" && "$lock_fd" =~ ^[0-9]+$ \
  && -e "/proc/$BASHPID/fd/$lock_fd" && -n "${CALYX_GATE_LOCK_PATH:-}" ]] || exit 97
[[ "$(readlink -f -- "/proc/$BASHPID/fd/$lock_fd")" == "$CALYX_GATE_LOCK_PATH" \
  && "$(stat -c '%d:%i' -- "$target")" == "$expected" ]] || exit 98
flock --fcntl --exclusive --nonblock "$lock_fd" || exit 99
printf 'target=%s identity=%s use_lines=%s\n' "$target" "$expected" \
  "$(wc -l <"$target/.gate-stage-use")" >"${CHECK_FINAL_FAIL_MARKER:?}"
exit 73
STUB
chmod +x "$check_final_repo/scripts/cargo-target-clean.sh"
check_final_log="$work/check-final/gate.log"
check_final_stages="$work/check-final/stages.log"
check_final_marker="$work/check-final/cleaner-failed"
export CHECK_FINAL_FAIL_MARKER="$check_final_marker"
run_gate "$check_final_repo" "$check_final_log" "$check_final_stages"
unset CHECK_FINAL_FAIL_MARKER
check_final_tmp="$(gate_tmp_from_log "$check_final_log")"
check_final_target="$(gate_target_from_log "$check_final_log")"
check_final_slice="$(sed -n 's/.*run_slice_released unit=\([^ ]*\).*/\1/p' "$check_final_log" | tail -n1)"
check_final_stage_control="$(sed -n 's/.* cgroup=\([^ ]*\) command=.*/\1/p' "$check_final_log" | head -n1)"
check_final_slice_control="${check_final_stage_control%/*}"
check_final_lock="$(dirname "$check_final_repo")/tmp/.cargo-gate.lock"
exec {check_final_lock_fd}>>"$check_final_lock"
set +e
flock --fcntl --exclusive --nonblock "$check_final_lock_fd"
check_final_lock_rc=$?
set -e
eval "exec ${check_final_lock_fd}>&-"
[[ "$RUN_RC" -eq 125 && -s "$check_final_marker" \
  && "$(wc -l <"$check_final_stages" | tr -d ' ')" -eq 15 \
  && -n "$check_final_tmp" && ! -e "$check_final_tmp" \
  && "$check_final_target" == "$(dirname "$check_final_repo")/target-scratch/"check.* \
  && -d "$check_final_target" && "$(wc -l <"$check_final_target/.gate-stage-use")" -eq 15 \
  && "$check_final_lock_rc" -eq 0 && "$check_final_slice" =~ ^calyxgate-u[0-9]+-r[0-9]+\.slice$ \
  && "$check_final_stage_control" == "$check_final_slice_control"/calyxgate-*.scope \
  && ! -e "/sys/fs/cgroup$check_final_slice_control" ]] \
  && grep -Fq 'CALYX_CHECK_TARGET_CLEANUP_FAILED: status=73' "$check_final_log" \
  && grep -Fq 'cleanup_summary status=0 cleanup_status=73' "$check_final_log" \
  && pass "check green workload becomes 125, removes TMPDIR, and retains exact recovery target" \
  || {
    bad "check final cleanup rc=$RUN_RC hid failure or lost recovery state"
    printf '    final-check: marker=%s stages=%s tmp=%q tmp_exists=%s target=%q target_exists=%s use_lines=%s lock_rc=%s slice=%q control=%q control_exists=%s\n' \
      "$(test -s "$check_final_marker" && echo yes || echo no)" \
      "$(wc -l <"$check_final_stages" 2>/dev/null || echo missing)" "$check_final_tmp" \
      "$(test -e "$check_final_tmp" && echo yes || echo no)" "$check_final_target" \
      "$(test -d "$check_final_target" && echo yes || echo no)" \
      "$(wc -l <"$check_final_target/.gate-stage-use" 2>/dev/null || echo missing)" \
      "$check_final_lock_rc" "$check_final_slice" "$check_final_slice_control" \
      "$(test -e "/sys/fs/cgroup$check_final_slice_control" && echo yes || echo no)"
    tail -20 "$check_final_log" | sed 's/^/    final-check-log: /'
  }
case "$check_final_target" in
  "$(dirname "$check_final_repo")/target-scratch"/check.*) rm -rf -- "$check_final_target" ;;
  *) bad "refusing unexpected check final-clean recovery path: $check_final_target" ;;
esac
