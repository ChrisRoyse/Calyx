#!/usr/bin/env bash
# Regression coverage for public and standalone gate TMPDIR ownership (#1421).
set -euo pipefail
unset CALYX_GATE_RUN_SLICE CALYX_GATE_RUN_SLICE_OWNER_PID \
  CALYX_GATE_RUN_SLICE_OWNER_STARTTIME

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
repo_scripts="$(cd "$script_dir/.." && pwd -P)"
helper_src="$repo_scripts/check-tmpdir.sh"
guard_src="$repo_scripts/tmp_scratch_guard.sh"
fuzz_src="$repo_scripts/fuzz-gate.sh"
public_src="$repo_scripts/public-mirror-gate.sh"
runner_src="$repo_scripts/gate-stage-runner.sh"
runtime_src="$repo_scripts/gate-runtime-state.sh"
capture_src="$repo_scripts/gate-identity-capture.sh"
record_exec_src="$repo_scripts/gate_identity_record_exec.py"
cgroup_src="$repo_scripts/gate-cgroup-runtime.sh"
cgroup_scope_src="$repo_scripts/gate-cgroup-scope.sh"
ofd_query_src="$repo_scripts/gate_ofd_lock_query.py"
safe_helper_src="$repo_scripts/gate_safe_rmtree.py"
creator_src="$repo_scripts/gate_owned_dir_create.py"
test_lib="$script_dir/gate-tmpdir-regression-lib.sh"
public_scenarios="$script_dir/gate-wrapper-public-regression.sh"
public_boundary="$script_dir/gate-wrapper-public-boundary-regression.sh"
public_signal_variants="$script_dir/gate-wrapper-public-signal-variants-regression.sh"
nested_contracts="$script_dir/gate-nested-fuzz-contract-regression.sh"
cross_lock_contracts="$script_dir/gate-cross-wrapper-lock-regression.sh"
setup_publication_contracts="$script_dir/gate-wrapper-setup-publication-regression.sh"
fuzz_extra_signals="$script_dir/gate-wrapper-fuzz-extra-signals-regression.sh"
nested_worktree_contracts="$script_dir/gate-nested-worktree-regression.sh"
final_cleanup_contracts="$script_dir/gate-wrapper-final-cleanup-regression.sh"
for required in "$helper_src" "$guard_src" "$fuzz_src" "$public_src" "$runner_src" \
  "$runtime_src" "$capture_src" "$record_exec_src" "$cgroup_src" "$cgroup_scope_src" "$ofd_query_src" "$safe_helper_src" "$creator_src" \
  "$test_lib" "$public_scenarios" "$public_boundary" "$public_signal_variants" "$nested_contracts" "$cross_lock_contracts" \
  "$setup_publication_contracts" "$fuzz_extra_signals" "$nested_worktree_contracts" \
  "$final_cleanup_contracts"; do
  [[ -f "$required" ]] || { echo "REGRESSION SETUP ERROR: missing $required" >&2; exit 2; }
done
# shellcheck source=gate-tmpdir-regression-lib.sh
source "$test_lib"
for required_command in setsid flock timeout python3 systemd-run systemctl findmnt; do
  command -v "$required_command" >/dev/null 2>&1 || {
    echo "REGRESSION SETUP ERROR: $required_command is required" >&2; exit 2;
  }
done

work="$(mktemp -d)"
owned_pids=""
owned_pgids=""
cleanup_test() {
  local status=$? pid pgid
  trap - EXIT HUP INT TERM
  for pid in $owned_pids; do
    kill -TERM "$pid" 2>/dev/null || true
    kill -KILL "$pid" 2>/dev/null || true
  done
  for pgid in $owned_pgids; do
    gate_test_stop_group_bounded "$pgid" || true
  done
  rm -rf -- "$work"
  exit "$status"
}
trap cleanup_test EXIT
fail=0
pass() { echo "  PASS: $1"; }
bad()  { echo "  FAIL: $1" >&2; fail=1; }

write_fake_cargo() {
  local root="$1"
  mkdir -p "$root/fakebin"
  cat >"$root/fakebin/cargo" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
stage="${1:-none}"
cargo_scope=gate
for arg in "$@"; do
  [[ "$arg" != */fuzz/Cargo.toml ]] || cargo_scope=fuzz
done
[[ -d "${TMPDIR:?}" ]] || { echo "WRAPPER_TMPDIR_MISSING: $stage" >&2; exit 90; }
if [[ "${FAKE_REQUIRE_OWNED_MODE:-0}" == "1" ]]; then
  [[ "$(stat -c '%u:%a' -- "$TMPDIR")" == "$(id -u):700" \
    && "$(stat -c '%u:%a' -- "${CARGO_TARGET_DIR:?}")" == "$(id -u):700" ]] || exit 95
fi
pgid="$(ps -o pgid= -p "$$" | tr -d ' ')" cgroup="$(awk -F: '$1=="0"{print $3}' /proc/self/cgroup)" cgroup_id="$(stat -c '%d:%i' -- "/sys/fs/cgroup$cgroup")"
printf 'stage=cargo:%s tmp=%s target=%s pid=%s pgid=%s cgroup=%s cgroup_id=%s\n' \
  "$stage" "$TMPDIR" "${CARGO_TARGET_DIR:-}" "$$" "$pgid" "$cgroup" "$cgroup_id" >>"${FAKE_TMP_LOG:?}"
[[ ! -e "${CARGO_TARGET_DIR:?}/.preclean-sentinel" ]] || exit 94
printf 'cargo:%s\n' "$stage" >>"$CARGO_TARGET_DIR/.gate-stage-use"
printf '%s\n' "$stage" >"$TMPDIR/cargo-$stage.marker"
if [[ "$stage" == "check" && "$cargo_scope" == "gate" \
  && "${FAKE_DELETE_BASELINE:-0}" == "1" ]]; then
  shopt -s nullglob
  baselines=("$TMPDIR"/tmp-guard-baseline)
  ((${#baselines[@]} == 1)) || exit 93
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
  [[ -z "${FAKE_READY_FILE:-}" ]] || printf 'pid=%s pgid=%s cgroup=%s cgroup_id=%s child=%s child_pgid=%s child_cgroup=%s\n' \
    "$$" "$pgid" "$cgroup" "$cgroup_id" "$stubborn_pid" "$stubborn_pgid" "$stubborn_cgroup" >"$FAKE_READY_FILE"
  trap 'exit 143' TERM
  while :; do sleep 1; done
fi
if [[ -n "${FAKE_FAIL_STAGE:-}" && "$stage" == "$FAKE_FAIL_STAGE" ]]; then
  exit "${FAKE_FAIL_CODE:-42}"
fi
STUB
  chmod +x "$root/fakebin/cargo"
  cat >"$root/fakebin/cargo-nextest" <<'STUB'
#!/usr/bin/env bash
exit 0
STUB
  chmod +x "$root/fakebin/cargo-nextest"
}

write_target_clean_stub() {
  local path="$1"
  cat >"$path" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
target="" repo_root="" expected="" lock_fd=""
workspace=0 repo_targets=0 home_targets=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --target-dir) target="$2"; shift 2 ;;
    --repo-root) repo_root="$2"; shift 2 ;;
    --expected-target-identity) expected="$2"; shift 2 ;;
    --gate-lock-fd) lock_fd="$2"; shift 2 ;;
    --also-workspace-target) workspace=1; shift ;;
    --also-repo-targets) repo_targets=1; shift ;;
    --also-calyx-home-target) home_targets=1; shift ;;
    *) echo "TARGET_STUB_UNKNOWN_ARG: $1" >&2; exit 96 ;;
  esac
done
[[ -n "$target" && -n "$repo_root" && "$workspace" -eq 1 \
  && "$repo_targets" -eq 1 && "$home_targets" -eq 1 ]] || exit 91
[[ "$lock_fd" =~ ^[0-9]+$ && -e "/proc/$BASHPID/fd/$lock_fd" \
  && -n "${CALYX_GATE_LOCK_PATH:-}" ]] || exit 97
[[ "$(readlink -f -- "/proc/$BASHPID/fd/$lock_fd")" == "$CALYX_GATE_LOCK_PATH" ]] || exit 98
flock --fcntl --exclusive --nonblock "$lock_fd" || exit 99
if [[ -n "$expected" ]]; then
  [[ -d "$target" && "$(stat -c '%d:%i' -- "$target")" == "$expected" ]] || exit 100
fi
sentinel=0
[[ ! -e "$target/.preclean-sentinel" ]] || sentinel=1
use_lines=0
if [[ -f "$target/.gate-stage-use" ]]; then
  use_lines="$(wc -l <"$target/.gate-stage-use" | tr -d ' ')"
fi
[[ "$use_lines" -eq 0 || -n "$expected" ]] || exit 101
if [[ -n "${FAKE_TARGET_CLEAN_LOG:-}" ]]; then
  printf 'target=%s sentinel=%s use_lines=%s\n' "$target" "$sentinel" "$use_lines" \
    >>"$FAKE_TARGET_CLEAN_LOG"
fi
if [[ -n "${FAKE_TARGET_CLEAN_FAIL_ONCE_FILE:-}" && ! -e "$FAKE_TARGET_CLEAN_FAIL_ONCE_FILE" ]]; then
  printf 'failed-once\n' >"$FAKE_TARGET_CLEAN_FAIL_ONCE_FILE"
  echo "TARGET_STUB_FAIL_ONCE: target=$target" >&2
  exit 73
fi
if [[ -n "${FAKE_TARGET_CLEAN_BLOCK_READY:-}" ]]; then
  clean_pgid="$(ps -o pgid= -p "$BASHPID" | tr -d ' ')"
  printf 'pid=%s pgid=%s target=%s\n' "$BASHPID" "$clean_pgid" "$target" \
    >"$FAKE_TARGET_CLEAN_BLOCK_READY"
  while [[ ! -e "${FAKE_TARGET_CLEAN_BLOCK_RELEASE:?}" ]]; do sleep 0.05; done
fi
rm -rf -- "$target"
[[ ! -e "$target" ]] || exit 92
echo "cargo-target-clean: target_absent path=$target"
STUB
  chmod +x "$path"
}

unique_tmps() {
  sed -n 's/.* tmp=\([^ ]*\) target=.*/\1/p' "$1" | sort -u
}

unique_targets() {
  sed -n 's/.* target=\([^ ]*\) pid=.*/\1/p' "$1" | sort -u
}

echo "=================================================================="
echo "SCENARIO 1 - standalone fuzz gate owns and removes its TMPDIR"
echo "=================================================================="
fuzz_repo="$work/fuzz/calyx/repo"
mkdir -p "$fuzz_repo/scripts" "$fuzz_repo/fuzz" "$fuzz_repo/home/.cargo" \
  "$(dirname "$fuzz_repo")/target-scratch"
chmod 755 "$(dirname "$fuzz_repo")" "$fuzz_repo"
chmod 700 "$(dirname "$fuzz_repo")/target-scratch"
cp "$helper_src" "$fuzz_repo/scripts/check-tmpdir.sh"
cp "$fuzz_src" "$fuzz_repo/scripts/fuzz-gate.sh"
cp "$runner_src" "$fuzz_repo/scripts/gate-stage-runner.sh"
cp "$runtime_src" "$fuzz_repo/scripts/gate-runtime-state.sh"
cp "$capture_src" "$fuzz_repo/scripts/gate-identity-capture.sh"
cp "$safe_helper_src" "$creator_src" "$record_exec_src" "$fuzz_repo/scripts/"
cp "$cgroup_src" "$fuzz_repo/scripts/gate-cgroup-runtime.sh"
cp "$cgroup_scope_src" "$fuzz_repo/scripts/gate-cgroup-scope.sh"
cp "$ofd_query_src" "$fuzz_repo/scripts/gate_ofd_lock_query.py"
write_target_clean_stub "$fuzz_repo/scripts/cargo-target-clean.sh"
write_fake_cargo "$fuzz_repo"
: >"$fuzz_repo/home/.cargo/env"
printf 'export CALYX_HOME=%q\n' "$(dirname "$fuzz_repo")" >"$fuzz_repo/env.sh"
printf '[workspace]\nmembers=[]\n' >"$fuzz_repo/fuzz/Cargo.toml"
unowned_target="$work/fuzz/unowned-target"
mkdir -m 700 "$unowned_target"
printf 'unowned-target-sentinel\n' >"$unowned_target/sentinel"
unowned_sha="$(sha256sum "$unowned_target/sentinel" | awk '{print $1}')"
set +e
env -u CALYX_FSV_ROOT -u CALYX_GATE_RUN_SLICE -u CALYX_GATE_RUN_SLICE_OWNER_PID -u CALYX_GATE_TMPDIR -u CALYX_GATE_TMP_ROOT \
  -u CALYX_GATE_LOCK_PATH -u CALYX_GATE_LOCK_FD \
  HOME="$fuzz_repo/home" PATH="$fuzz_repo/fakebin:$PATH" \
  CARGO_TARGET_DIR="$unowned_target" FAKE_TMP_LOG="$work/fuzz/unowned-stages.log" \
  bash "$fuzz_repo/scripts/fuzz-gate.sh" >"$work/fuzz/unowned.log" 2>&1
unowned_rc=$?
set -e
[[ "$unowned_rc" -eq 2 && ! -e "$work/fuzz/unowned-stages.log" \
  && "$(sha256sum "$unowned_target/sentinel" | awk '{print $1}')" == "$unowned_sha" ]] \
  && pass "standalone fuzz refuses an unowned inherited Cargo target before scratch" \
  || bad "standalone fuzz accepted or changed an unowned target"
rm -rf -- "$unowned_target"
fuzz_log="$work/fuzz/gate.log"
fuzz_stages="$work/fuzz/stages.log"
set +e
env -u CALYX_FSV_ROOT -u CALYX_GATE_RUN_SLICE -u CALYX_GATE_RUN_SLICE_OWNER_PID -u CALYX_GATE_TMPDIR -u CALYX_GATE_TMP_ROOT \
  -u CARGO_TARGET_DIR -u TMPDIR \
  HOME="$fuzz_repo/home" PATH="$fuzz_repo/fakebin:$PATH" \
  CALYX_TARGET_SCRATCH_ROOT="$(dirname "$fuzz_repo")/target-scratch" \
  CALYX_GATE_SIGNAL_GRACE_SECONDS=0.5 \
  FAKE_TMP_LOG="$fuzz_stages" FAKE_DELETE_BASELINE=0 FAKE_FAIL_STAGE= \
  FAKE_BLOCK_STAGE= FAKE_READY_FILE= FAKE_STUBBORN_CHILD=0 FAKE_REQUIRE_OWNED_MODE=1 \
  timeout --signal=TERM --kill-after=2s 30s bash "$fuzz_repo/scripts/fuzz-gate.sh" >"$fuzz_log" 2>&1
fuzz_rc=$?
set -e
fuzz_tmp="$(sed -n 's/^fuzz-gate: TMPDIR=//p' "$fuzz_log" | head -n 1)"
fuzz_target="$(sed -n 's/^fuzz-gate: CARGO_TARGET_DIR=//p' "$fuzz_log" | head -n 1)"
[[ "$fuzz_rc" -eq 0 ]] && pass "standalone fuzz gate exits 0" || bad "standalone fuzz rc=$fuzz_rc"
[[ "$(wc -l <"$fuzz_stages" | tr -d ' ')" -eq 3 \
  && "$(unique_tmps "$fuzz_stages" | wc -l | tr -d ' ')" -eq 1 ]] \
  && pass "all three fuzz Cargo stages observed one TMPDIR" \
  || bad "fuzz stages did not share one TMPDIR"
gate_test_exact_stage_manifest "$fuzz_stages" cargo:fmt cargo:check cargo:clippy \
  && pass "standalone fuzz stage manifest is exact and ordered" \
  || bad "standalone fuzz stage manifest was dropped, duplicated, or reordered"
[[ "$fuzz_tmp" == "$(dirname "$fuzz_repo")/tmp/"check.* && ! -e "$fuzz_tmp" ]] \
  && pass "standalone fuzz TMPDIR is a cleaned Calyx child" \
  || bad "standalone fuzz TMPDIR remains or escaped: $fuzz_tmp"
[[ "$(unique_targets "$fuzz_stages")" == "$fuzz_target" \
  && "$fuzz_target" == "$(dirname "$fuzz_repo")/target-scratch/"fuzz-gate.* && ! -e "$fuzz_target" ]] \
  && pass "standalone fuzz stages used and removed one exact owned target" \
  || bad "standalone fuzz target binding diverged: $fuzz_target"
gate_test_log_groups_owned "$fuzz_stages" "$fuzz_target" \
  && pass "all standalone fuzz stages owned separate process groups" \
  || bad "a standalone fuzz stage bypassed target or process-group ownership"
gate_test_cleanup_readback_bounded "$fuzz_log" \
  && pass "standalone fuzz emits one bounded run cleanup readback" \
  || bad "standalone fuzz cleanup readback is missing, duplicated, or unbounded"

echo
echo "=================================================================="
echo "SCENARIO 2 - nested fuzz gate validates and preserves the outer root"
echo "=================================================================="
outer_tmp="$(bash "$helper_src" create "$(dirname "$fuzz_repo")/tmp" 2>"$work/fuzz/outer-create.log")"
outer_target_root="$(dirname "$fuzz_repo")/target-scratch"
mkdir -p "$outer_target_root/outer-target"
chmod 700 "$outer_target_root/outer-target"
printf 'caller-owned-target\n' >"$outer_target_root/outer-target/sentinel"
outer_target_sha="$(sha256sum "$outer_target_root/outer-target/sentinel" | awk '{print $1}')"
outer_tmp_root="$(dirname "$fuzz_repo")/tmp"
outer_tmp_root_identity="$(stat -c '%d:%i' -- "$outer_tmp_root")"
outer_tmp_identity="$(stat -c '%d:%i' -- "$outer_tmp")"
outer_target_root_identity="$(stat -c '%d:%i' -- "$outer_target_root")"
outer_target_identity="$(stat -c '%d:%i' -- "$outer_target_root/outer-target")"
outer_lock_path="$(dirname "$fuzz_repo")/tmp/.cargo-gate.lock"
(umask 077; : >"$outer_lock_path")
exec {outer_lock_fd}>"$outer_lock_path"
flock --fcntl --exclusive "$outer_lock_fd"
nested_log="$work/fuzz/nested.log"
nested_stages="$work/fuzz/nested-stages.log"
set +e
env -u CALYX_FSV_ROOT -u CALYX_GATE_RUN_SLICE -u CALYX_GATE_RUN_SLICE_OWNER_PID -u CALYX_GATE_RUN_SLICE_OWNER_STARTTIME \
  HOME="$fuzz_repo/home" PATH="$fuzz_repo/fakebin:$PATH" \
  TMPDIR="$outer_tmp" CALYX_GATE_TMPDIR="$outer_tmp" \
  CALYX_GATE_TMP_ROOT="$outer_tmp_root" \
  CALYX_GATE_TMP_ROOT_IDENTITY="$outer_tmp_root_identity" \
  CALYX_GATE_TMP_IDENTITY="$outer_tmp_identity" \
  CALYX_GATE_LOCK_PATH="$outer_lock_path" CALYX_GATE_LOCK_FD="$outer_lock_fd" \
  CARGO_TARGET_DIR="$outer_target_root/outer-target" \
  CALYX_GATE_TARGET_ROOT="$outer_target_root" \
  CALYX_GATE_TARGET_ROOT_IDENTITY="$outer_target_root_identity" \
  CALYX_GATE_TARGET_DIR="$outer_target_root/outer-target" \
  CALYX_GATE_TARGET_IDENTITY="$outer_target_identity" \
  CALYX_GATE_SIGNAL_GRACE_SECONDS=0.5 \
  FAKE_TMP_LOG="$nested_stages" FAKE_DELETE_BASELINE=0 FAKE_FAIL_STAGE= \
  FAKE_BLOCK_STAGE= FAKE_READY_FILE= \
  timeout --signal=TERM --kill-after=2s 30s setsid bash "$fuzz_repo/scripts/fuzz-gate.sh" >"$nested_log" 2>&1
nested_rc=$?
set -e
[[ "$nested_rc" -eq 0 && -d "$outer_tmp" ]] \
  && pass "nested fuzz gate accepts but does not remove the outer root" \
  || bad "nested fuzz rc=$nested_rc or outer root was removed"
[[ "$(unique_tmps "$nested_stages")" == "$outer_tmp" ]] \
  && pass "nested fuzz stages inherited the exact outer TMPDIR" \
  || bad "nested fuzz did not preserve the outer TMPDIR"
[[ "$(unique_targets "$nested_stages")" == "$outer_target_root/outer-target" \
  && "$(sha256sum "$outer_target_root/outer-target/sentinel" | awk '{print $1}')" == "$outer_target_sha" ]] \
  && pass "nested fuzz propagated and preserved the caller-owned Cargo target" \
  || bad "nested fuzz reclaimed or replaced the caller-owned Cargo target"
# shellcheck source=gate-nested-fuzz-contract-regression.sh
source "$nested_contracts"

echo
echo "=================================================================="
echo "SCENARIO 3 - forged outer marker is rejected before Cargo"
echo "=================================================================="
forged_tmp="$work/fuzz/forged/check.forged"
mkdir -p "$forged_tmp"
chmod 700 "$(dirname "$forged_tmp")" "$forged_tmp"
printf 'forged-marker-sentinel\n' >"$forged_tmp/sentinel"
forged_sha="$(sha256sum "$forged_tmp/sentinel" | awk '{print $1}')"
forged_identity="$(stat -c '%d:%i' -- "$forged_tmp")"
forged_stages="$work/fuzz/forged-stages.log"
set +e
env -u CALYX_FSV_ROOT -u CALYX_GATE_RUN_SLICE -u CALYX_GATE_RUN_SLICE_OWNER_PID \
  HOME="$fuzz_repo/home" PATH="$fuzz_repo/fakebin:$PATH" \
  TMPDIR="$forged_tmp" CALYX_GATE_TMPDIR="$forged_tmp" \
  CALYX_GATE_TMP_ROOT="$outer_tmp_root" \
  CALYX_GATE_TMP_ROOT_IDENTITY="$outer_tmp_root_identity" \
  CALYX_GATE_TMP_IDENTITY="$forged_identity" \
  CALYX_GATE_LOCK_PATH="$outer_lock_path" CALYX_GATE_LOCK_FD="$outer_lock_fd" \
  CARGO_TARGET_DIR="$outer_target_root/outer-target" \
  CALYX_GATE_TARGET_ROOT="$outer_target_root" \
  CALYX_GATE_TARGET_ROOT_IDENTITY="$outer_target_root_identity" \
  CALYX_GATE_TARGET_DIR="$outer_target_root/outer-target" \
  CALYX_GATE_TARGET_IDENTITY="$outer_target_identity" \
  FAKE_TMP_LOG="$forged_stages" FAKE_DELETE_BASELINE=0 FAKE_FAIL_STAGE= \
  FAKE_BLOCK_STAGE= FAKE_READY_FILE= \
  timeout --signal=TERM --kill-after=2s 30s setsid bash "$fuzz_repo/scripts/fuzz-gate.sh" >"$work/fuzz/forged.log" 2>&1
forged_rc=$?
set -e
if [[ "$forged_rc" -ne 0 && ! -e "$forged_stages" \
  && "$(sha256sum "$forged_tmp/sentinel" | awk '{print $1}')" == "$forged_sha" ]] \
  && grep -Fq 'is not a direct child' "$work/fuzz/forged.log"; then
  pass "forged marker failed closed with sentinel preserved"
else
  bad "forged marker reached Cargo or changed its sentinel"
  sed -n '1,30p' "$work/fuzz/forged.log" | sed 's/^/    forged: /'
fi
eval "exec ${outer_lock_fd}>&-"
bash "$helper_src" cleanup "$outer_tmp_root" "$outer_tmp" \
  "$outer_tmp_root_identity" "$outer_tmp_identity" >"$work/fuzz/outer-cleanup.log"

echo
echo "=================================================================="
echo "SCENARIO 4 - standalone fuzz stage rc 42 is preserved and cleaned"
echo "=================================================================="
fuzz_fail_log="$work/fuzz/failure.log"
fuzz_fail_stages="$work/fuzz/failure-stages.log"
set +e
env -u CALYX_FSV_ROOT -u CALYX_GATE_RUN_SLICE -u CALYX_GATE_RUN_SLICE_OWNER_PID -u CALYX_GATE_TMPDIR -u CALYX_GATE_TMP_ROOT \
  -u CARGO_TARGET_DIR -u TMPDIR \
  HOME="$fuzz_repo/home" PATH="$fuzz_repo/fakebin:$PATH" \
  CALYX_TARGET_SCRATCH_ROOT="$(dirname "$fuzz_repo")/target-scratch" \
  CALYX_GATE_SIGNAL_GRACE_SECONDS=0.5 \
  FAKE_TMP_LOG="$fuzz_fail_stages" FAKE_DELETE_BASELINE=0 \
  FAKE_FAIL_STAGE=check FAKE_FAIL_CODE=42 FAKE_BLOCK_STAGE= FAKE_READY_FILE= \
  FAKE_REQUIRE_OWNED_MODE=1 \
  timeout --signal=TERM --kill-after=2s 30s bash "$fuzz_repo/scripts/fuzz-gate.sh" >"$fuzz_fail_log" 2>&1
fuzz_fail_rc=$?
set -e
fuzz_fail_tmp="$(sed -n 's/^fuzz-gate: TMPDIR=//p' "$fuzz_fail_log" | head -n1)"
fuzz_fail_target="$(sed -n 's/^fuzz-gate: CARGO_TARGET_DIR=//p' "$fuzz_fail_log" | head -n1)"
[[ "$fuzz_fail_rc" -eq 42 && ! -e "$fuzz_fail_tmp" && ! -e "$fuzz_fail_target" ]] \
  && pass "fuzz rc 42 preserved with temp and target roots absent" \
  || bad "fuzz failure rc=$fuzz_fail_rc or scratch remained"
! grep -Fq 'stage=cargo:clippy ' "$fuzz_fail_stages" \
  && pass "fuzz stages after the failing check did not run" \
  || bad "fuzz clippy ran after check failed"

echo
echo "=================================================================="
echo "SCENARIO 5 - PID-only TERM is forwarded by the standalone fuzz gate"
echo "=================================================================="
fuzz_signal_log="$work/fuzz/signal.log"
fuzz_signal_stages="$work/fuzz/signal-stages.log"
fuzz_ready="$work/fuzz/signal-ready"
setsid env -u CALYX_FSV_ROOT -u CALYX_GATE_RUN_SLICE -u CALYX_GATE_RUN_SLICE_OWNER_PID \
  -u CALYX_GATE_RUN_SLICE_OWNER_STARTTIME -u CALYX_GATE_TMPDIR -u CALYX_GATE_TMP_ROOT \
  -u CARGO_TARGET_DIR -u TMPDIR \
  HOME="$fuzz_repo/home" PATH="$fuzz_repo/fakebin:$PATH" \
  CALYX_TARGET_SCRATCH_ROOT="$(dirname "$fuzz_repo")/target-scratch" \
  CALYX_GATE_SIGNAL_GRACE_SECONDS=0.5 \
  FAKE_TMP_LOG="$fuzz_signal_stages" FAKE_DELETE_BASELINE=0 FAKE_FAIL_STAGE= \
  FAKE_BLOCK_STAGE=check FAKE_READY_FILE="$fuzz_ready" FAKE_STUBBORN_CHILD=1 \
  FAKE_BLOCK_SCOPE=fuzz \
  FAKE_REQUIRE_OWNED_MODE=1 \
  bash "$fuzz_repo/scripts/fuzz-gate.sh" >"$fuzz_signal_log" 2>&1 &
fuzz_signal_pid=$!
owned_pids="$fuzz_signal_pid"
fuzz_signal_pgid="$(ps -o pgid= -p "$fuzz_signal_pid" 2>/dev/null | tr -d ' ')"
[[ "$fuzz_signal_pgid" == "$fuzz_signal_pid" ]] && owned_pgids="$fuzz_signal_pgid"
gate_test_wait_ready "$fuzz_ready" || true
if [[ -f "$fuzz_ready" && "$fuzz_signal_pgid" == "$fuzz_signal_pid" ]]; then
  read -r fuzz_stage_pid_field fuzz_stage_pgid_field fuzz_cgroup_field fuzz_cgroup_id_field \
    fuzz_child_pid_field fuzz_child_pgid_field fuzz_child_cgroup_field <"$fuzz_ready"
  fuzz_stage_pid="${fuzz_stage_pid_field#pid=}"
  fuzz_stage_pgid="${fuzz_stage_pgid_field#pgid=}"
  fuzz_cgroup="${fuzz_cgroup_field#cgroup=}"
  fuzz_cgroup_id="${fuzz_cgroup_id_field#cgroup_id=}"
  fuzz_child_pid="${fuzz_child_pid_field#child=}"
  fuzz_child_pgid="${fuzz_child_pgid_field#child_pgid=}"
  fuzz_child_cgroup="${fuzz_child_cgroup_field#child_cgroup=}"
  [[ "$fuzz_stage_pid" == "$fuzz_stage_pgid" ]] \
    && owned_pgids="$owned_pgids $fuzz_stage_pgid $fuzz_child_pgid"
  [[ -n "$fuzz_child_pid" && "$fuzz_child_pgid" == "$fuzz_child_pid" \
    && "$fuzz_child_pgid" != "$fuzz_stage_pgid" && "$fuzz_child_cgroup" == "$fuzz_cgroup" ]] \
    && pass "fuzz setsid descendant escaped the PGID but stayed in the cgroup" \
    || bad "fuzz stubborn descendant cgroup containment is invalid"
  kill -TERM "$fuzz_signal_pid"
  fuzz_watchdog_marker="$work/fuzz/signal-watchdog-fired"
  (
    sleep 8
    printf 'fired\n' >"$fuzz_watchdog_marker"
    kill -TERM "$fuzz_signal_pid" 2>/dev/null || true
    kill -TERM -- "-$fuzz_stage_pgid" 2>/dev/null || true
    sleep 0.5
    kill -KILL "$fuzz_signal_pid" 2>/dev/null || true
    kill -KILL -- "-$fuzz_stage_pgid" 2>/dev/null || true
    [[ -n "$fuzz_child_pid" ]] && kill -KILL "$fuzz_child_pid" 2>/dev/null || true
  ) &
  fuzz_watchdog=$!
  set +e
  wait "$fuzz_signal_pid"; fuzz_signal_rc=$?
  set -e
  kill -KILL "$fuzz_watchdog" 2>/dev/null || true
  wait "$fuzz_watchdog" 2>/dev/null || true
  [[ ! -e "$fuzz_watchdog_marker" ]] \
    && pass "fuzz TERM completed before its regression watchdog" \
    || bad "fuzz TERM required watchdog rescue"
  fuzz_signal_tmp="$(sed -n 's/^fuzz-gate: TMPDIR=//p' "$fuzz_signal_log" | head -n1)"
  fuzz_signal_target="$(sed -n 's/^fuzz-gate: CARGO_TARGET_DIR=//p' "$fuzz_signal_log" | head -n1)"
  [[ "$fuzz_signal_rc" -eq 143 && ! -e "$fuzz_signal_tmp" && ! -e "$fuzz_signal_target" \
    && ! -e "/proc/$fuzz_child_pid" ]] \
    && gate_test_cgroup_absent "$fuzz_cgroup" "$fuzz_cgroup_id" \
    && grep -Fq 'forwarding TERM to run_slice=' "$fuzz_signal_log" \
    && pass "fuzz PID-only TERM drains the setsid cgroup and cleans both roots" \
    || bad "fuzz TERM rc=$fuzz_signal_rc or owned state remained"
else
  bad "fuzz signal case never reached a verified ready process group"
  [[ "$fuzz_signal_pgid" == "$fuzz_signal_pid" ]] \
    && gate_test_stop_group_bounded "$fuzz_signal_pgid" || kill -KILL "$fuzz_signal_pid" 2>/dev/null || true
  wait "$fuzz_signal_pid" 2>/dev/null || true
fi
owned_pids=""
owned_pgids=""

# shellcheck source=gate-wrapper-fuzz-extra-signals-regression.sh
source "$fuzz_extra_signals"

# shellcheck source=gate-wrapper-public-regression.sh
source "$public_scenarios"
# shellcheck source=gate-wrapper-final-cleanup-regression.sh
source "$final_cleanup_contracts"
# shellcheck source=gate-nested-worktree-regression.sh
source "$nested_worktree_contracts"
# shellcheck source=gate-cross-wrapper-lock-regression.sh
source "$cross_lock_contracts"
# shellcheck source=gate-wrapper-setup-publication-regression.sh
source "$setup_publication_contracts"

if [[ "$fail" -eq 0 ]]; then
  echo "ALL WRAPPER LIFECYCLE SCENARIOS AND CONTRACTS PASSED"
else
  echo "REGRESSION DETECTED" >&2
fi
exit "$fail"
