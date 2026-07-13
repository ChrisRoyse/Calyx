#!/usr/bin/env bash
# Source-only canonical-home cleanup coverage for nested worktrees (#1421).
[[ "${BASH_SOURCE[0]}" != "$0" ]] || {
  echo "source this module from gate-wrapper-tmpdir-regression.sh" >&2
  exit 64
}

echo
echo "=================================================================="
echo "NESTED WORKTREES - wrappers clean canonical CALYX_HOME roots"
echo "=================================================================="

nested_home="$work/nested-wrappers/calyx"
nested_fuzz="$nested_home/worktrees/fuzz/repo"
mkdir -p "$nested_fuzz/scripts" "$nested_fuzz/fuzz" "$nested_fuzz/home/.cargo" \
  "$nested_fuzz/global-tmp" "$nested_home/target-scratch"
chmod 755 "$nested_home" "$nested_home/worktrees" \
  "$nested_home/worktrees/fuzz" "$nested_fuzz"
chmod 700 "$nested_home/target-scratch"
cp "$helper_src" "$nested_fuzz/scripts/check-tmpdir.sh"
cp "$fuzz_src" "$nested_fuzz/scripts/fuzz-gate.sh"
cp "$runner_src" "$nested_fuzz/scripts/gate-stage-runner.sh"
cp "$runtime_src" "$nested_fuzz/scripts/gate-runtime-state.sh"
cp "$safe_helper_src" "$creator_src" "$nested_fuzz/scripts/"
cp "$cgroup_src" "$nested_fuzz/scripts/gate-cgroup-runtime.sh"
cp "$cgroup_scope_src" "$nested_fuzz/scripts/gate-cgroup-scope.sh"
cp "$ofd_query_src" "$nested_fuzz/scripts/gate_ofd_lock_query.py"
write_target_clean_stub "$nested_fuzz/scripts/cargo-target-clean.sh"
write_fake_cargo "$nested_fuzz"
: >"$nested_fuzz/home/.cargo/env"
printf 'export CALYX_HOME=%q\n' "$nested_home" >"$nested_fuzz/env.sh"
printf '[workspace]\nmembers=[]\n' >"$nested_fuzz/fuzz/Cargo.toml"
nested_fuzz_log="$work/nested-wrappers/fuzz.log"
nested_fuzz_stages="$work/nested-wrappers/fuzz-stages.log"
set +e
env -u CALYX_FSV_ROOT -u CALYX_GATE_RUN_SLICE -u CALYX_GATE_RUN_SLICE_OWNER_PID -u CALYX_GATE_TMPDIR -u CALYX_GATE_TMP_ROOT \
  -u CALYX_GATE_TMP_ROOT_IDENTITY -u CALYX_GATE_TMP_IDENTITY \
  -u CALYX_GATE_TARGET_ROOT -u CALYX_GATE_TARGET_ROOT_IDENTITY \
  -u CALYX_GATE_TARGET_DIR -u CALYX_GATE_TARGET_IDENTITY \
  -u CALYX_GATE_LOCK_PATH -u CALYX_GATE_LOCK_FD -u CARGO_TARGET_DIR -u TMPDIR \
  HOME="$nested_fuzz/home" PATH="$nested_fuzz/fakebin:$PATH" \
  FAKE_TMP_LOG="$nested_fuzz_stages" FAKE_REQUIRE_OWNED_MODE=1 \
  FAKE_TARGET_CLEAN_FAIL_ONCE_FILE= FAKE_TARGET_CLEAN_LOG= \
  FAKE_DELETE_BASELINE=0 FAKE_FAIL_STAGE= FAKE_BLOCK_STAGE= FAKE_READY_FILE= \
  timeout --signal=TERM --kill-after=2s 30s \
  bash "$nested_fuzz/scripts/fuzz-gate.sh" >"$nested_fuzz_log" 2>&1
nested_fuzz_rc=$?
set -e
nested_fuzz_tmp="$(sed -n 's/^fuzz-gate: TMPDIR=//p' "$nested_fuzz_log" | head -n1)"
nested_fuzz_target="$(sed -n 's/^fuzz-gate: CARGO_TARGET_DIR=//p' "$nested_fuzz_log" | head -n1)"
[[ "$nested_fuzz_rc" -eq 0 && "$nested_fuzz_tmp" == "$nested_home/tmp/"check.* \
  && "$nested_fuzz_target" == "$nested_home/target-scratch/"fuzz-gate.* \
  && ! -e "$nested_fuzz_tmp" && ! -L "$nested_fuzz_tmp" \
  && ! -e "$nested_fuzz_target" && ! -L "$nested_fuzz_target" ]] \
  && pass "nested-worktree fuzz cleans canonical temp and target identities" \
  || bad "nested-worktree fuzz rc=$nested_fuzz_rc leaked or used a repo-parent root"

nested_dev="$nested_home/worktrees/dev/repo"
nested_public="$nested_home/public-mirror"
setup_public_fixture "$nested_dev" "$nested_public"
printf 'export CALYX_HOME=%q\n' "$nested_home" >"$nested_dev/env.sh"
nested_public_log="$work/nested-wrappers/public.log"
nested_public_stages="$work/nested-wrappers/public-stages.log"
set +e
env -u CALYX_FSV_ROOT -u CALYX_GATE_RUN_SLICE -u CALYX_GATE_RUN_SLICE_OWNER_PID -u CALYX_GATE_TMPDIR -u CALYX_GATE_TMP_ROOT \
  -u CALYX_GATE_TMP_ROOT_IDENTITY -u CALYX_GATE_TMP_IDENTITY \
  -u CALYX_GATE_TARGET_ROOT -u CALYX_GATE_TARGET_ROOT_IDENTITY \
  -u CALYX_GATE_TARGET_DIR -u CALYX_GATE_TARGET_IDENTITY \
  -u CALYX_GATE_LOCK_PATH -u CALYX_GATE_LOCK_FD -u CARGO_TARGET_DIR -u TMPDIR \
  HOME="$nested_dev/home" PATH="$nested_dev/fakebin:$PATH" \
  CALYX_TMP_ROOT="$nested_dev/global-tmp" CALYX_TMP_MIN_FREE_INODE_PERCENT=0 \
  CALYX_TMP_MIN_FREE_BLOCK_PERCENT=0 FAKE_TMP_LOG="$nested_public_stages" \
  FAKE_DELETE_BASELINE=0 FAKE_FAIL_STAGE= FAKE_BLOCK_STAGE= FAKE_READY_FILE= \
  FAKE_TARGET_CLEAN_FAIL_ONCE_FILE= FAKE_TARGET_CLEAN_LOG= \
  FAKE_DELAY_CREATE_PREFIX= FAKE_CREATE_READY= FAKE_STUBBORN_CHILD=0 \
  FAKE_REQUIRE_OWNED_MODE=1 timeout --signal=TERM --kill-after=2s 45s \
  bash "$nested_dev/scripts/public-mirror-gate.sh" --public-dir "$nested_public" \
  >"$nested_public_log" 2>&1
nested_public_rc=$?
set -e
nested_public_tmp="$(sed -n 's/^public-mirror-gate: TMPDIR=//p' "$nested_public_log" | head -n1)"
nested_public_target="$(sed -n 's/^public-mirror-gate: target_dir=//p' "$nested_public_log" | head -n1)"
[[ "$nested_public_rc" -eq 0 && "$nested_public_tmp" == "$nested_home/tmp/"check.* \
  && "$nested_public_target" == "$nested_home/target-scratch/"public-mirror-gate.* \
  && ! -e "$nested_public_tmp" && ! -L "$nested_public_tmp" \
  && ! -e "$nested_public_target" && ! -L "$nested_public_target" \
  && -z "$(find "$nested_home/tmp" -mindepth 1 -maxdepth 1 -name 'check.*' -print -quit)" \
  && -z "$(find "$nested_home/target-scratch" -mindepth 1 -print -quit)" ]] \
  && pass "nested-worktree public gate cleans canonical temp and target identities" \
  || bad "nested-worktree public rc=$nested_public_rc leaked or used a dev-parent root"
