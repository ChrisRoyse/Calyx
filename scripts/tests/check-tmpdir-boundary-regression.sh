#!/usr/bin/env bash
# Source-only path, identity, permission, mount, and create-signal scenarios.
[[ "${BASH_SOURCE[0]}" != "$0" ]] || { echo "source this module from check-tmpdir-regression.sh" >&2; exit 64; }

echo
echo "=================================================================="
echo "SCENARIO 6 - outside and symlink cleanup targets are rejected"
echo "=================================================================="
root5="$work/boundary/calyx/tmp"
mkdir -p "$(dirname "$root5")" "$work/boundary/outside/check.keep"
chmod 755 "$(dirname "$root5")"
printf 'preserve-me\n' >"$work/boundary/outside/check.keep/sentinel"
sentinel_before="$(sha256sum "$work/boundary/outside/check.keep/sentinel" | awk '{print $1}')"
created5="$(bash "$tmpdir_src" create "$root5" 2>"$work/boundary/create.log")"
root5_identity="$(stat -c '%d:%i' -- "$root5")"
created5_identity="$(stat -c '%d:%i' -- "$created5")"
set +e
bash "$tmpdir_src" cleanup "$root5" "$work/boundary/outside/check.keep" >"$work/boundary/outside.log" 2>&1
outside_rc=$?
set -e
sentinel_after="$(sha256sum "$work/boundary/outside/check.keep/sentinel" | awk '{print $1}')"
[[ "$outside_rc" -ne 0 && "$sentinel_before" == "$sentinel_after" ]] \
  && pass "outside direct-child lookalike rejected and preserved" \
  || bad "outside path rc=$outside_rc sentinel changed"
ln -s "$work/boundary/outside/check.keep" "$root5/check.symlink"
set +e
bash "$tmpdir_src" cleanup "$root5" "$root5/check.symlink" >"$work/boundary/symlink.log" 2>&1
symlink_rc=$?
set -e
[[ "$symlink_rc" -ne 0 && -L "$root5/check.symlink" && -f "$work/boundary/outside/check.keep/sentinel" ]] \
  && pass "in-root symlink rejected without touching its target" \
  || bad "symlink boundary was not preserved"
rm -f -- "$root5/check.symlink"
noncheck5="$root5/manual.keep"
mkdir -p "$noncheck5"
printf 'non-check sentinel\n' >"$noncheck5/sentinel"
set +e
bash "$tmpdir_src" cleanup "$root5" "$noncheck5" >"$work/boundary/noncheck.log" 2>&1
noncheck_rc=$?
set -e
[[ "$noncheck_rc" -ne 0 && "$(tr -d '\n' <"$noncheck5/sentinel")" == "non-check sentinel" ]] \
  && pass "in-root non-check child rejected and preserved" \
  || bad "non-check basename restriction was bypassed"
rm -rf -- "$noncheck5"
printf 'permission-sentinel\n' >"$created5/sentinel"
permission_sha="$(sha256sum "$created5/sentinel" | awk '{print $1}')"
boundary_parent="$(dirname "$root5")"
for unsafe_mode in 770 707; do
  chmod "$unsafe_mode" "$root5"
  set +e
  bash "$tmpdir_src" cleanup "$root5" "$created5" >"$work/boundary/root-mode-$unsafe_mode.log" 2>&1
  root_mode_rc=$?
  set -e
  chmod 700 "$root5"
  [[ "$root_mode_rc" -ne 0 && "$(sha256sum "$created5/sentinel" | awk '{print $1}')" == "$permission_sha" ]] \
    && pass "tmp root mode $unsafe_mode is rejected with bytes preserved" \
    || bad "tmp root mode $unsafe_mode mutation was accepted"

  chmod "$unsafe_mode" "$created5"
  set +e
  bash "$tmpdir_src" validate "$root5" "$created5" >"$work/boundary/run-mode-$unsafe_mode.log" 2>&1
  run_mode_rc=$?
  set -e
  chmod 700 "$created5"
  [[ "$run_mode_rc" -ne 0 && "$(sha256sum "$created5/sentinel" | awk '{print $1}')" == "$permission_sha" ]] \
    && pass "run-dir mode $unsafe_mode is rejected with bytes preserved" \
    || bad "run-dir mode $unsafe_mode mutation was accepted"

  chmod "$unsafe_mode" "$boundary_parent"
  set +e
  bash "$tmpdir_src" cleanup "$root5" "$created5" >"$work/boundary/parent-mode-$unsafe_mode.log" 2>&1
  parent_mode_rc=$?
  set -e
  chmod 755 "$boundary_parent"
  [[ "$parent_mode_rc" -ne 0 && "$(sha256sum "$created5/sentinel" | awk '{print $1}')" == "$permission_sha" ]] \
    && pass "root-parent mode $unsafe_mode is rejected with bytes preserved" \
    || bad "root-parent mode $unsafe_mode mutation was accepted"
done

echo
echo "=================================================================="
echo "NESTED WORKTREE - cleanup stays bound to canonical CALYX_HOME"
echo "=================================================================="
nested_home="$work/nested-worktree/calyx"
nested_repo="$nested_home/worktrees/issue1421/repo"
mkdir -p "$nested_repo"
setup_repo "$nested_repo" "$nested_home"
nested_log="$work/nested-worktree/gate.log"
nested_stages="$work/nested-worktree/stages.log"
run_gate "$nested_repo" "$nested_log" "$nested_stages" "" 0 "" "$nested_home"
nested_run="$(gate_tmp_from_log "$nested_log")"
nested_target="$(gate_target_from_log "$nested_log")"
[[ "$RUN_RC" -eq 0 && "$nested_run" == "$nested_home/tmp/"check.* \
  && "$nested_target" == "$nested_home/target-scratch/"check.* \
  && ! -e "$nested_run" && ! -L "$nested_run" \
  && ! -e "$nested_target" && ! -L "$nested_target" \
  && -z "$(find "$nested_home/tmp" -mindepth 1 -maxdepth 1 -name 'check.*' -print -quit)" \
  && -z "$(find "$nested_home/target-scratch" -mindepth 1 -print -quit)" ]] \
  && pass "nested-worktree check cleans its canonical temp and target identities" \
  || {
    bad "nested-worktree check rc=$RUN_RC leaked or used a repo-parent root"
    sed -n '1,45p' "$nested_log" | sed 's/^/    nested: /'
  }

mkdir -p "$work/boundary/fakebin"
real_stat="$(command -v stat)"
cat >"$work/boundary/fakebin/stat" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
last="${!#}"
if [[ "${1:-}" == "-c" && "${2:-}" == "%u" \
  && "$last" == "${FAKE_FOREIGN_PATH:-__no_foreign_path__}" ]]; then
  echo "$(( $(id -u) + 1 ))"
  exit 0
fi
exec "${REAL_STAT_PATH:?}" "$@"
STUB
chmod +x "$work/boundary/fakebin/stat"
set +e
PATH="$work/boundary/fakebin:$PATH" REAL_STAT_PATH="$real_stat" FAKE_FOREIGN_PATH="$root5" \
  bash "$tmpdir_src" cleanup "$root5" "$created5" >"$work/boundary/foreign-owner.log" 2>&1
foreign_owner_rc=$?
set -e
[[ "$foreign_owner_rc" -ne 0 && "$(sha256sum "$created5/sentinel" | awk '{print $1}')" == "$permission_sha" ]] \
  && grep -Fq 'owner uid' "$work/boundary/foreign-owner.log" \
  && pass "foreign-owner readback is rejected with bytes preserved" \
  || bad "foreign-owner mutation was accepted or changed bytes"
set +e
PATH="$work/boundary/fakebin:$PATH" REAL_STAT_PATH="$real_stat" FAKE_FOREIGN_PATH="$boundary_parent" \
  bash "$tmpdir_src" cleanup "$root5" "$created5" >"$work/boundary/foreign-parent.log" 2>&1
foreign_parent_rc=$?
set -e
[[ "$foreign_parent_rc" -ne 0 && "$(sha256sum "$created5/sentinel" | awk '{print $1}')" == "$permission_sha" ]] \
  && grep -Fq 'owner uid' "$work/boundary/foreign-parent.log" \
  && pass "foreign-owner parent readback is rejected with bytes preserved" \
  || bad "foreign-owner parent mutation was accepted or changed bytes"
set +e
PATH="$work/boundary/fakebin:$PATH" REAL_STAT_PATH="$real_stat" FAKE_FOREIGN_PATH="$created5" \
  bash "$tmpdir_src" cleanup "$root5" "$created5" >"$work/boundary/foreign-run.log" 2>&1
foreign_run_rc=$?
set -e
[[ "$foreign_run_rc" -ne 0 && "$(sha256sum "$created5/sentinel" | awk '{print $1}')" == "$permission_sha" ]] \
  && grep -Fq 'owner uid' "$work/boundary/foreign-run.log" \
  && pass "foreign-owner run dir is rejected with bytes preserved" \
  || bad "foreign-owner run-dir mutation was accepted or changed bytes"

cat >"$work/boundary/fakebin/findmnt" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "${FAKE_MOUNT_TARGET:?}"
STUB
chmod +x "$work/boundary/fakebin/findmnt"
set +e
PATH="$work/boundary/fakebin:$PATH" REAL_STAT_PATH="$real_stat" \
  FAKE_MOUNT_TARGET="$created5/check.bind" \
  bash "$tmpdir_src" cleanup "$root5" "$created5" "$root5_identity" "$created5_identity" \
  >"$work/boundary/mount.log" 2>&1
mount_rc=$?
set -e
[[ "$mount_rc" -ne 0 && "$(sha256sum "$created5/sentinel" | awk '{print $1}')" == "$permission_sha" ]] \
  && grep -Fq 'mountpoint at or below run dir' "$work/boundary/mount.log" \
  && pass "nested mount readback refuses cleanup and preserves run bytes" \
  || bad "nested mount guard was bypassed"
cat >"$work/boundary/fakebin/findmnt" <<'STUB'
#!/usr/bin/env bash
exit 66
STUB
chmod +x "$work/boundary/fakebin/findmnt"
set +e
PATH="$work/boundary/fakebin:$PATH" REAL_STAT_PATH="$real_stat" \
  bash "$tmpdir_src" cleanup "$root5" "$created5" "$root5_identity" "$created5_identity" \
  >"$work/boundary/findmnt-failure.log" 2>&1
findmnt_failure_rc=$?
set -e
[[ "$findmnt_failure_rc" -ne 0 && "$(sha256sum "$created5/sentinel" | awk '{print $1}')" == "$permission_sha" ]] \
  && grep -Fq 'findmnt failed while proving cleanup boundaries' "$work/boundary/findmnt-failure.log" \
  && pass "findmnt failure preserves the run and fails closed" \
  || bad "findmnt failure rc=$findmnt_failure_rc was accepted or destructive"
rm -f -- "$work/boundary/fakebin/findmnt"

original5="$root5/check.original-identity"
mv "$created5" "$original5"
mkdir "$created5"
chmod 700 "$created5"
printf 'replacement\n' >"$created5/replacement"
replacement_sha="$(sha256sum "$created5/replacement" | awk '{print $1}')"
set +e
bash "$tmpdir_src" cleanup "$root5" "$created5" "$root5_identity" "$created5_identity" \
  >"$work/boundary/identity-substitution.log" 2>&1
identity_sub_rc=$?
set -e
[[ "$identity_sub_rc" -ne 0 && -f "$original5/sentinel" \
  && "$(sha256sum "$created5/replacement" | awk '{print $1}')" == "$replacement_sha" ]] \
  && pass "run-path substitution is detected without deleting original or replacement" \
  || bad "run identity substitution deleted or accepted the wrong directory"
rm -rf -- "$created5"
mv "$original5" "$created5"

collision5="$root5/check.collision"
mkdir "$collision5"
printf 'collision-sentinel\n' >"$collision5/sentinel"
collision_sha="$(sha256sum "$collision5/sentinel" | awk '{print $1}')"
set +e
bash "$tmpdir_src" create-at "$root5" "$collision5" >"$work/boundary/collision.out" 2>&1
collision_rc=$?
set -e
[[ "$collision_rc" -eq 75 && "$(sha256sum "$collision5/sentinel" | awk '{print $1}')" == "$collision_sha" ]] \
  && pass "exact-path collision returns 75 without claiming or changing bytes" \
  || bad "exact-path collision rc=$collision_rc or changed pre-existing bytes"
rm -rf -- "$collision5"
target_collision_root="$work/boundary/calyx/target-scratch"
mkdir "$target_collision_root"
chmod 700 "$target_collision_root"
target_mode_dir="$target_collision_root/check.permissions"
mkdir -m 700 "$target_mode_dir"
printf 'target-permission-sentinel\n' >"$target_mode_dir/sentinel"
target_mode_sha="$(sha256sum "$target_mode_dir/sentinel" | awk '{print $1}')"
target_root_identity="$(stat -c '%d:%i' -- "$target_collision_root")"
target_mode_identity="$(stat -c '%d:%i' -- "$target_mode_dir")"
for unsafe_mode in 770 707; do
  chmod "$unsafe_mode" "$target_collision_root"
  set +e
  bash "$tmpdir_src" validate-target "$target_collision_root" "$target_mode_dir" \
    "$target_root_identity" "$target_mode_identity" >"$work/boundary/target-root-mode-$unsafe_mode.log" 2>&1
  target_root_mode_rc=$?
  set -e
  chmod 700 "$target_collision_root"
  [[ "$target_root_mode_rc" -ne 0 \
    && "$(sha256sum "$target_mode_dir/sentinel" | awk '{print $1}')" == "$target_mode_sha" ]] \
    && pass "target root mode $unsafe_mode is rejected" \
    || bad "target root mode $unsafe_mode was accepted or destructive"

  chmod "$unsafe_mode" "$target_mode_dir"
  set +e
  bash "$tmpdir_src" validate-target "$target_collision_root" "$target_mode_dir" \
    "$target_root_identity" "$target_mode_identity" >"$work/boundary/target-mode-$unsafe_mode.log" 2>&1
  target_mode_rc=$?
  set -e
  chmod 700 "$target_mode_dir"
  [[ "$target_mode_rc" -ne 0 \
    && "$(sha256sum "$target_mode_dir/sentinel" | awk '{print $1}')" == "$target_mode_sha" ]] \
    && pass "target mode $unsafe_mode is rejected" \
    || bad "target mode $unsafe_mode was accepted or destructive"
done
for foreign_target_path in "$target_collision_root" "$target_mode_dir"; do
  foreign_key="$(basename "$foreign_target_path")"
  set +e
  PATH="$work/boundary/fakebin:$PATH" REAL_STAT_PATH="$real_stat" \
    FAKE_FOREIGN_PATH="$foreign_target_path" \
    bash "$tmpdir_src" validate-target "$target_collision_root" "$target_mode_dir" \
      "$target_root_identity" "$target_mode_identity" >"$work/boundary/foreign-target-$foreign_key.log" 2>&1
  foreign_target_rc=$?
  set -e
  [[ "$foreign_target_rc" -ne 0 \
    && "$(sha256sum "$target_mode_dir/sentinel" | awk '{print $1}')" == "$target_mode_sha" ]] \
    && pass "foreign-owner target path $foreign_key is rejected" \
    || bad "foreign-owner target path $foreign_key was accepted or destructive"
done
rm -rf -- "$target_mode_dir"
printf 'target-file\n' >"$target_collision_root/check.file"
ln -s "$work/boundary/missing-target" "$target_collision_root/check.link"
set +e
bash "$tmpdir_src" create-target-at "$target_collision_root" check \
  "$target_collision_root/check.file" >"$work/boundary/target-file.log" 2>&1
target_file_rc=$?
bash "$tmpdir_src" create-target-at "$target_collision_root" check \
  "$target_collision_root/check.link" >"$work/boundary/target-link.log" 2>&1
target_link_rc=$?
set -e
[[ "$target_file_rc" -eq 75 && "$target_link_rc" -eq 75 \
  && "$(tr -d '\n' <"$target_collision_root/check.file")" == "target-file" \
  && -L "$target_collision_root/check.link" ]] \
  && pass "target file and dangling-symlink collisions return 75 and remain unchanged" \
  || bad "target collision type was claimed or changed"
rm -f -- "$target_collision_root/check.file" "$target_collision_root/check.link"

expected_absent5="$root5/check.expected-absent"
set +e
bash "$tmpdir_src" cleanup "$root5" "$expected_absent5" "$root5_identity" 1:1 \
  >"$work/boundary/expected-absent.log" 2>&1
expected_absent_rc=$?
set -e
[[ "$expected_absent_rc" -ne 0 && ! -e "$expected_absent5" ]] \
  && grep -Fq 'expected run dir is absent' "$work/boundary/expected-absent.log" \
  && pass "identity-bound absent run fails closed without creation" \
  || bad "identity-bound absent run returned $expected_absent_rc"

root_original5="$work/boundary/calyx/tmp-original"
mv -T -- "$root5" "$root_original5"
mkdir -m 700 "$root5"
printf 'replacement-root\n' >"$root5/root-sentinel"
root_replacement_sha="$(sha256sum "$root5/root-sentinel" | awk '{print $1}')"
set +e
bash "$tmpdir_src" cleanup "$root5" "$created5" "$root5_identity" "$created5_identity" \
  >"$work/boundary/root-substitution.log" 2>&1
root_substitution_rc=$?
set -e
[[ "$root_substitution_rc" -ne 0 && -f "$root_original5/$(basename "$created5")/sentinel" \
  && "$(sha256sum "$root5/root-sentinel" | awk '{print $1}')" == "$root_replacement_sha" ]] \
  && pass "tmp-root substitution preserves original and replacement roots" \
  || bad "tmp-root substitution rc=$root_substitution_rc changed bytes"
rm -rf -- "$root5"
mv -T -- "$root_original5" "$root5"

real_mv="$(command -v mv)"
run_race_flag="$work/boundary/path-mv-called"
cat >"$work/boundary/fakebin/mv" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
: >"${RUN_RACE_FLAG:?}"
exec "${REAL_MV_PATH:?}" "$@"
STUB
chmod +x "$work/boundary/fakebin/mv"
set +e
PATH="$work/boundary/fakebin:$PATH" REAL_MV_PATH="$real_mv" REAL_STAT_PATH="$real_stat" \
  RUN_RACE_FLAG="$run_race_flag" \
  bash "$tmpdir_src" cleanup "$root5" "$created5" "$root5_identity" "$created5_identity" \
  >"$work/boundary/run-race.log" 2>&1
run_race_rc=$?
set -e
if [[ "$run_race_rc" -eq 0 && ! -e "$run_race_flag" && ! -e "$created5" ]]; then
  pass "run cleanup uses FD-anchored renameat2 instead of a PATH-resolved mv"
else
  bad "run cleanup rc=$run_race_rc invoked pathname mv or retained bytes"
  sed -n '1,20p' "$work/boundary/run-race.log" | sed 's/^/    race: /'
fi
rm -f -- "$work/boundary/fakebin/mv"

created5="$(bash "$tmpdir_src" create-at "$root5" "$root5/check.valid")"
created5_identity="$(stat -c '%d:%i' -- "$created5")"

bash "$tmpdir_src" cleanup "$root5" "$created5" "$root5_identity" "$created5_identity" \
  >"$work/boundary/cleanup.log"
[[ ! -e "$created5" ]] && pass "valid direct child cleanup succeeds" || bad "valid run child remains"

guard_mount_root="$work/boundary/guard-root"
guard_state_dir="$work/boundary/guard-state"
guard_mount_baseline="$guard_state_dir/guard-baseline"
mkdir -p "$guard_mount_root"
mkdir -m 700 "$guard_state_dir"
CALYX_TMP_ROOT="$guard_mount_root" CALYX_TMP_MIN_FREE_INODE_PERCENT=0 \
  CALYX_TMP_MIN_FREE_BLOCK_PERCENT=0 \
  bash "$guard_src" pre "$guard_mount_baseline" >"$work/boundary/guard-pre.log"
guard_baseline_identity="$(stat -c '%d:%i' -- "$guard_mount_baseline")"
guard_baseline_bytes="$(stat -c '%s' -- "$guard_mount_baseline")"
guard_baseline_sha="$(sha256sum "$guard_mount_baseline" | awk '{print $1}')"
guard_mount_path="$guard_mount_root/calyx-new-mount"
mkdir -m 700 "$guard_mount_path"
printf 'guard-mount-sentinel\n' >"$guard_mount_path/sentinel"
guard_mount_sha="$(sha256sum "$guard_mount_path/sentinel" | awk '{print $1}')"
cat >"$work/boundary/fakebin/findmnt" <<'STUB'
#!/usr/bin/env bash
exit 66
STUB
chmod +x "$work/boundary/fakebin/findmnt"
set +e
PATH="$work/boundary/fakebin:$PATH" REAL_STAT_PATH="$real_stat" CALYX_TMP_ROOT="$guard_mount_root" \
  CALYX_TMP_MIN_FREE_INODE_PERCENT=0 CALYX_TMP_MIN_FREE_BLOCK_PERCENT=0 \
  bash "$guard_src" post "$guard_mount_baseline" "$guard_baseline_identity" \
    "$guard_baseline_bytes" "$guard_baseline_sha" \
    >"$work/boundary/guard-findmnt-failure.log" 2>&1
guard_mount_rc=$?
set -e
[[ "$guard_mount_rc" -ne 0 \
  && "$(sha256sum "$guard_mount_path/sentinel" | awk '{print $1}')" == "$guard_mount_sha" ]] \
  && grep -Fq 'findmnt failed while proving cleanup boundaries' "$work/boundary/guard-findmnt-failure.log" \
  && pass "tmp guard findmnt failure preserves the candidate and fails closed" \
  || {
    bad "tmp guard findmnt failure rc=$guard_mount_rc was destructive"
    sed -n '1,30p' "$work/boundary/guard-findmnt-failure.log" | sed 's/^/    guard: /'
  }
rm -- "$work/boundary/fakebin/findmnt"
rm -rf -- "$guard_mount_root"

guard_tamper_root="$work/boundary/guard-tamper-root"
guard_tamper_state="$work/boundary/guard-tamper-state"
guard_tamper_baseline="$guard_tamper_state/baseline"
guard_tamper_path="$guard_tamper_root/calyx-baseline-tamper"
mkdir -m 700 "$guard_tamper_root" "$guard_tamper_state"
CALYX_TMP_ROOT="$guard_tamper_root" CALYX_TMP_MIN_FREE_INODE_PERCENT=0 \
  CALYX_TMP_MIN_FREE_BLOCK_PERCENT=0 \
  bash "$guard_src" pre "$guard_tamper_baseline" >"$work/boundary/guard-tamper-pre.log"
guard_tamper_identity="$(stat -c '%d:%i' -- "$guard_tamper_baseline")"
guard_tamper_bytes="$(stat -c '%s' -- "$guard_tamper_baseline")"
guard_tamper_sha="$(sha256sum "$guard_tamper_baseline" | awk '{print $1}')"
mkdir -m 700 "$guard_tamper_path"
printf 'baseline-tamper-sentinel\n' >"$guard_tamper_path/sentinel"
guard_tamper_sentinel_sha="$(sha256sum "$guard_tamper_path/sentinel" | awk '{print $1}')"
printf '%s\0' "$guard_tamper_path" >>"$guard_tamper_baseline"
set +e
CALYX_TMP_ROOT="$guard_tamper_root" CALYX_TMP_MIN_FREE_INODE_PERCENT=0 \
  CALYX_TMP_MIN_FREE_BLOCK_PERCENT=0 \
  bash "$guard_src" post "$guard_tamper_baseline" "$guard_tamper_identity" \
    "$guard_tamper_bytes" "$guard_tamper_sha" >"$work/boundary/guard-tamper-post.log" 2>&1
guard_tamper_rc=$?
set -e
[[ "$guard_tamper_rc" -ne 0 \
  && "$(sha256sum "$guard_tamper_path/sentinel" | awk '{print $1}')" == "$guard_tamper_sentinel_sha" ]] \
  && grep -Fq 'CALYX_TMP_GUARD_BASELINE_CHANGED: label=open' "$work/boundary/guard-tamper-post.log" \
  && pass "tmp guard rejects a forged baseline and preserves its hidden leak candidate" \
  || bad "tmp guard baseline tamper rc=$guard_tamper_rc was accepted or destructive"
rm -rf -- "$guard_tamper_root" "$guard_tamper_state"

guard_enum_root="$work/boundary/guard-enum-root"
guard_enum_state="$work/boundary/guard-enum-state"
guard_enum_path="$guard_enum_root/calyx-enumeration-failure"
mkdir -m 700 "$guard_enum_root" "$guard_enum_state" "$guard_enum_path"
printf 'guard-enumeration-sentinel\n' >"$guard_enum_path/sentinel"
guard_enum_sha="$(sha256sum "$guard_enum_path/sentinel" | awk '{print $1}')"
cat >"$work/boundary/fakebin/find" <<'STUB'
#!/usr/bin/env bash
printf '%s\0' "${FAKE_FIND_RECORD:?}"
exit 66
STUB
chmod +x "$work/boundary/fakebin/find"
set +e
PATH="$work/boundary/fakebin:$PATH" REAL_STAT_PATH="$real_stat" FAKE_FIND_RECORD="$guard_enum_path" \
  CALYX_TMP_ROOT="$guard_enum_root" CALYX_TMP_MIN_FREE_INODE_PERCENT=0 \
  CALYX_TMP_MIN_FREE_BLOCK_PERCENT=0 \
  bash "$guard_src" pre "$guard_enum_state/baseline" \
  >"$work/boundary/guard-enumeration-failure.log" 2>&1
guard_enum_rc=$?
set -e
[[ "$guard_enum_rc" -eq 66 && -f "$guard_enum_path/sentinel" \
  && "$(sha256sum "$guard_enum_path/sentinel" | awk '{print $1}')" == "$guard_enum_sha" ]] \
  && grep -Fq 'CALYX_TMP_GUARD_ENUMERATION_FAILED: label=baseline status=66' \
    "$work/boundary/guard-enumeration-failure.log" \
  && pass "tmp guard enumeration producer failure preserves every emitted candidate" \
  || {
    bad "tmp guard enumeration failure rc=$guard_enum_rc was fail-open or destructive"
    sed -n '1,25p' "$work/boundary/guard-enumeration-failure.log" | sed 's/^/    guard-enum: /'
  }
rm -- "$work/boundary/fakebin/find"
rm -rf -- "$guard_enum_root" "$guard_enum_state"

real_mkdir="$(command -v mkdir)"
cat >"$work/boundary/fakebin/mkdir" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
last="${!#}"
"${REAL_MKDIR_PATH:?}" "$@"
if [[ "$last" == "${FAKE_SWAP_CREATE_PATH:?}" ]]; then
  "${REAL_MV_PATH:?}" -- "$last" "$last.parked"
  "${REAL_MKDIR_PATH:?}" -- "$last"
  chmod 700 "$last"
  printf 'fake-mkdir-invoked\n' >"${FAKE_MKDIR_FLAG:?}"
fi
STUB
chmod +x "$work/boundary/fakebin/mkdir"
pinned_create="$root5/check.creator-pinned"
pinned_flag="$work/boundary/fake-mkdir-invoked"
set +e
PATH="$work/boundary/fakebin:$PATH" REAL_MKDIR_PATH="$real_mkdir" REAL_MV_PATH="$real_mv" \
  REAL_STAT_PATH="$real_stat" FAKE_SWAP_CREATE_PATH="$pinned_create" FAKE_MKDIR_FLAG="$pinned_flag" \
  bash "$tmpdir_src" create-at "$root5" "$pinned_create" \
  >"$work/boundary/pinned-create.out" 2>"$work/boundary/pinned-create.log"
pinned_rc=$?
set -e
pinned_identity="$(stat -c '%d:%i' -- "$pinned_create" 2>/dev/null || true)"
[[ "$pinned_rc" -eq 0 && "$pinned_identity" =~ ^[0-9]+:[0-9]+$ \
  && -d "$pinned_create" && ! -e "$pinned_create.parked" && ! -e "$pinned_flag" ]] \
  && pass "pinned creator bypasses PATH mkdir post-create substitution" \
  || {
    bad "owned create rc=$pinned_rc invoked a replaceable PATH mkdir or leaked a parked inode"
    sed -n '1,20p' "$work/boundary/pinned-create.log" | sed 's/^/    pinned-create: /'
  }
[[ ! "$pinned_identity" =~ ^[0-9]+:[0-9]+$ ]] || \
  bash "$tmpdir_src" cleanup "$root5" "$pinned_create" "$root5_identity" "$pinned_identity" \
    >"$work/boundary/pinned-create-cleanup.log"
[[ ! -e "$pinned_create" ]] \
  && pass "pinned creator identity remains cleanup-authoritative" \
  || bad "pinned creator path remained after exact cleanup"
