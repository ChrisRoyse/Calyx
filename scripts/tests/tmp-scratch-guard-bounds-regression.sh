#!/usr/bin/env bash
# Source-only bounded-enumeration regressions for tmp_scratch_guard.sh.
[[ "${BASH_SOURCE[0]}" != "$0" ]] || {
  echo "source this module from check-tmpdir-boundary-regression.sh" >&2
  exit 64
}

guard_bounds="$work/boundary/guard-bounds"
guard_bounds_root="$guard_bounds/root"
guard_bounds_state="$guard_bounds/state"
guard_bounds_fake="$guard_bounds/fakebin"
mkdir -p "$guard_bounds_root/calyx-limit-candidate" "$guard_bounds_state" "$guard_bounds_fake"
chmod 700 "$guard_bounds_root" "$guard_bounds_state"
printf 'bounded-enumeration-sentinel\n' >"$guard_bounds_root/calyx-limit-candidate/sentinel"
guard_bounds_sha="$(sha256sum "$guard_bounds_root/calyx-limit-candidate/sentinel" | awk '{print $1}')"
cat >"$guard_bounds_fake/find" <<'STUB'
#!/usr/bin/env bash
for ((i = 0; i < 16385; i++)); do
  printf '%s\0' "${FAKE_LIMIT_RECORD:?}-$i"
done
STUB
chmod +x "$guard_bounds_fake/find"
set +e
PATH="$guard_bounds_fake:$PATH" FAKE_LIMIT_RECORD="$guard_bounds_root/calyx-limit-record" \
  CALYX_TMP_ROOT="$guard_bounds_root" CALYX_TMP_MIN_FREE_INODE_PERCENT=0 \
  CALYX_TMP_MIN_FREE_BLOCK_PERCENT=0 \
  bash "$guard_src" pre "$guard_bounds_state/baseline" \
  >"$guard_bounds/limit.log" 2>&1
guard_bounds_rc=$?
set -e
[[ "$guard_bounds_rc" -eq 125 \
  && "$(sha256sum "$guard_bounds_root/calyx-limit-candidate/sentinel" | awk '{print $1}')" == "$guard_bounds_sha" ]] \
  && grep -Fq 'CALYX_TMP_GUARD_ENUMERATION_LIMIT' "$guard_bounds/limit.log" \
  && pass "tmp guard entry cap fails closed before mutating an emitted candidate" \
  || {
    bad "tmp guard enumeration cap rc=$guard_bounds_rc was unbounded or destructive"
    sed -n '1,20p' "$guard_bounds/limit.log" | sed 's/^/    guard-limit: /'
  }
rm -- "$guard_bounds_fake/find"

rm -rf -- "$guard_bounds_root" "$guard_bounds_state"
mkdir -p "$guard_bounds_root/calyx-linear-old" "$guard_bounds_state"
chmod 700 "$guard_bounds_root" "$guard_bounds_state"
printf 'linear-baseline-sentinel\n' >"$guard_bounds_root/calyx-linear-old/sentinel"
guard_linear_old_sha="$(sha256sum "$guard_bounds_root/calyx-linear-old/sentinel" | awk '{print $1}')"
CALYX_TMP_ROOT="$guard_bounds_root" CALYX_TMP_MIN_FREE_INODE_PERCENT=0 \
  CALYX_TMP_MIN_FREE_BLOCK_PERCENT=0 \
  bash "$guard_src" pre "$guard_bounds_state/baseline" >/dev/null
guard_baseline_id="$(stat -c '%d:%i' -- "$guard_bounds_state/baseline")"
guard_baseline_bytes="$(stat -c '%s' -- "$guard_bounds_state/baseline")"
guard_baseline_sha="$(sha256sum "$guard_bounds_state/baseline" | awk '{print $1}')"
mkdir -p "$guard_bounds_root/calyx-linear-new"
printf 'linear-compare-sentinel\n' >"$guard_bounds_root/calyx-linear-new/sentinel"
cat >"$guard_bounds_fake/grep" <<'STUB'
#!/usr/bin/env bash
printf 'called\n' >"${FAKE_GREP_CALLED:?}"
exit 99
STUB
chmod +x "$guard_bounds_fake/grep"
guard_grep_called="$guard_bounds/grep-called"
set +e
PATH="$guard_bounds_fake:$PATH" FAKE_GREP_CALLED="$guard_grep_called" \
  CALYX_TMP_ROOT="$guard_bounds_root" CALYX_TMP_MIN_FREE_INODE_PERCENT=0 \
  CALYX_TMP_MIN_FREE_BLOCK_PERCENT=0 \
  bash "$guard_src" post "$guard_bounds_state/baseline" "$guard_baseline_id" \
    "$guard_baseline_bytes" "$guard_baseline_sha" >"$guard_bounds/linear.log" 2>&1
guard_linear_rc=$?
set -e
[[ "$guard_linear_rc" -eq 0 && ! -e "$guard_bounds_root/calyx-linear-new" \
  && "$(sha256sum "$guard_bounds_root/calyx-linear-old/sentinel" | awk '{print $1}')" == "$guard_linear_old_sha" \
  && ! -e "$guard_grep_called" ]] \
  && pass "tmp guard compares sorted NUL sets once without per-entry grep" \
  || {
    bad "tmp guard linear comparison rc=$guard_linear_rc retained a candidate or called grep"
    sed -n '1,25p' "$guard_bounds/linear.log" | sed 's/^/    guard-linear: /'
  }
rm -rf -- "$guard_bounds_root" "$guard_bounds_state" "$guard_bounds_fake"
