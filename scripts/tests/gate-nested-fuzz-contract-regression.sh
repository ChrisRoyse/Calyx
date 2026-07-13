#!/usr/bin/env bash
# Source-only outer TMPDIR/target/lock validation mutations for fuzz-gate.
[[ "${BASH_SOURCE[0]}" != "$0" ]] || {
  echo "source this module from gate-wrapper-tmpdir-regression.sh" >&2
  exit 64
}

echo
echo "=================================================================="
echo "NESTED FUZZ CONTRACTS - complete identities and inherited lock"
echo "=================================================================="

run_nested_mutation() {
  local key="$1" diagnostic="$2"
  shift 2
  local log="$work/fuzz/nested-$key.log" stages="$work/fuzz/nested-$key-stages.log"
  set +e
  env -u CALYX_FSV_ROOT -u CALYX_GATE_RUN_SLICE -u CALYX_GATE_RUN_SLICE_OWNER_PID \
    -u CALYX_GATE_RUN_SLICE_OWNER_STARTTIME \
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
    FAKE_TMP_LOG="$stages" FAKE_DELETE_BASELINE=0 FAKE_FAIL_STAGE= \
    FAKE_BLOCK_STAGE= FAKE_READY_FILE= "$@" \
    timeout --signal=TERM --kill-after=2s 20s setsid \
      bash "$fuzz_repo/scripts/fuzz-gate.sh" >"$log" 2>&1
  nested_mutation_rc=$?
  set -e
  if [[ "$nested_mutation_rc" -ne 0 && ! -s "$stages" \
    && "$(sha256sum "$outer_target_root/outer-target/sentinel" | awk '{print $1}')" == "$outer_target_sha" ]] \
    && grep -Fq "$diagnostic" "$log"; then
    pass "nested fuzz rejects $key before Cargo with caller target preserved"
  else
    bad "nested fuzz mutation $key rc=$nested_mutation_rc reached Cargo or missed '$diagnostic'"
    sed -n '1,30p' "$log" | sed 's/^/    nested: /'
  fi
}

run_nested_mutation missing-tmp-identity \
  'outer gate TMPDIR/root marker is incomplete or inconsistent' \
  CALYX_GATE_TMP_IDENTITY=
run_nested_mutation missing-tmp-root-identity \
  'outer gate TMPDIR/root marker is incomplete or inconsistent' \
  CALYX_GATE_TMP_ROOT_IDENTITY=
run_nested_mutation wrong-tmp-identity 'run dir identity changed' \
  CALYX_GATE_TMP_IDENTITY=1:1
run_nested_mutation wrong-tmp-root-identity 'tmp root identity changed' \
  CALYX_GATE_TMP_ROOT_IDENTITY=1:1
run_nested_mutation missing-target-identity \
  'outer gate Cargo target marker is incomplete or inconsistent' \
  CALYX_GATE_TARGET_IDENTITY=
run_nested_mutation missing-target-root-identity \
  'outer gate Cargo target marker is incomplete or inconsistent' \
  CALYX_GATE_TARGET_ROOT_IDENTITY=
run_nested_mutation wrong-target-identity 'Cargo target dir identity changed' \
  CALYX_GATE_TARGET_IDENTITY=1:1
run_nested_mutation wrong-target-root-identity 'Cargo target root identity changed' \
  CALYX_GATE_TARGET_ROOT_IDENTITY=1:1
target_mismatch="$outer_target_root/mismatched-target"
mkdir -m 700 "$target_mismatch"
printf 'mismatched-target-sentinel\n' >"$target_mismatch/sentinel"
target_mismatch_sha="$(sha256sum "$target_mismatch/sentinel" | awk '{print $1}')"
run_nested_mutation mismatched-target-vars \
  'outer gate Cargo target marker is incomplete or inconsistent' \
  CARGO_TARGET_DIR="$target_mismatch"
[[ "$(sha256sum "$target_mismatch/sentinel" | awk '{print $1}')" == "$target_mismatch_sha" ]] \
  && pass "nested fuzz target-variable mismatch preserves both candidate trees" \
  || bad "nested fuzz target-variable mismatch changed the alternate tree"
rm -rf -- "$target_mismatch"
run_nested_mutation missing-lock 'outer gate Cargo lock marker is incomplete' \
  CALYX_GATE_LOCK_FD=

exec {unlocked_fd}>>"$outer_lock_path"
run_nested_mutation separate-lock-description 'not the locked open description' \
  CALYX_GATE_LOCK_FD="$unlocked_fd"
eval "exec ${unlocked_fd}>&-"

eval "exec ${outer_lock_fd}>&-"
exec {unheld_fd}>>"$outer_lock_path"
run_nested_mutation globally-unheld-lock 'lock is not held' \
  CALYX_GATE_LOCK_FD="$unheld_fd"
eval "exec ${unheld_fd}>&-"
exec {outer_lock_fd}>>"$outer_lock_path"
flock --fcntl --exclusive "$outer_lock_fd"

other_lock="$outer_tmp_root/.nested-other.lock"
(umask 077; : >"$other_lock")
exec {other_lock_fd}>>"$other_lock"
flock --fcntl --exclusive "$other_lock_fd"
run_nested_mutation wrong-lock-fd 'lock path mismatch' \
  CALYX_GATE_LOCK_FD="$other_lock_fd"
eval "exec ${other_lock_fd}>&-"
rm -- "$other_lock"

outside_parent="$work/fuzz/outside"
mkdir -m 700 "$outside_parent"
outside_target="$outside_parent/target"
mkdir -m 700 "$outside_target"
printf 'outside-target-sentinel\n' >"$outside_target/sentinel"
outside_target_identity="$(stat -c '%d:%i' -- "$outside_target")"
outside_target_sha="$(sha256sum "$outside_target/sentinel" | awk '{print $1}')"
run_nested_mutation outside-target 'is not a direct child' \
  CARGO_TARGET_DIR="$outside_target" CALYX_GATE_TARGET_DIR="$outside_target" \
  CALYX_GATE_TARGET_IDENTITY="$outside_target_identity"
[[ "$(sha256sum "$outside_target/sentinel" | awk '{print $1}')" == "$outside_target_sha" ]] \
  || bad "outside nested target sentinel changed"
rm -rf -- "$outside_parent"

# The canonical lock path itself must remain a regular file. Preserve the
# original inode, mutate only its directory entry, then restore and reacquire.
eval "exec ${outer_lock_fd}>&-"
outer_lock_original="$outer_tmp_root/.cargo-gate.lock.original"
mv -T -- "$outer_lock_path" "$outer_lock_original"
ln -s "$outer_lock_original" "$outer_lock_path"
exec {symlink_lock_fd}>>"$outer_lock_path"
flock --fcntl --exclusive "$symlink_lock_fd"
outer_lock_fd="$symlink_lock_fd"
run_nested_mutation symlink-lock 'gate lock is not a regular file' \
  CALYX_GATE_LOCK_FD="$symlink_lock_fd"
eval "exec ${symlink_lock_fd}>&-"
rm -- "$outer_lock_path"
mv -T -- "$outer_lock_original" "$outer_lock_path"
exec {outer_lock_fd}>>"$outer_lock_path"
flock --fcntl --exclusive "$outer_lock_fd"
