#!/usr/bin/env bash
# Regression coverage for the per-gate TMPDIR lifecycle (issue #1421).
set -euo pipefail
unset CALYX_GATE_RUN_SLICE CALYX_GATE_RUN_SLICE_OWNER_PID CALYX_GATE_RUN_SLICE_OWNER_STARTTIME
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
repo_scripts="$(cd "$script_dir/.." && pwd -P)"
check_src="$repo_scripts/check.sh"
tmpdir_src="$repo_scripts/check-tmpdir.sh"
guard_src="$repo_scripts/tmp_scratch_guard.sh"
runner_src="$repo_scripts/gate-stage-runner.sh"
runtime_src="$repo_scripts/gate-runtime-state.sh"
capture_src="$repo_scripts/gate-identity-capture.sh" record_exec_src="$repo_scripts/gate_identity_record_exec.py"
cgroup_src="$repo_scripts/gate-cgroup-runtime.sh"
cgroup_scope_src="$repo_scripts/gate-cgroup-scope.sh"
ofd_query_src="$repo_scripts/gate_ofd_lock_query.py"
safe_helper_src="$repo_scripts/gate_safe_rmtree.py"
creator_src="$repo_scripts/gate_owned_dir_create.py"
safe_helper_regression="$script_dir/gate-safe-rmtree-regression.py"
fuzz_src="$repo_scripts/fuzz-gate.sh"
target_clean_src="$repo_scripts/cargo-target-clean.sh"
test_lib="$script_dir/gate-tmpdir-regression-lib.sh"
process_scenarios="$script_dir/check-tmpdir-process-regression.sh"
boundary_scenarios="$script_dir/check-tmpdir-boundary-regression.sh"
runner_contracts="$script_dir/gate-runner-contract-regression.sh"
runner_handoff_contracts="$script_dir/gate-runner-handoff-regression.sh"
target_clean_contracts="$script_dir/cargo-target-clean-regression.sh"
lock_wait_scenario="$script_dir/gate-lock-wait-regression.sh"
setup_publication_scenario="$script_dir/check-setup-publication-regression.sh" launch_handshake_contracts="$script_dir/gate-launch-handshake-regression.sh"
final_contracts="$script_dir/gate-final-contracts-regression.sh" check_final_cleanup_contracts="$script_dir/gate-check-final-cleanup-regression.sh" cleanup_prediagnostic_contracts="$script_dir/check-cleanup-prediagnostic-regression.sh"
for required in "$check_src" "$tmpdir_src" "$guard_src" "$runner_src" "$fuzz_src" \
  "$target_clean_src" "$runtime_src" "$capture_src" "$record_exec_src" "$cgroup_src" "$cgroup_scope_src" "$ofd_query_src" \
  "$safe_helper_src" "$creator_src" "$safe_helper_regression" "$test_lib" "$process_scenarios" "$boundary_scenarios" \
  "$runner_contracts" "$runner_handoff_contracts" "$target_clean_contracts" "$lock_wait_scenario" \
  "$setup_publication_scenario" "$final_contracts" "$launch_handshake_contracts" "$check_final_cleanup_contracts" \
  "$cleanup_prediagnostic_contracts"; do
  [[ -f "$required" ]] || { echo "REGRESSION SETUP ERROR: missing $required" >&2; exit 2; }
done
# shellcheck source=gate-tmpdir-regression-lib.sh
source "$test_lib"
for required_command in setsid flock timeout python3 systemd-run systemctl busctl findmnt; do
  command -v "$required_command" >/dev/null 2>&1 || {
    echo "REGRESSION SETUP ERROR: $required_command is required" >&2; exit 2;
  }
done

work="$(mktemp -d)"
owned_pgid=""
owned_gate_pid=""
owned_stage_pgid=""
owned_extra_pgids=""
cleanup_test() {
  local status=$?
  trap - EXIT HUP INT TERM
  if [[ -n "$owned_pgid" ]]; then
    gate_test_stop_group_bounded "$owned_pgid" || true
  fi
  if [[ -n "$owned_gate_pid" ]]; then
    kill -TERM "$owned_gate_pid" 2>/dev/null || true
    kill -KILL "$owned_gate_pid" 2>/dev/null || true
  fi
  if [[ -n "$owned_stage_pgid" ]]; then
    gate_test_stop_group_bounded "$owned_stage_pgid" || true
  fi
  local pgid
  for pgid in $owned_extra_pgids; do
    gate_test_stop_group_bounded "$pgid" || true
  done
  rm -rf -- "$work"
  exit "$status"
}
trap cleanup_test EXIT
fail=0

pass() { echo "  PASS: $1"; }
bad()  { echo "  FAIL: $1" >&2; fail=1; }

setup_repo() {
  local repo="$1" calyx_home stub
  calyx_home="${2:-$(dirname "$repo")}"
  mkdir -p "$repo/scripts/tests" "$repo/fakebin" "$repo/home/.cargo" \
    "$repo/global-tmp" "$calyx_home/target-scratch" "$repo/fuzz"
  chmod 755 "$calyx_home" "$repo"
  chmod 700 "$calyx_home/target-scratch"
  cp "$check_src" "$repo/scripts/check.sh"
  cp "$tmpdir_src" "$repo/scripts/check-tmpdir.sh"
  cp "$guard_src" "$repo/scripts/tmp_scratch_guard.sh"
  cp "$runner_src" "$repo/scripts/gate-stage-runner.sh"
  cp "$runtime_src" "$capture_src" "$record_exec_src" "$cgroup_src" "$cgroup_scope_src" "$ofd_query_src" \
    "$safe_helper_src" "$creator_src" "$repo/scripts/"
  cp "$fuzz_src" "$repo/scripts/fuzz-gate.sh"
  chmod +x "$repo/scripts/check.sh" "$repo/scripts/check-tmpdir.sh" \
    "$repo/scripts/tmp_scratch_guard.sh" "$repo/scripts/gate-stage-runner.sh" \
    "$repo/scripts/gate-runtime-state.sh" "$repo/scripts/gate-cgroup-runtime.sh"
  : >"$repo/home/.cargo/env"
  printf 'export CALYX_HOME=%q\n' "$calyx_home" >"$repo/env.sh"
  printf '[workspace]\nmembers=[]\n' >"$repo/fuzz/Cargo.toml"

  cat >"$repo/scripts/stage-stub.sh" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
stage="$(basename -- "$0")"
[[ -d "${TMPDIR:?}" ]] || { echo "STUB_TMPDIR_MISSING: $stage" >&2; exit 90; }
if [[ "${FAKE_REQUIRE_OWNED_MODE:-0}" == "1" ]]; then
  [[ "$(stat -c '%u:%a' -- "$TMPDIR")" == "$(id -u):700" \
    && "$(stat -c '%u:%a' -- "${CARGO_TARGET_DIR:?}")" == "$(id -u):700" ]] || exit 95
fi
pgid="$(ps -o pgid= -p "$$" | tr -d ' ')" cgroup="$(awk -F: '$1=="0"{print $3}' /proc/self/cgroup)" cgroup_id="$(stat -c '%d:%i' -- "/sys/fs/cgroup$cgroup")"
printf 'stage=%s tmp=%s target=%s pid=%s pgid=%s cgroup=%s cgroup_id=%s\n' \
  "$stage" "$TMPDIR" "${CARGO_TARGET_DIR:-}" "$$" "$pgid" "$cgroup" "$cgroup_id" >>"${FAKE_TMP_LOG:?}"
printf '%s\n' "$stage" >>"${CARGO_TARGET_DIR:?}/.gate-stage-use"
printf '%s\n' "$stage" >"$TMPDIR/$stage.marker"
if [[ -n "${FAKE_BARRIER_STAGE:-}" && "$stage" == "$FAKE_BARRIER_STAGE" ]]; then
  printf 'pid=%s pgid=%s stage=%s\n' "$$" "$pgid" "$stage" >"${FAKE_BARRIER_READY:?}"
  while [[ ! -e "${FAKE_BARRIER_RELEASE:?}" ]]; do sleep 0.05; done
fi
STUB
  chmod +x "$repo/scripts/stage-stub.sh"
  for stub in secret-scan.sh cargo-fmt-workspace.sh orphan_rs.sh \
    linecount.sh verify_dataset.sh check_manifest_coverage.sh; do
    cp "$repo/scripts/stage-stub.sh" "$repo/scripts/$stub"
  done
  # If the real gate invokes this regression, the synthetic gate must terminate
  # at the subject boundary rather than recursively running another battery.
  cat >"$repo/scripts/tests/tmpdir-regression-stub.sh" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
stage="$(basename -- "$0")"
pgid="$(ps -o pgid= -p "$$" | tr -d ' ')" cgroup="$(awk -F: '$1=="0"{print $3}' /proc/self/cgroup)" cgroup_id="$(stat -c '%d:%i' -- "/sys/fs/cgroup$cgroup")"
if [[ "${FAKE_REQUIRE_OWNED_MODE:-0}" == "1" ]]; then
  [[ "$(stat -c '%u:%a' -- "${TMPDIR:?}")" == "$(id -u):700" \
    && "$(stat -c '%u:%a' -- "${CARGO_TARGET_DIR:?}")" == "$(id -u):700" ]] || exit 95
fi
printf 'stage=%s tmp=%s target=%s pid=%s pgid=%s cgroup=%s cgroup_id=%s\n' \
  "$stage" "${TMPDIR:?}" "${CARGO_TARGET_DIR:?}" "$$" "$pgid" "$cgroup" "$cgroup_id" >>"${FAKE_TMP_LOG:?}"
printf '%s\n' "$stage" >>"$CARGO_TARGET_DIR/.gate-stage-use"
STUB
  for stub in check-tmpdir-regression.sh gate-wrapper-tmpdir-regression.sh; do
    cp "$repo/scripts/tests/tmpdir-regression-stub.sh" "$repo/scripts/tests/$stub"
  done
  chmod +x "$repo/scripts/tests/"*-tmpdir-regression.sh

  cat >"$repo/scripts/cargo-target-clean.sh" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
target=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --target-dir) target="$2"; shift 2 ;;
    *) shift ;;
  esac
done
[[ -n "$target" ]] || { echo "TARGET_STUB_MISSING_PATH" >&2; exit 91; }
rm -rf -- "$target"
[[ ! -e "$target" ]] || exit 92
echo "cargo-target-clean: target_absent path=$target"
STUB
  chmod +x "$repo/scripts/cargo-target-clean.sh"

  cat >"$repo/fakebin/cargo" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
stage="${1:-none}"
cargo_scope=gate
for arg in "$@"; do
  [[ "$arg" != */fuzz/Cargo.toml ]] || cargo_scope=fuzz
done
[[ -d "${TMPDIR:?}" ]] || { echo "FAKE_CARGO_TMPDIR_MISSING: $stage" >&2; exit 90; }
if [[ "${FAKE_REQUIRE_OWNED_MODE:-0}" == "1" ]]; then
  [[ "$(stat -c '%u:%a' -- "$TMPDIR")" == "$(id -u):700" \
    && "$(stat -c '%u:%a' -- "${CARGO_TARGET_DIR:?}")" == "$(id -u):700" ]] || exit 95
fi
pgid="$(ps -o pgid= -p "$$" | tr -d ' ')" cgroup="$(awk -F: '$1=="0"{print $3}' /proc/self/cgroup)" cgroup_id="$(stat -c '%d:%i' -- "/sys/fs/cgroup$cgroup")"
printf 'stage=cargo:%s tmp=%s target=%s pid=%s pgid=%s cgroup=%s cgroup_id=%s\n' \
  "$stage" "$TMPDIR" "${CARGO_TARGET_DIR:-}" "$$" "$pgid" "$cgroup" "$cgroup_id" >>"${FAKE_TMP_LOG:?}"
printf 'cargo:%s\n' "$stage" >>"${CARGO_TARGET_DIR:?}/.gate-stage-use"
printf '%s\n' "$stage" >"$TMPDIR/cargo-$stage.marker"
if [[ "$stage" == "check" && "$cargo_scope" == "gate" \
  && "${FAKE_DELETE_BASELINE:-0}" == "1" ]]; then
  shopt -s nullglob
  baselines=("$TMPDIR"/tmp-guard-baseline)
  ((${#baselines[@]} == 1)) || { echo "FAKE_BASELINE_COUNT=${#baselines[@]}" >&2; exit 93; }
  rm -f -- "${baselines[0]}"
fi
if [[ "$stage" == "check" && "${FAKE_BLOCK_STAGE:-}" == "check" \
  && ( "${FAKE_BLOCK_SCOPE:-gate}" == "any" || "${FAKE_BLOCK_SCOPE:-gate}" == "$cargo_scope" ) ]]; then
  stubborn_pid="" stubborn_pgid="" stubborn_cgroup=""
  if [[ "${FAKE_STUBBORN_CHILD:-0}" == "1" ]]; then
    setsid bash -c 'trap "" HUP INT TERM; while :; do sleep 60; done' &
    stubborn_pid=$!
    for _ in {1..100}; do stubborn_pgid="$(ps -o pgid= -p "$stubborn_pid" | tr -d ' ')"; [[ "$stubborn_pgid" == "$stubborn_pid" ]] && break; sleep 0.01; done
    [[ "$stubborn_pgid" == "$stubborn_pid" ]] || exit 96
    stubborn_cgroup="$(awk -F: '$1=="0"{print $3}' "/proc/$stubborn_pid/cgroup")"
  fi
  if [[ -n "${FAKE_READY_FILE:-}" ]]; then
    printf 'pid=%s pgid=%s cgroup=%s cgroup_id=%s child=%s child_pgid=%s child_cgroup=%s\n' \
      "$$" "$pgid" "$cgroup" "$cgroup_id" "$stubborn_pid" "$stubborn_pgid" "$stubborn_cgroup" >"$FAKE_READY_FILE"
  fi
  trap 'exit 143' TERM
  while :; do sleep 1; done
fi
if [[ "$stage" == "check" && "${FAKE_LEAK_DESCENDANT:-0}" == "1" \
  && ( "${FAKE_LEAK_SCOPE:-gate}" == "any" || "${FAKE_LEAK_SCOPE:-gate}" == "$cargo_scope" ) ]]; then
  setsid bash -c 'trap "" HUP INT TERM; while :; do sleep 60; done' &
  leak_pid=$!
  for _ in {1..100}; do leak_pgid="$(ps -o pgid= -p "$leak_pid" | tr -d ' ')"; [[ "$leak_pgid" == "$leak_pid" ]] && break; sleep 0.01; done
  [[ "$leak_pgid" == "$leak_pid" ]] || exit 96
  leak_cgroup="$(awk -F: '$1=="0"{print $3}' "/proc/$leak_pid/cgroup")"
  [[ -z "${FAKE_READY_FILE:-}" ]] || printf 'pid=%s pgid=%s cgroup=%s cgroup_id=%s child=%s child_pgid=%s child_cgroup=%s\n' \
    "$$" "$pgid" "$cgroup" "$cgroup_id" "$leak_pid" "$leak_pgid" "$leak_cgroup" >"$FAKE_READY_FILE"
fi
if [[ "$stage" == "check" && -n "${FAKE_DELAY_SECONDS:-}" ]]; then
  sleep "$FAKE_DELAY_SECONDS"
fi
if [[ -n "${FAKE_FAIL_STAGE:-}" && "$stage" == "$FAKE_FAIL_STAGE" ]]; then
  exit "${FAKE_FAIL_CODE:-42}"
fi
STUB
  chmod +x "$repo/fakebin/cargo"
  cat >"$repo/fakebin/cargo-nextest" <<'STUB'
#!/usr/bin/env bash
exit 0
STUB
  chmod +x "$repo/fakebin/cargo-nextest"
}

run_gate() {
  local repo="$1" log="$2" stage_log="$3" fail_stage="${4:-}"
  local delete_baseline="${5:-0}" delay="${6:-}"
  local calyx_home="${7:-$(dirname "$repo")}"
  : >"$stage_log"
  set +e
  env -u CALYX_FSV_ROOT -u CALYX_GATE_RUN_SLICE -u CALYX_GATE_RUN_SLICE_OWNER_PID \
    HOME="$repo/home" \
    PATH="$repo/fakebin:$PATH" \
    CALYX_TMP_ROOT="$repo/global-tmp" \
    CALYX_TMP_MIN_FREE_INODE_PERCENT=0 \
    CALYX_TMP_MIN_FREE_BLOCK_PERCENT=0 \
    CALYX_TARGET_SCRATCH_ROOT="$calyx_home/target-scratch" \
    FAKE_TMP_LOG="$stage_log" \
    FAKE_FAIL_STAGE="$fail_stage" \
    FAKE_FAIL_CODE=42 \
    FAKE_DELETE_BASELINE="$delete_baseline" \
    FAKE_BLOCK_STAGE= \
    FAKE_BLOCK_SCOPE=gate \
    FAKE_READY_FILE= \
    FAKE_STUBBORN_CHILD=0 \
  FAKE_LEAK_DESCENDANT=0 \
    FAKE_LEAK_SCOPE=gate \
    FAKE_REQUIRE_OWNED_MODE=1 \
    FAKE_DELAY_SECONDS="$delay" \
    CALYX_GATE_SIGNAL_GRACE_SECONDS=0.5 \
    timeout --signal=TERM --kill-after=2s 30s bash "$repo/scripts/check.sh" >"$log" 2>&1
  RUN_RC=$?
  set -e
}

gate_tmp_from_log() {
  sed -n 's/^scripts\/check.sh: TMPDIR=//p' "$1" | head -n 1
}

gate_created_tmp_from_log() {
  sed -n 's/.*calyx_check_tmpdir_created root=[^ ]* run=\([^ ]*\) uid=.*/\1/p' "$1" | head -n 1
}

unique_stage_tmps() {
  sed -n 's/.* tmp=\([^ ]*\) target=.*/\1/p' "$1" | sort -u
}

unique_stage_targets() {
  sed -n 's/.* target=\([^ ]*\) pid=.*/\1/p' "$1" | sort -u
}

gate_target_from_log() {
  sed -n 's/^scripts\/check.sh: CARGO_TARGET_DIR=//p' "$1" | head -n 1
}

assert_gate_clean() {
  local repo="$1" log="$2" label="$3" run
  run="$(gate_tmp_from_log "$log")"
  [[ "$run" == "$(dirname "$repo")/tmp/"check.* ]] \
    && pass "$label run path is inside the Calyx tmp root" \
    || bad "$label run path is unexpected: $run"
  [[ -n "$run" && ! -e "$run" && ! -L "$run" ]] \
    && pass "$label run path is absent after exit" \
    || bad "$label run path remains: $run"
}

echo "=================================================================="
echo "SCENARIO 1 - successful gate shares one run-owned TMPDIR and cleans it"
echo "=================================================================="
repo1="$work/success/calyx/repo"
mkdir -p "$repo1"
setup_repo "$repo1"
log1="$work/success/gate.log"
stages1="$work/success/stages.log"
run_gate "$repo1" "$log1" "$stages1"
[[ "$RUN_RC" -eq 0 ]] && pass "successful gate exits 0" || bad "successful gate exited $RUN_RC"
if [[ "$RUN_RC" -ne 0 ]]; then
  sed -n '1,80p' "$log1" | sed 's/^/    gate: /'
fi
stage_count="$(wc -l <"$stages1" | tr -d ' ')"
unique_count="$(unique_stage_tmps "$stages1" | wc -l | tr -d ' ')"
target1="$(gate_target_from_log "$log1")"
[[ "$stage_count" -eq 15 && "$unique_count" -eq 1 ]] \
  && pass "all 15 check and real-fuzz stages observed one identical TMPDIR" \
  || bad "stage_count=$stage_count unique_tmp_count=$unique_count"
gate_test_exact_stage_manifest "$stages1" \
  secret-scan.sh check-tmpdir-regression.sh gate-wrapper-tmpdir-regression.sh \
  cargo-fmt-workspace.sh cargo:check cargo:clippy cargo:fmt cargo:check cargo:clippy \
  cargo:nextest cargo:test orphan_rs.sh linecount.sh verify_dataset.sh check_manifest_coverage.sh \
  && pass "check gate stage manifest is exact and ordered" \
  || bad "check gate stage manifest was dropped, duplicated, or reordered"
[[ "$(unique_stage_targets "$stages1")" == "$target1" \
  && "$target1" == "$(dirname "$repo1")/target-scratch/"check.* ]] \
  && pass "every stage observed the exact nonempty check-owned Cargo target" \
  || bad "stage targets diverged from '$target1'"
gate_test_log_groups_owned "$stages1" "$target1" \
  && pass "every stage ran as its own process-group leader" \
  || bad "a stage bypassed owned process-group or target binding"
grep -Fq 'calyx_check_tmpdir_valid root=' "$log1" \
  && grep -Fq 'stage=cargo:fmt ' "$stages1" \
  && pass "check invoked real fuzz with a validated outer TMPDIR marker" \
  || bad "real fuzz marker validation evidence is absent"
fuzz_wrapper_pid=""
fuzz_wrapper_pgid=""
read -r fuzz_wrapper_pid fuzz_wrapper_pgid < <(
  sed -n 's/^fuzz-gate: pid=\([0-9]*\) pgid=\([0-9]*\)$/\1 \2/p' "$log1" | head -n1
) || true
[[ -n "${fuzz_wrapper_pid:-}" && "$fuzz_wrapper_pid" == "$fuzz_wrapper_pgid" ]] \
  && pass "check launched the real fuzz wrapper as an owned process-group leader" \
  || bad "real fuzz wrapper bypassed stage-group ownership"
assert_gate_clean "$repo1" "$log1" "success"
gate_test_cleanup_readback_bounded "$log1" \
  && gate_test_tmp_guard_readbacks_bounded "$log1" 3 \
  && pass "check emits one cleanup plus bounded pre/post capacity readbacks" \
  || bad "check cleanup/capacity readback shape or count diverged"
[[ -n "$target1" && ! -e "$target1" ]] \
  && pass "check-owned Cargo target is absent after success" \
  || bad "check-owned Cargo target remains: $target1"
lock1="$(dirname "$repo1")/tmp/.cargo-gate.lock"
[[ -z "$(find "$(dirname "$repo1")/tmp" -mindepth 1 -maxdepth 1 -name 'check.*' -print -quit)" \
  && -f "$lock1" && ! -L "$lock1" \
  && "$(stat -c '%u:%a' -- "$lock1")" == "$(id -u):600" ]] \
  && pass "Calyx tmp run roots are absent and the stable gate lock is private" \
  || bad "Calyx tmp root retained a run or unsafe gate lock"

echo
echo "=================================================================="
echo "SCENARIO 2 - stage rc 42 is preserved and cleanup still runs"
echo "=================================================================="
repo2="$work/failure/calyx/repo"
mkdir -p "$repo2"
setup_repo "$repo2"
log2="$work/failure/gate.log"
stages2="$work/failure/stages.log"
run_gate "$repo2" "$log2" "$stages2" check
[[ "$RUN_RC" -eq 42 ]] && pass "failing stage preserves rc 42" || bad "failing gate exited $RUN_RC"
! grep -Fq 'stage=cargo:clippy ' "$stages2" \
  && pass "stages after cargo check did not run" \
  || bad "a later stage ran after cargo check failed"
assert_gate_clean "$repo2" "$log2" "failure"

echo
echo "=================================================================="
echo "SCENARIO 3 - missing completed baseline fails an otherwise-green gate"
echo "=================================================================="
repo3="$work/baseline/calyx/repo"
mkdir -p "$repo3"
setup_repo "$repo3"
log3="$work/baseline/gate.log"
stages3="$work/baseline/stages.log"
run_gate "$repo3" "$log3" "$stages3" "" 1
[[ "$RUN_RC" -ne 0 ]] && pass "missing baseline makes the gate nonzero" || bad "missing baseline was ignored"
if [[ "$RUN_RC" -eq 0 ]]; then
  sed -n '1,120p' "$log3" | sed 's/^/    gate: /'
fi
grep -Fq 'CALYX_TMP_GUARD_MISSING_BASELINE' "$log3" \
  && pass "tmp guard names the missing baseline" \
  || bad "missing-baseline diagnostic absent"
grep -Fq 'CALYX_CHECK_TMP_GUARD_CLEANUP_FAILED' "$log3" \
  && pass "gate propagates the tmp-guard cleanup failure" \
  || bad "gate cleanup failure diagnostic absent"
assert_gate_clean "$repo3" "$log3" "missing-baseline"
# shellcheck source=check-tmpdir-process-regression.sh
source "$script_dir/check-tmpdir-process-regression.sh"
# shellcheck source=check-tmpdir-boundary-regression.sh
source "$script_dir/check-tmpdir-boundary-regression.sh"
# shellcheck source=check-setup-publication-regression.sh
source "$setup_publication_scenario"
echo
echo "=================================================================="
echo "SCENARIO 7 - concurrent gates receive distinct roots and both clean"
echo "=================================================================="
repo6="$work/concurrent/calyx/repo"
mkdir -p "$repo6"
setup_repo "$repo6"
cp "$target_clean_src" "$repo6/scripts/cargo-target-clean.sh"
chmod +x "$repo6/scripts/cargo-target-clean.sh"
log6a="$work/concurrent/gate-a.log"
log6b="$work/concurrent/gate-b.log"
stages6a="$work/concurrent/stages-a.log"
stages6b="$work/concurrent/stages-b.log"
common6=(env -u CALYX_FSV_ROOT -u CALYX_GATE_RUN_SLICE -u CALYX_GATE_RUN_SLICE_OWNER_PID HOME="$repo6/home" PATH="$repo6/fakebin:$PATH"
  CALYX_TMP_ROOT="$repo6/global-tmp" CALYX_TMP_MIN_FREE_INODE_PERCENT=0
  CALYX_TMP_MIN_FREE_BLOCK_PERCENT=0 CALYX_TARGET_SCRATCH_ROOT="$(dirname "$repo6")/target-scratch"
  CALYX_GATE_SIGNAL_GRACE_SECONDS=0.5
  FAKE_REQUIRE_OWNED_MODE=1
  FAKE_FAIL_STAGE= FAKE_DELETE_BASELINE=0 FAKE_BLOCK_STAGE= FAKE_BLOCK_SCOPE=gate FAKE_READY_FILE=
  FAKE_STUBBORN_CHILD=0 FAKE_LEAK_DESCENDANT=0 FAKE_DELAY_SECONDS=)
barrier6_ready="$work/concurrent/gate-a-ready"
barrier6_release="$work/concurrent/gate-a-release"
setsid "${common6[@]}" FAKE_TMP_LOG="$stages6a" \
  FAKE_BARRIER_STAGE=secret-scan.sh FAKE_BARRIER_READY="$barrier6_ready" \
  FAKE_BARRIER_RELEASE="$barrier6_release" \
  bash "$repo6/scripts/check.sh" >"$log6a" 2>&1 &
pid6a=$!
gate_test_wait_ready "$barrier6_ready" || true
concurrent_flock_ready="$work/concurrent/gate-b-flock-ready"
cat >"$repo6/fakebin/flock" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
[[ " $* " != *" --help "* ]] || exec /usr/bin/flock "$@"
[[ -z "${CONCURRENT_FLOCK_READY:-}" ]] || printf 'pid=%s pgid=%s\n' \
  "$BASHPID" "$(ps -o pgid= -p "$BASHPID" | tr -d ' ')" >"$CONCURRENT_FLOCK_READY"
exec /usr/bin/flock "$@"
STUB
chmod +x "$repo6/fakebin/flock"
setsid "${common6[@]}" FAKE_TMP_LOG="$stages6b" \
  CONCURRENT_FLOCK_READY="$concurrent_flock_ready" \
  bash "$repo6/scripts/check.sh" >"$log6b" 2>&1 &
pid6b=$!
owned_extra_pgids="$pid6a $pid6b"
gate_test_wait_ready "$concurrent_flock_ready" || true
run6a_live="$(gate_created_tmp_from_log "$log6a")"
run6b_live="$(gate_created_tmp_from_log "$log6b")"
target6a_live="$(gate_target_from_log "$log6a")"
target6b_live="$(gate_target_from_log "$log6b")"
scratch6="$(dirname "$repo6")/target-scratch"
scratch6_count="$(find "$scratch6" -mindepth 1 -maxdepth 1 -type d -print | wc -l | tr -d ' ')"
concurrent_flock_pid="$(sed -n 's/^pid=\([0-9]*\).*/\1/p' "$concurrent_flock_ready" 2>/dev/null | head -n1 || true)"
sleep 0.1
[[ -s "$barrier6_ready" && -d "$run6a_live" && -d "$run6b_live" \
  && "$run6a_live" != "$run6b_live" && -d "$target6a_live" \
  && -z "$target6b_live" && "$scratch6_count" -eq 1 \
  && "$concurrent_flock_pid" =~ ^[1-9][0-9]*$ && -e "/proc/$concurrent_flock_pid" ]] \
  && pass "overlap readback shows two TMPDIRs, one published target, and gate B queued on flock" \
  || bad "concurrent pre-release state did not prove global serialization"
printf 'release\n' >"$barrier6_release"
concurrent_watchdog_marker="$work/concurrent/watchdog-fired"
(
  sleep 15
  printf 'fired\n' >"$concurrent_watchdog_marker"
  kill -TERM -- "-$pid6a" "-$pid6b" 2>/dev/null || true
  sleep 0.5
  kill -KILL -- "-$pid6a" "-$pid6b" 2>/dev/null || true
) &
concurrent_watchdog=$!
set +e
wait "$pid6a"; rc6a=$?
wait "$pid6b"; rc6b=$?
set -e
kill -KILL "$concurrent_watchdog" 2>/dev/null || true
wait "$concurrent_watchdog" 2>/dev/null || true
[[ ! -e "$concurrent_watchdog_marker" ]] \
  && pass "concurrent gates completed before their regression watchdog" \
  || bad "concurrent gates required watchdog rescue"
owned_extra_pgids=""
run6a="$(gate_tmp_from_log "$log6a")"
run6b="$(gate_tmp_from_log "$log6b")"
target6a="$(gate_target_from_log "$log6a")"
target6b="$(gate_target_from_log "$log6b")"
[[ "$rc6a" -eq 0 && "$rc6b" -eq 0 ]] \
  && pass "both concurrent gates exit 0" \
  || { bad "concurrent exits were $rc6a and $rc6b"; tail -25 "$log6a" | sed 's/^/    gate-a: /'; tail -25 "$log6b" | sed 's/^/    gate-b: /'; }
[[ -n "$run6a" && -n "$run6b" && "$run6a" != "$run6b" ]] \
  && pass "concurrent gates used distinct run roots" \
  || bad "concurrent run roots collided: '$run6a' '$run6b'"
[[ -n "$target6a" && -n "$target6b" && "$target6a" != "$target6b" \
  && ! -e "$target6a" && ! -e "$target6b" ]] \
  && pass "serialized gates eventually used and removed distinct targets" \
  || bad "concurrent target identities collided or remained"
[[ "$(unique_stage_tmps "$stages6a" | wc -l | tr -d ' ')" -eq 1 \
  && "$(unique_stage_tmps "$stages6b" | wc -l | tr -d ' ')" -eq 1 ]] \
  && pass "each concurrent gate kept an internally uniform TMPDIR" \
  || bad "a concurrent gate observed multiple TMPDIR values"
[[ ! -e "$run6a" && ! -e "$run6b" \
  && -z "$(find "$(dirname "$repo6")/tmp" -mindepth 1 -maxdepth 1 -name 'check.*' -print -quit)" \
  && -f "$(dirname "$repo6")/tmp/.cargo-gate.lock" \
  && -z "$(find "$scratch6" -mindepth 1 -print -quit)" ]] \
  && pass "both concurrent run roots are absent and target-scratch is exactly empty" \
  || bad "a concurrent run root remains"
rm -- "$repo6/fakebin/flock"
source "$final_contracts"

if [[ "$fail" -eq 0 ]]; then
  echo "ALL 7 LIFECYCLE SCENARIOS AND RUNNER/TARGET CONTRACTS PASSED"
else
  echo "REGRESSION DETECTED" >&2
fi
exit "$fail"
