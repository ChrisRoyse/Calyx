#!/usr/bin/env bash
# Source-only target identity, mount, TOCTOU, and serialization mutations (#1421).
[[ "${BASH_SOURCE[0]}" != "$0" ]] || {
  echo "source this module from check-tmpdir-regression.sh" >&2
  exit 64
}

echo
echo "=================================================================="
echo "TARGET CLEANER CONTRACTS - identity, mount safety, and serialization"
echo "=================================================================="

clean_root="$work/target-clean/calyx"
clean_repo="$clean_root/repo"
clean_scratch="$clean_root/target-scratch"
mkdir -p "$clean_repo" "$clean_scratch"
chmod 755 "$clean_root" "$clean_repo"
chmod 700 "$clean_scratch"
export CALYX_HOME="$clean_root"
export CALYX_TARGET_SCRATCH_ROOT="$clean_scratch"

clean_target="$clean_scratch/check.identity"
mkdir -m 700 "$clean_target"
printf 'known-target-bytes\n' >"$clean_target/sentinel"
clean_identity="$(stat -c '%d:%i' -- "$clean_target")"
CALYX_TARGET_SCRATCH_ROOT="$clean_scratch" \
  env -u CALYX_GATE_LOCK_PATH -u CALYX_GATE_LOCK_FD \
  bash "$target_clean_src" --repo-root "$clean_repo" --target-dir "$clean_target" \
  --expected-target-identity "$clean_identity" \
  >"$work/target-clean/identity-success.log" 2>&1 \
  && [[ ! -e "$clean_target" \
    && -z "$(find "$clean_scratch" -mindepth 1 -print -quit)" ]] \
  && pass "identity-bound cleanup atomically quarantines and removes the expected target" \
  || bad "identity-bound target cleanup failed or left bytes"

snapshotless_target="$clean_scratch/check.snapshotless"
mkdir -m 700 "$snapshotless_target"
snapshotless_identity="$(stat -c '%d:%i' -- "$snapshotless_target")"
snapshotless_bin="$work/target-clean/snapshotless-bin"
snapshotless_flag="$work/target-clean/path-mktemp-invoked"
mkdir -p "$snapshotless_bin"
real_mktemp="$(command -v mktemp)"
cat >"$snapshotless_bin/mktemp" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
path="$("${REAL_MKTEMP:?}" "$@")"
mv -- "$path" "$path.parked"
printf 'unrelated-snapshot-replacement\n' >"$path"
printf 'invoked\n' >"${SNAPSHOTLESS_FLAG:?}"
printf '%s\n' "$path"
STUB
chmod +x "$snapshotless_bin/mktemp"
PATH="$snapshotless_bin:$PATH" REAL_MKTEMP="$real_mktemp" \
  SNAPSHOTLESS_FLAG="$snapshotless_flag" CALYX_TARGET_SCRATCH_ROOT="$clean_scratch" \
  env -u CALYX_GATE_LOCK_PATH -u CALYX_GATE_LOCK_FD \
  bash "$target_clean_src" --repo-root "$clean_repo" --target-dir "$snapshotless_target" \
  --expected-target-identity "$snapshotless_identity" \
  >"$work/target-clean/snapshotless.log" 2>&1
[[ ! -e "$snapshotless_target" && ! -e "$snapshotless_flag" \
  && -z "$(find "$clean_root/tmp" -maxdepth 1 -name '.cargo-target-enum.*' -print -quit)" ]] \
  && pass "target enumeration remains in bounded arrays with no pathname snapshot window" \
  || bad "target enumeration invoked pathname mktemp or left linked snapshot state"

missing_target="$clean_scratch/check.expected-absent"
set +e
env -u CALYX_GATE_LOCK_PATH -u CALYX_GATE_LOCK_FD \
  bash "$target_clean_src" --repo-root "$clean_repo" --target-dir "$missing_target" \
  --expected-target-identity 1:1 >"$work/target-clean/expected-absent.log" 2>&1
missing_target_rc=$?
set -e
[[ "$missing_target_rc" -ne 0 && ! -e "$missing_target" ]] \
  && grep -Fq 'expected target is absent' "$work/target-clean/expected-absent.log" \
  && pass "an absent identity-bound target fails closed without creation" \
  || bad "expected-but-absent target returned $missing_target_rc"

sub_target="$clean_scratch/check.substitution"
sub_original="$clean_scratch/check.substitution-original"
mkdir -m 700 "$sub_target"
printf 'original-target\n' >"$sub_target/sentinel"
sub_identity="$(stat -c '%d:%i' -- "$sub_target")"
mv -T -- "$sub_target" "$sub_original"
mkdir -m 700 "$sub_target"
printf 'replacement-target\n' >"$sub_target/sentinel"
sub_original_sha="$(sha256sum "$sub_original/sentinel" | awk '{print $1}')"
sub_replacement_sha="$(sha256sum "$sub_target/sentinel" | awk '{print $1}')"
set +e
CALYX_TARGET_SCRATCH_ROOT="$clean_scratch" \
  env -u CALYX_GATE_LOCK_PATH -u CALYX_GATE_LOCK_FD \
  bash "$target_clean_src" --repo-root "$clean_repo" --target-dir "$sub_target" \
  --expected-target-identity "$sub_identity" \
  >"$work/target-clean/identity-substitution.log" 2>&1
sub_rc=$?
set -e
[[ "$sub_rc" -ne 0 \
  && "$(sha256sum "$sub_original/sentinel" | awk '{print $1}')" == "$sub_original_sha" \
  && "$(sha256sum "$sub_target/sentinel" | awk '{print $1}')" == "$sub_replacement_sha" ]] \
  && pass "pre-clean target substitution is rejected without deleting either inode" \
  || bad "pre-clean substitution rc=$sub_rc changed target bytes"
rm -rf -- "$sub_original" "$sub_target"

mount_target="$clean_scratch/check.mount"
mkdir -m 700 "$mount_target"
printf 'mount-sentinel\n' >"$mount_target/sentinel"
mount_identity="$(stat -c '%d:%i' -- "$mount_target")"
mount_sha="$(sha256sum "$mount_target/sentinel" | awk '{print $1}')"
mount_bin="$work/target-clean/mount-bin"
mkdir -p "$mount_bin"
cat >"$mount_bin/findmnt" <<'STUB'
#!/usr/bin/env bash
printf '%s\n' "${FAKE_MOUNT_TARGET:?}"
STUB
chmod +x "$mount_bin/findmnt"
set +e
PATH="$mount_bin:$PATH" FAKE_MOUNT_TARGET="$mount_target/bind" \
  CALYX_TARGET_SCRATCH_ROOT="$clean_scratch" \
  env -u CALYX_GATE_LOCK_PATH -u CALYX_GATE_LOCK_FD \
  bash "$target_clean_src" --repo-root "$clean_repo" --target-dir "$mount_target" \
  --expected-target-identity "$mount_identity" \
  >"$work/target-clean/mount.log" 2>&1
mount_clean_rc=$?
set -e
[[ "$mount_clean_rc" -ne 0 \
  && "$(sha256sum "$mount_target/sentinel" | awk '{print $1}')" == "$mount_sha" ]] \
  && grep -Fq 'mountpoint at or below Cargo target' "$work/target-clean/mount.log" \
  && pass "target cleanup refuses nested mounts without changing bytes" \
  || bad "target mount mutation rc=$mount_clean_rc was accepted or destructive"
rm -rf -- "$mount_target"

mounted_parent="$clean_repo/mounted-parent"
mounted_nested_target="$mounted_parent/nested/target"
mkdir -p "$mounted_nested_target"
printf 'mounted-parent-sentinel\n' >"$mounted_nested_target/sentinel"
mounted_parent_sha="$(sha256sum "$mounted_nested_target/sentinel" | awk '{print $1}')"
cat >"$mount_bin/findmnt" <<'STUB'
#!/usr/bin/env bash
printf '%s\n' "${FAKE_MOUNT_TARGET:?}"
STUB
chmod +x "$mount_bin/findmnt"
set +e
PATH="$mount_bin:$PATH" FAKE_MOUNT_TARGET="$mounted_parent" \
  env -u CALYX_GATE_LOCK_PATH -u CALYX_GATE_LOCK_FD \
  bash "$target_clean_src" --repo-root "$clean_repo" \
  --target-dir "$clean_scratch/absent" --also-repo-targets \
  >"$work/target-clean/mounted-parent.log" 2>&1
mounted_parent_rc=$?
set -e
[[ "$mounted_parent_rc" -ne 0 \
  && "$(sha256sum "$mounted_nested_target/sentinel" | awk '{print $1}')" == "$mounted_parent_sha" ]] \
  && grep -Fq 'mountpoint at or below repository' "$work/target-clean/mounted-parent.log" \
  && pass "repo sweep refuses a mount above a nested target without traversing it" \
  || bad "mounted parent rc=$mounted_parent_rc was traversed or changed"
rm -rf -- "$mounted_parent"

findmnt_fail_target="$clean_scratch/check.findmnt-failure"
mkdir -m 700 "$findmnt_fail_target"
printf 'findmnt-failure-sentinel\n' >"$findmnt_fail_target/sentinel"
findmnt_fail_sha="$(sha256sum "$findmnt_fail_target/sentinel" | awk '{print $1}')"
cat >"$mount_bin/findmnt" <<'STUB'
#!/usr/bin/env bash
exit 66
STUB
chmod +x "$mount_bin/findmnt"
set +e
PATH="$mount_bin:$PATH" env -u CALYX_GATE_LOCK_PATH -u CALYX_GATE_LOCK_FD \
  bash "$target_clean_src" --repo-root "$clean_repo" \
  --target-dir "$findmnt_fail_target" >"$work/target-clean/findmnt-failure.log" 2>&1
findmnt_fail_rc=$?
set -e
[[ "$findmnt_fail_rc" -ne 0 \
  && "$(sha256sum "$findmnt_fail_target/sentinel" | awk '{print $1}')" == "$findmnt_fail_sha" ]] \
  && grep -Fq 'findmnt failed while proving cleanup boundaries' "$work/target-clean/findmnt-failure.log" \
  && pass "findmnt failure preserves the exact target and fails closed" \
  || bad "findmnt failure rc=$findmnt_fail_rc was accepted or destructive"
rm -rf -- "$findmnt_fail_target"

enum_target="$clean_repo/enumeration/target"
mkdir -p "$enum_target"
chmod 700 "$enum_target"
printf 'enumeration-sentinel\n' >"$enum_target/sentinel"
enum_sha="$(sha256sum "$enum_target/sentinel" | awk '{print $1}')"
enum_bin="$work/target-clean/enum-bin"
mkdir -p "$enum_bin"
cat >"$enum_bin/find" <<'STUB'
#!/usr/bin/env bash
printf '%s\0' "${FAKE_FIND_RECORD:?}"
exit 66
STUB
chmod +x "$enum_bin/find"
set +e
PATH="$enum_bin:$PATH" FAKE_FIND_RECORD="$enum_target" \
  env -u CALYX_GATE_LOCK_PATH -u CALYX_GATE_LOCK_FD \
  bash "$target_clean_src" --repo-root "$clean_repo" \
  --target-dir "$clean_scratch/absent-enumeration" --also-repo-targets \
  >"$work/target-clean/enumeration-failure.log" 2>&1
enum_rc=$?
set -e
[[ "$enum_rc" -eq 66 && -f "$enum_target/sentinel" \
  && "$(sha256sum "$enum_target/sentinel" | awk '{print $1}')" == "$enum_sha" ]] \
  && grep -Fq 'enumeration failed: label=repo-targets status=66' \
    "$work/target-clean/enumeration-failure.log" \
  && pass "repo target enumeration producer failure preserves every emitted candidate" \
  || bad "repo target enumeration failure rc=$enum_rc was fail-open or destructive"
rm -rf -- "$clean_repo/enumeration"

classification_target="$(dirname "$clean_repo")/target-classification-failure"
mkdir -m 700 "$classification_target"
printf 'classification-sentinel\n' >"$classification_target/sentinel"
classification_sha="$(sha256sum "$classification_target/sentinel" | awk '{print $1}')"
set +e
PATH="$enum_bin:$PATH" FAKE_FIND_RECORD="$classification_target/sentinel" \
  env -u CALYX_GATE_LOCK_PATH -u CALYX_GATE_LOCK_FD \
  bash "$target_clean_src" --repo-root "$clean_repo" \
  --target-dir "$classification_target" \
  >"$work/target-clean/classification-failure.log" 2>&1
classification_rc=$?
set -e
[[ "$classification_rc" -eq 66 \
  && "$(sha256sum "$classification_target/sentinel" | awk '{print $1}')" == "$classification_sha" ]] \
  && grep -Fq 'enumeration failed: label=cargo-root-classification status=66' \
    "$work/target-clean/classification-failure.log" \
  && pass "legacy target classification fails closed on producer failure" \
  || bad "legacy classification failure rc=$classification_rc was destructive"
rm -rf -- "$classification_target"

enum_limit_target="$clean_repo/enumeration-limit/target"
mkdir -p "$enum_limit_target"
printf 'enumeration-limit-sentinel\n' >"$enum_limit_target/sentinel"
enum_limit_sha="$(sha256sum "$enum_limit_target/sentinel" | awk '{print $1}')"
cat >"$enum_bin/find" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
for ((index = 0; index < 16385; index++)); do
  printf '%s\0' "${FAKE_FIND_RECORD:?}"
done
STUB
chmod +x "$enum_bin/find"
set +e
PATH="$enum_bin:$PATH" FAKE_FIND_RECORD="$enum_limit_target" \
  env -u CALYX_GATE_LOCK_PATH -u CALYX_GATE_LOCK_FD \
  bash "$target_clean_src" --repo-root "$clean_repo" \
  --target-dir "$clean_scratch/absent-enumeration-limit" --also-repo-targets \
  >"$work/target-clean/enumeration-limit.log" 2>&1
enum_limit_rc=$?
set -e
[[ "$enum_limit_rc" -ne 0 \
  && "$(sha256sum "$enum_limit_target/sentinel" | awk '{print $1}')" == "$enum_limit_sha" ]] \
  && grep -Fq 'enumeration limit exceeded: label=repo-targets' \
    "$work/target-clean/enumeration-limit.log" \
  && pass "repo target enumeration is capped before sort and preserves candidate bytes" \
  || bad "repo target enumeration limit rc=$enum_limit_rc was late or destructive"
rm -rf -- "$clean_repo/enumeration-limit"

scratch_enum_bin="$work/target-clean/scratch-enum-bin"
scratch_real_find="$(command -v find)"
mkdir -p "$scratch_enum_bin"
cat >"$scratch_enum_bin/find" <<'STUB'
#!/usr/bin/env bash
if [[ "${1:-}" == "${FAKE_FAIL_ROOT:?}" ]]; then exit 66; fi
exec "${REAL_FIND:?}" "$@"
STUB
chmod +x "$scratch_enum_bin/find"
set +e
PATH="$scratch_enum_bin:$PATH" FAKE_FAIL_ROOT="$clean_scratch" REAL_FIND="$scratch_real_find" \
  env -u CALYX_GATE_LOCK_PATH -u CALYX_GATE_LOCK_FD \
  bash "$target_clean_src" --repo-root "$clean_repo" \
  --target-dir "$clean_scratch/absent-scratch-final" \
  >"$work/target-clean/scratch-enumeration-failure.log" 2>&1
scratch_enum_rc=$?
set -e
[[ "$scratch_enum_rc" -eq 66 ]] \
  && grep -Fq 'enumeration failed: label=scratch-final status=66' \
    "$work/target-clean/scratch-enumeration-failure.log" \
  && pass "final scratch enumeration cannot report false empty on producer failure" \
  || bad "final scratch enumeration failure rc=$scratch_enum_rc was accepted"

race_target="$clean_scratch/check.race"
mkdir -m 700 "$race_target"
printf 'race-original\n' >"$race_target/sentinel"
race_identity="$(stat -c '%d:%i' -- "$race_target")"
race_bin="$work/target-clean/race-bin"
race_flag="$work/target-clean/path-mv-called"
mkdir -p "$race_bin"
cat >"$race_bin/mv" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
: >"${MUTATE_FLAG:?}"
exec /usr/bin/mv "$@"
STUB
chmod +x "$race_bin/mv"
set +e
PATH="$race_bin:$PATH" MUTATE_FLAG="$race_flag" CALYX_TARGET_SCRATCH_ROOT="$clean_scratch" \
  env -u CALYX_GATE_LOCK_PATH -u CALYX_GATE_LOCK_FD \
  bash "$target_clean_src" --repo-root "$clean_repo" --target-dir "$race_target" \
  --expected-target-identity "$race_identity" \
  >"$work/target-clean/race.log" 2>&1
race_rc=$?
set -e
[[ "$race_rc" -eq 0 && ! -e "$race_flag" && ! -e "$race_target" ]] \
  && pass "target cleanup uses FD-anchored renameat2 instead of a PATH-resolved mv" \
  || bad "target cleanup rc=$race_rc invoked pathname mv or retained bytes"

valuable_target="$clean_root/valuable-target"
mkdir -m 700 "$valuable_target"
printf 'valuable-target-sentinel\n' >"$valuable_target/sentinel"
valuable_sha="$(sha256sum "$valuable_target/sentinel" | awk '{print $1}')"
ln -s "$valuable_target" "$clean_repo/target"
set +e
env -u CALYX_GATE_LOCK_PATH -u CALYX_GATE_LOCK_FD \
  bash "$target_clean_src" --repo-root "$clean_repo" --target-dir "$clean_repo/target" \
  >"$work/target-clean/workspace-symlink.log" 2>&1
workspace_symlink_rc=$?
set -e
[[ "$workspace_symlink_rc" -ne 0 && -L "$clean_repo/target" \
  && "$(sha256sum "$valuable_target/sentinel" | awk '{print $1}')" == "$valuable_sha" ]] \
  && pass "workspace target symlink cannot redefine the authorized removal root" \
  || bad "workspace target symlink rc=$workspace_symlink_rc changed its referent"
rm -- "$clean_repo/target"
rm -rf -- "$valuable_target"

explicit_valuable="$clean_scratch/check.explicit-valuable"
explicit_link="$clean_scratch/check.explicit-link"
mkdir -m 700 "$explicit_valuable"
printf 'explicit-valuable-sentinel\n' >"$explicit_valuable/sentinel"
explicit_valuable_sha="$(sha256sum "$explicit_valuable/sentinel" | awk '{print $1}')"
ln -s -- "$explicit_valuable" "$explicit_link"
set +e
env -u CALYX_GATE_LOCK_PATH -u CALYX_GATE_LOCK_FD \
  bash "$target_clean_src" --repo-root "$clean_repo" --target-dir "$explicit_link" \
  >"$work/target-clean/explicit-symlink.log" 2>&1
explicit_symlink_rc=$?
set -e
[[ "$explicit_symlink_rc" -ne 0 && -L "$explicit_link" && -d "$explicit_valuable" \
  && "$(sha256sum "$explicit_valuable/sentinel" | awk '{print $1}')" == "$explicit_valuable_sha" ]] \
  && grep -Fq 'refusing target with symlink component' "$work/target-clean/explicit-symlink.log" \
  && pass "an authorized scratch symlink cannot redirect cleanup to its referent" \
  || bad "explicit target symlink rc=$explicit_symlink_rc changed its referent"
rm -- "$explicit_link"
rm -rf -- "$explicit_valuable"

scratch_alias="$clean_root/target-issue1421-alias"
ln -s -- "$clean_scratch" "$scratch_alias"
set +e
env -u CALYX_GATE_LOCK_PATH -u CALYX_GATE_LOCK_FD \
  bash "$target_clean_src" --repo-root "$clean_repo" \
  --target-dir "$clean_scratch/check.alias-absent" \
  >"$work/target-clean/scratch-alias.log" 2>&1
scratch_alias_rc=$?
set -e
[[ "$scratch_alias_rc" -ne 0 && -L "$scratch_alias" \
  && "$(readlink "$scratch_alias")" == "$clean_scratch" ]] \
  && grep -Fq 'Calyx-home target-* path remains' "$work/target-clean/scratch-alias.log" \
  && pass "a target-* symlink alias cannot be exempted as canonical target-scratch" \
  || bad "target-scratch alias rc=$scratch_alias_rc was falsely reported absent"
rm -- "$scratch_alias"

lock_sentinel="$clean_root/lock-sentinel"
printf 'lock-sentinel-bytes\n' >"$lock_sentinel"
lock_sentinel_sha="$(sha256sum "$lock_sentinel" | awk '{print $1}')"
mkdir -p "$clean_root/tmp"
rm -f -- "$clean_root/tmp/.cargo-gate.lock"
ln -s "$lock_sentinel" "$clean_root/tmp/.cargo-gate.lock"
set +e
env -u CALYX_GATE_LOCK_PATH -u CALYX_GATE_LOCK_FD \
  bash "$target_clean_src" --repo-root "$clean_repo" --target-dir "$clean_scratch/absent" \
  >"$work/target-clean/lock-symlink.log" 2>&1
lock_symlink_rc=$?
set -e
[[ "$lock_symlink_rc" -ne 0 && -L "$clean_root/tmp/.cargo-gate.lock" \
  && "$(sha256sum "$lock_sentinel" | awk '{print $1}')" == "$lock_sentinel_sha" ]] \
  && pass "canonical lock symlink is rejected without opening or truncating its referent" \
  || bad "lock symlink rc=$lock_symlink_rc changed its referent"
rm -- "$clean_root/tmp/.cargo-gate.lock"
(umask 077; : >"$clean_root/tmp/.cargo-gate.lock")
rm -- "$lock_sentinel"

serial_target="$clean_scratch/check.serialized"
mkdir -m 700 "$serial_target"
printf 'serialized-target\n' >"$serial_target/sentinel"
serial_identity="$(stat -c '%d:%i' -- "$serial_target")"
serial_lock="$clean_root/tmp/.cargo-gate.lock"
mkdir -p "$clean_root/tmp"
chmod 700 "$clean_root/tmp"
(umask 077; : >"$serial_lock")
exec {serial_fd}>>"$serial_lock"
flock --fcntl --exclusive "$serial_fd"
serial_bin="$work/target-clean/serial-bin"
serial_ready="$work/target-clean/serial-ready"
mkdir -p "$serial_bin"
cat >"$serial_bin/flock" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
printf 'waiting fd=%s\n' "${1:-missing}" >"${SERIAL_READY:?}"
exec /usr/bin/flock "$@"
STUB
chmod +x "$serial_bin/flock"
(
  eval "exec ${serial_fd}>&-"
  PATH="$serial_bin:$PATH" SERIAL_READY="$serial_ready" \
    timeout --signal=TERM --kill-after=1s 5s \
    env -u CALYX_GATE_LOCK_PATH -u CALYX_GATE_LOCK_FD \
    bash "$target_clean_src" --repo-root "$clean_repo" --target-dir "$serial_target" \
    --expected-target-identity "$serial_identity" \
    >"$work/target-clean/serialized.log" 2>&1
) &
serial_clean_pid=$!
owned_gate_pid="$serial_clean_pid"
gate_test_wait_ready "$serial_ready" || true
[[ -s "$serial_ready" && -d "$serial_target" && -e "/proc/$serial_clean_pid" ]] \
  && pass "a competing cleanup waits while the canonical gate lock is held" \
  || bad "competing cleanup bypassed the held gate lock"
eval "exec ${serial_fd}>&-"
set +e
wait "$serial_clean_pid"
serial_rc=$?
set -e
owned_gate_pid=""
[[ "$serial_rc" -eq 0 && ! -e "$serial_target" \
  && -z "$(find "$clean_scratch" -mindepth 1 -print -quit)" ]] \
  && pass "queued cleanup proceeds only after lock release and leaves scratch empty" \
  || bad "serialized cleanup rc=$serial_rc or target remained"
