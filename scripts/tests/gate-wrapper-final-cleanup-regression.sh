#!/usr/bin/env bash
# Source-only final target-clean failure propagation for the fuzz wrapper.
[[ "${BASH_SOURCE[0]}" != "$0" ]] || {
  echo "source this module from gate-wrapper-tmpdir-regression.sh" >&2
  exit 64
}

echo
echo "=================================================================="
echo "FINAL CLEANUP FAILURE - containment rc overrides a green fuzz stage"
echo "=================================================================="
final_clean_log="$work/fuzz/final-clean-failure.log"
final_clean_stages="$work/fuzz/final-clean-failure-stages.log"
final_clean_once="$work/fuzz/final-clean-failed-once"
set +e
setsid env --default-signal=HUP,INT,TERM -u CALYX_FSV_ROOT -u CALYX_GATE_RUN_SLICE \
  -u CALYX_GATE_RUN_SLICE_OWNER_PID -u CALYX_GATE_RUN_SLICE_OWNER_STARTTIME \
  -u CALYX_GATE_TMPDIR -u CALYX_GATE_TMP_ROOT -u CARGO_TARGET_DIR -u TMPDIR \
  HOME="$fuzz_repo/home" PATH="$fuzz_repo/fakebin:$PATH" \
  FAKE_TMP_LOG="$final_clean_stages" FAKE_DELETE_BASELINE=0 FAKE_FAIL_STAGE= \
  FAKE_BLOCK_STAGE= FAKE_READY_FILE= FAKE_REQUIRE_OWNED_MODE=1 \
  FAKE_TARGET_CLEAN_FAIL_ONCE_FILE="$final_clean_once" \
  timeout --signal=TERM --kill-after=2s 20s bash "$fuzz_repo/scripts/fuzz-gate.sh" \
  >"$final_clean_log" 2>&1
final_clean_rc=$?
set -e
final_clean_tmp="$(sed -n 's/.*calyx_check_tmpdir_created root=[^ ]* run=\([^ ]*\).*/\1/p' "$final_clean_log" | head -n1)"
final_clean_target="$(sed -n 's/.*calyx_gate_target_created root=[^ ]* target=\([^ ]*\).*/\1/p' "$final_clean_log" | head -n1)"
[[ "$final_clean_rc" -eq 125 && -n "$final_clean_tmp" && ! -e "$final_clean_tmp" \
  && -n "$final_clean_target" && -d "$final_clean_target" && -e "$final_clean_once" \
  && "$(wc -l <"$final_clean_stages" | tr -d ' ')" -eq 3 \
  && "$(wc -l <"$final_clean_target/.gate-stage-use")" -eq 3 ]] \
  && gate_test_exact_stage_manifest "$final_clean_stages" cargo:fmt cargo:check cargo:clippy \
  && grep -Fq 'target cleanup failed: status=73' "$final_clean_log" \
  && pass "green fuzz workload becomes 125, cleans TMPDIR, and preserves recovery target" \
  || bad "fuzz final cleanup failure rc=$final_clean_rc hid failure or lost recovery state"
case "$final_clean_target" in
  "$(dirname "$fuzz_repo")/target-scratch"/fuzz-gate.*) rm -rf -- "$final_clean_target" ;;
  *) bad "refusing unexpected final-clean recovery path: $final_clean_target" ;;
esac
final_clean_remaining="$(find "$(dirname "$fuzz_repo")/target-scratch" -mindepth 1 -maxdepth 1 -print)"
[[ "$final_clean_remaining" == "$outer_target_root/outer-target" \
  && "$(sha256sum "$outer_target_root/outer-target/sentinel" | awk '{print $1}')" == "$outer_target_sha" ]] \
  && pass "explicit recovery removes only its target and preserves the caller-owned target" \
  || bad "fuzz final-clean recovery changed unrelated scratch state: $final_clean_remaining"

echo
echo "=================================================================="
echo "PUBLIC FINAL CLEANUP FAILURE - rc 73 overrides a green workload"
echo "=================================================================="
public_final_log="$work/public/final-clean-failure.log"
public_final_stages="$work/public/final-clean-failure-stages.log"
public_final_once="$work/public/final-clean-failed-once"
run_public_gate "$public_dev" "$public_repo" "$public_final_log" \
  "$public_final_stages" 0 "" "" "" "" "$public_final_once"
public_final_tmp="$(sed -n 's/^public-mirror-gate: TMPDIR=//p' "$public_final_log" | head -n1)"
public_final_target="$(sed -n 's/^public-mirror-gate: target_dir=//p' "$public_final_log" | head -n1)"
public_final_slice="$(sed -n 's/.*run_slice_released unit=\([^ ]*\).*/\1/p' "$public_final_log" | tail -n1)"
public_final_stage_control="$(sed -n 's/.* cgroup=\([^ ]*\) command=.*/\1/p' "$public_final_log" | head -n1)"
public_final_slice_control="${public_final_stage_control%/*}"
public_final_lock="$(dirname "$public_dev")/tmp/.cargo-gate.lock"
exec {public_final_lock_fd}>>"$public_final_lock"
set +e
flock --fcntl --exclusive --nonblock "$public_final_lock_fd"
public_final_lock_rc=$?
set -e
eval "exec ${public_final_lock_fd}>&-"
[[ "$PUBLIC_RC" -eq 125 && -e "$public_final_once" \
  && "$(wc -l <"$public_final_stages" | tr -d ' ')" -eq 12 \
  && -n "$public_final_tmp" && ! -e "$public_final_tmp" \
  && "$public_final_target" == "$(dirname "$public_dev")/target-scratch/"public-mirror-gate.* \
  && -d "$public_final_target" && "$(wc -l <"$public_final_target/.gate-stage-use")" -eq 12 \
  && "$public_final_lock_rc" -eq 0 && "$public_final_slice" =~ ^calyxgate-u[0-9]+-r[0-9]+\.slice$ \
  && "$public_final_stage_control" == "$public_final_slice_control"/calyxgate-*.scope \
  && ! -e "/sys/fs/cgroup$public_final_slice_control" \
  && -z "$(git -C "$public_repo" status --porcelain)" ]] \
  && grep -Fq 'public-mirror-gate: ERROR target cleanup failed: status=73' "$public_final_log" \
  && pass "public green workload becomes 125, removes TMPDIR, releases containment, and retains recovery target" \
  || bad "public final cleanup rc=$PUBLIC_RC hid failure or lost recovery state"
case "$public_final_target" in
  "$(dirname "$public_dev")/target-scratch"/public-mirror-gate.*) rm -rf -- "$public_final_target" ;;
  *) bad "refusing unexpected public final-clean recovery path: $public_final_target" ;;
esac
[[ -z "$(find "$(dirname "$public_dev")/target-scratch" -mindepth 1 -print -quit)" ]] \
  && pass "public final-clean explicit recovery leaves target-scratch empty" \
  || bad "public final-clean recovery left target-scratch state"
