#!/usr/bin/env bash
# Source-only public mirror lifecycle scenarios for issue #1421.
[[ "${BASH_SOURCE[0]}" != "$0" ]] || { echo "source this module from gate-wrapper-tmpdir-regression.sh" >&2; exit 64; }

setup_public_fixture() {
  local dev="$1" public="$2" stub
  mkdir -p "$dev/scripts" "$dev/home/.cargo" "$dev/global-tmp" "$public"
  chmod 755 "$(dirname "$dev")" "$dev"
  cp "$helper_src" "$dev/scripts/check-tmpdir.sh"
  cp "$guard_src" "$dev/scripts/tmp_scratch_guard.sh"
  cp "$public_src" "$dev/scripts/public-mirror-gate.sh"
  cp "$fuzz_src" "$dev/scripts/fuzz-gate.sh"
  cp "$runner_src" "$dev/scripts/gate-stage-runner.sh"
  cp "$runtime_src" "$cgroup_src" "$cgroup_scope_src" "$ofd_query_src" \
    "$safe_helper_src" "$creator_src" "$dev/scripts/"
  write_target_clean_stub "$dev/scripts/cargo-target-clean.sh"
  write_fake_cargo "$dev"
  cat >"$dev/fakebin/mkdir" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
last="${!#}"
command -p mkdir "$@"
if [[ -n "${FAKE_DELAY_CREATE_PREFIX:-}" \
  && "$(basename -- "$last")" == "$FAKE_DELAY_CREATE_PREFIX".* ]]; then
  pgid="$(ps -o pgid= -p "$$" | tr -d ' ')"
  printf 'pid=%s pgid=%s path=%s\n' "$$" "$pgid" "$last" >"${FAKE_CREATE_READY:?}"
  sleep 0.3
fi
STUB
  chmod +x "$dev/fakebin/mkdir"
  : >"$dev/home/.cargo/env"
  printf 'export CALYX_HOME=%q\n' "$(dirname "$dev")" >"$dev/env.sh"

  cat >"$dev/scripts/stage-stub.sh" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
stage="$(basename -- "$0")"
[[ -d "${TMPDIR:?}" ]] || exit 90
if [[ "${FAKE_REQUIRE_OWNED_MODE:-0}" == "1" ]]; then
  [[ "$(stat -c '%u:%a' -- "$TMPDIR")" == "$(id -u):700" \
    && "$(stat -c '%u:%a' -- "${CARGO_TARGET_DIR:?}")" == "$(id -u):700" ]] || exit 95
fi
pgid="$(ps -o pgid= -p "$$" | tr -d ' ')" cgroup="$(awk -F: '$1=="0"{print $3}' /proc/self/cgroup)" cgroup_id="$(stat -c '%d:%i' -- "/sys/fs/cgroup$cgroup")"
printf 'stage=%s tmp=%s target=%s pid=%s pgid=%s cgroup=%s cgroup_id=%s\n' \
  "$stage" "$TMPDIR" "${CARGO_TARGET_DIR:-}" "$$" "$pgid" "$cgroup" "$cgroup_id" >>"${FAKE_TMP_LOG:?}"
[[ ! -e "${CARGO_TARGET_DIR:?}/.preclean-sentinel" ]] || exit 94
printf '%s\n' "$stage" >>"$CARGO_TARGET_DIR/.gate-stage-use"
printf '%s\n' "$stage" >"$TMPDIR/$stage.marker"
STUB
  chmod +x "$dev/scripts/stage-stub.sh"
  for stub in secret-scan.sh public-leak-scan.sh cargo-fmt-workspace.sh \
    orphan_rs.sh linecount.sh; do
    cp "$dev/scripts/stage-stub.sh" "$dev/scripts/$stub"
  done

  git -C "$public" init -q -b main
  git -C "$public" config user.name regression
  git -C "$public" config user.email regression@example.invalid
  mkdir -p "$public/fuzz"
  printf 'public fixture\n' >"$public/README.md"
  printf '[workspace]\nmembers=[]\n' >"$public/fuzz/Cargo.toml"
  git -C "$public" add README.md fuzz/Cargo.toml
  git -C "$public" commit -q -m fixture
  git -C "$public" remote add origin https://github.com/ChrisRoyse/Calyx.git
}

run_public_gate() {
  local dev="$1" public="$2" log="$3" stages="$4" delete_baseline="$5"
  local fail_stage="${6:-}" block_stage="${7:-}" ready_file="${8:-}" explicit_target="${9:-}"
  local target_fail_once_file="${10:-}"
  local target_clean_log="${11:-}" create_prefix="${12:-}" create_ready="${13:-}"
  local stubborn_child="${14:-0}"
  local block_scope="${15:-gate}"
  local gate_args=(--public-dir "$public")
  [[ -z "$explicit_target" ]] || gate_args+=(--target-dir "$explicit_target")
  set +e
  env -u CALYX_FSV_ROOT -u CALYX_GATE_RUN_SLICE -u CALYX_GATE_RUN_SLICE_OWNER_PID -u CALYX_GATE_TMPDIR -u CALYX_GATE_TMP_ROOT \
    -u CARGO_TARGET_DIR -u TMPDIR \
    HOME="$dev/home" PATH="$dev/fakebin:$PATH" \
    CALYX_TMP_ROOT="$dev/global-tmp" \
    CALYX_TMP_MIN_FREE_INODE_PERCENT=0 CALYX_TMP_MIN_FREE_BLOCK_PERCENT=0 \
    CALYX_TARGET_SCRATCH_ROOT="$(dirname "$dev")/target-scratch" \
    CALYX_GATE_SIGNAL_GRACE_SECONDS=0.5 \
    FAKE_TMP_LOG="$stages" FAKE_DELETE_BASELINE="$delete_baseline" \
    FAKE_FAIL_STAGE="$fail_stage" FAKE_FAIL_CODE=42 \
    FAKE_BLOCK_STAGE="$block_stage" FAKE_READY_FILE="$ready_file" \
    FAKE_BLOCK_SCOPE="$block_scope" \
    FAKE_TARGET_CLEAN_FAIL_ONCE_FILE="$target_fail_once_file" \
    FAKE_TARGET_CLEAN_LOG="$target_clean_log" \
    FAKE_DELAY_CREATE_PREFIX="$create_prefix" FAKE_CREATE_READY="$create_ready" \
    FAKE_STUBBORN_CHILD="$stubborn_child" \
    FAKE_REQUIRE_OWNED_MODE=1 \
    timeout --signal=TERM --kill-after=2s 45s \
      bash "$dev/scripts/public-mirror-gate.sh" "${gate_args[@]}" >"$log" 2>&1
  PUBLIC_RC=$?
  set -e
}

echo
echo "=================================================================="
echo "SCENARIO 6 - public mirror gate owns one TMPDIR and cleans it"
echo "=================================================================="
public_dev="$work/public/calyx/dev"
public_repo="$work/public/calyx/public-mirror"
setup_public_fixture "$public_dev" "$public_repo"

newline_victim="$(dirname "$public_dev")/target-scratch/newline-victim"
mkdir -p "$newline_victim" && chmod 700 "$(dirname "$newline_victim")" "$newline_victim"
printf 'newline-victim-sentinel\n' >"$newline_victim/sentinel"
newline_victim_sha="$(sha256sum "$newline_victim/sentinel" | awk '{print $1}')"
set +e
env -u CALYX_GATE_RUN_SLICE -u CALYX_GATE_RUN_SLICE_OWNER_PID \
  HOME="$public_dev/home" PATH="$public_dev/fakebin:$PATH" \
  bash "$public_dev/scripts/public-mirror-gate.sh" --public-dir "$public_repo" \
  --target-dir "$newline_victim"$'\nignored-suffix' >"$work/public/newline-target.log" 2>&1
newline_target_rc=$?
set -e
[[ "$newline_target_rc" -eq 2 \
  && "$(sha256sum "$newline_victim/sentinel" | awk '{print $1}')" == "$newline_victim_sha" \
  && ! -e "$(dirname "$public_dev")/tmp" ]] \
  && grep -Fq 'paths must be single-line' "$work/public/newline-target.log" \
  && pass "newline-bearing explicit target is rejected before normalization or cleanup" \
  || bad "newline target rc=$newline_target_rc reached or changed its first-line victim"
rm -rf -- "$newline_victim"

git -C "$public_repo" remote set-url origin https://github.com/ChrisRoyse/Calyx-Dev.git
set +e
env -u CALYX_FSV_ROOT -u CALYX_GATE_RUN_SLICE -u CALYX_GATE_RUN_SLICE_OWNER_PID HOME="$public_dev/home" PATH="$public_dev/fakebin:$PATH" \
  FAKE_TMP_LOG="$work/public/bad-origin-stages.log" \
  timeout --signal=TERM --kill-after=2s 20s bash "$public_dev/scripts/public-mirror-gate.sh" \
    --public-dir "$public_repo" >"$work/public/bad-origin.log" 2>&1
bad_origin_rc=$?
set -e
git -C "$public_repo" remote set-url origin https://github.com/ChrisRoyse/Calyx.git
[[ "$bad_origin_rc" -eq 2 && ! -e "$work/public/bad-origin-stages.log" \
  && ! -e "$(dirname "$public_dev")/tmp" ]] \
  && pass "near-match Calyx-Dev origin is rejected before scratch creation" \
  || bad "near-match public origin reached owned scratch or workload"

mkdir -p "$work/public/shared-fsv-sentinel"
printf 'preserve-fsv\n' >"$work/public/shared-fsv-sentinel/sentinel"
fsv_sentinel_sha="$(sha256sum "$work/public/shared-fsv-sentinel/sentinel" | awk '{print $1}')"
set +e
HOME="$public_dev/home" PATH="$public_dev/fakebin:$PATH" \
  CALYX_FSV_ROOT="$work/public/shared-fsv-sentinel" \
  FAKE_TMP_LOG="$work/public/fsv-root-stages.log" \
  timeout --signal=TERM --kill-after=2s 20s bash "$public_dev/scripts/public-mirror-gate.sh" \
    --public-dir "$public_repo" >"$work/public/fsv-root.log" 2>&1
fsv_root_rc=$?
set -e
if [[ "$fsv_root_rc" -eq 1 && ! -e "$work/public/fsv-root-stages.log" \
  && "$(sha256sum "$work/public/shared-fsv-sentinel/sentinel" | awk '{print $1}')" == "$fsv_sentinel_sha" \
  && ! -e "$(dirname "$public_dev")/tmp" ]]; then
  pass "public gate refuses suite-wide CALYX_FSV_ROOT before scratch and preserves bytes"
else
  bad "public CALYX_FSV_ROOT refusal was bypassed"
  printf '    public-fsv: rc=%s tmp_exists=%s stages_exists=%s\n' \
    "$fsv_root_rc" "$(test -e "$(dirname "$public_dev")/tmp" && echo yes || echo no)" \
    "$(test -e "$work/public/fsv-root-stages.log" && echo yes || echo no)"
  sed -n '1,20p' "$work/public/fsv-root.log" | sed 's/^/    public-fsv: /'
fi

public_log="$work/public/gate.log"
public_stages="$work/public/stages.log"
public_clean_events="$work/public/target-clean-events.log"
public_explicit_target="$(dirname "$public_dev")/target-scratch/public-explicit"
mkdir -p "$public_explicit_target"
chmod 700 "$(dirname "$public_explicit_target")"
printf 'preclean-sentinel\n' >"$public_explicit_target/.preclean-sentinel"
run_public_gate "$public_dev" "$public_repo" "$public_log" "$public_stages" \
  0 "" "" "" "$public_explicit_target" "" "$public_clean_events"
public_tmp="$(sed -n 's/^public-mirror-gate: TMPDIR=//p' "$public_log" | head -n 1)"
if [[ "$PUBLIC_RC" -eq 0 ]]; then
  pass "public mirror gate exits 0"
else
  bad "public mirror rc=$PUBLIC_RC"
  sed -n '1,60p' "$public_log" | sed 's/^/    public: /'
fi
[[ "$(wc -l <"$public_stages" | tr -d ' ')" -eq 12 \
  && "$(unique_tmps "$public_stages" | wc -l | tr -d ' ')" -eq 1 ]] \
  && pass "all twelve public and real nested-fuzz stages observed one TMPDIR" \
  || bad "public stages did not share one TMPDIR"
gate_test_exact_stage_manifest "$public_stages" \
  secret-scan.sh public-leak-scan.sh cargo-fmt-workspace.sh cargo:check cargo:clippy \
  cargo:fmt cargo:check cargo:clippy cargo:nextest cargo:test orphan_rs.sh linecount.sh \
  && pass "public gate stage manifest is exact and ordered" \
  || bad "public gate stage manifest was dropped, duplicated, or reordered"
[[ "$public_tmp" == "$(dirname "$public_dev")/tmp/"check.* && ! -e "$public_tmp" ]] \
  && pass "public gate TMPDIR is a cleaned Calyx child" \
  || bad "public TMPDIR remains or escaped: $public_tmp"
[[ -z "$(git -C "$public_repo" status --porcelain)" ]] \
  && pass "public fixture remains byte-clean" \
  || bad "public gate dirtied its checkout"
[[ ! -e "$public_explicit_target" ]] \
  && pass "public explicit Cargo target was precleaned, used, and removed" \
  || bad "public explicit Cargo target remains"
gate_test_cleanup_readback_bounded "$public_log" \
  && gate_test_tmp_guard_readbacks_bounded "$public_log" 3 \
  && pass "public gate emits one cleanup plus bounded pre/post capacity readbacks" \
  || bad "public cleanup/capacity readback shape or count diverged"
[[ "$(unique_targets "$public_stages")" == "$public_explicit_target" ]] \
  && gate_test_log_groups_owned "$public_stages" "$public_explicit_target" \
  && pass "every public and nested-fuzz stage used its exact owned process group and target" \
  || bad "public stage target or process-group ownership diverged"
public_fuzz_pid=""
public_fuzz_pgid=""
read -r public_fuzz_pid public_fuzz_pgid < <(
  sed -n 's/^fuzz-gate: pid=\([0-9]*\) pgid=\([0-9]*\)$/\1 \2/p' "$public_log" | head -n1
) || true
[[ -n "$public_fuzz_pid" && "$public_fuzz_pid" == "$public_fuzz_pgid" ]] \
  && pass "public gate launched nested fuzz as an owned process-group leader" \
  || bad "public nested fuzz wrapper bypassed stage-group ownership"
first_clean="$(sed -n '1p' "$public_clean_events")"
last_clean="$(sed -n '$p' "$public_clean_events")"
[[ "$first_clean" == "target=$public_explicit_target sentinel=1 use_lines=0" \
  && "$last_clean" == "target=$public_explicit_target sentinel=0 use_lines="* \
  && "${last_clean##*use_lines=}" -gt 0 \
  && "$(wc -l <"$public_clean_events" | tr -d ' ')" -eq 2 ]] \
  && pass "public target was precleaned before use and contained physical use bytes before final cleanup" \
  || bad "public target clean/use ordering is unproven: first='$first_clean' last='$last_clean'"

echo
echo "=================================================================="
echo "SCENARIO 7 - public gate propagates a missing-baseline cleanup error"
echo "=================================================================="
public_fail_log="$work/public/missing-baseline.log"
public_fail_stages="$work/public/missing-baseline-stages.log"
run_public_gate "$public_dev" "$public_repo" "$public_fail_log" "$public_fail_stages" 1
public_fail_tmp="$(sed -n 's/^public-mirror-gate: TMPDIR=//p' "$public_fail_log" | head -n 1)"
[[ "$PUBLIC_RC" -ne 0 ]] \
  && pass "public missing baseline makes the gate nonzero" \
  || bad "public missing baseline was ignored"
grep -Fq 'CALYX_TMP_GUARD_MISSING_BASELINE' "$public_fail_log" \
  && grep -Fq 'public-mirror-gate: ERROR tmp guard cleanup failed' "$public_fail_log" \
  && pass "public gate reports and propagates tmp-guard failure" \
  || bad "public tmp-guard failure diagnostic absent"
[[ -n "$public_fail_tmp" && ! -e "$public_fail_tmp" ]] \
  && pass "public failed-cleanup run root is still absent" \
  || bad "public failed-cleanup run root remains"

echo
echo "=================================================================="
echo "SCENARIO 8 - public stage rc 42 is preserved and both roots clean"
echo "=================================================================="
public_stage_fail_log="$work/public/stage-failure.log"
public_stage_fail_stages="$work/public/stage-failure-stages.log"
run_public_gate "$public_dev" "$public_repo" "$public_stage_fail_log" "$public_stage_fail_stages" 0 check
public_stage_fail_tmp="$(sed -n 's/^public-mirror-gate: TMPDIR=//p' "$public_stage_fail_log" | head -n1)"
public_stage_fail_target="$(sed -n 's/^public-mirror-gate: target_dir=//p' "$public_stage_fail_log" | head -n1)"
[[ "$PUBLIC_RC" -eq 42 && ! -e "$public_stage_fail_tmp" && ! -e "$public_stage_fail_target" ]] \
  && pass "public rc 42 preserved with temp and target roots absent" \
  || bad "public failure rc=$PUBLIC_RC or scratch remained"
! grep -Fq 'stage=cargo:clippy ' "$public_stage_fail_stages" \
  && pass "public stages after the failing check did not run" \
  || bad "public clippy ran after check failed"

echo
echo "=================================================================="
echo "SCENARIO 9 - PID-only TERM is forwarded by the public gate"
echo "=================================================================="
public_signal_log="$work/public/signal.log"
public_signal_stages="$work/public/signal-stages.log"
public_ready="$work/public/signal-ready"
setsid env -u CALYX_FSV_ROOT -u CALYX_GATE_RUN_SLICE -u CALYX_GATE_RUN_SLICE_OWNER_PID \
  -u CALYX_GATE_RUN_SLICE_OWNER_STARTTIME -u CALYX_GATE_TMPDIR -u CALYX_GATE_TMP_ROOT \
  -u CARGO_TARGET_DIR -u TMPDIR \
  HOME="$public_dev/home" PATH="$public_dev/fakebin:$PATH" \
  CALYX_TMP_ROOT="$public_dev/global-tmp" \
  CALYX_TMP_MIN_FREE_INODE_PERCENT=0 CALYX_TMP_MIN_FREE_BLOCK_PERCENT=0 \
  CALYX_TARGET_SCRATCH_ROOT="$(dirname "$public_dev")/target-scratch" \
  CALYX_GATE_SIGNAL_GRACE_SECONDS=0.5 \
  FAKE_TMP_LOG="$public_signal_stages" FAKE_DELETE_BASELINE=0 FAKE_FAIL_STAGE= \
  FAKE_BLOCK_STAGE=check FAKE_READY_FILE="$public_ready" \
  FAKE_BLOCK_SCOPE=fuzz \
  FAKE_TARGET_CLEAN_FAIL_ONCE_FILE= \
  FAKE_TARGET_CLEAN_LOG= FAKE_DELAY_CREATE_PREFIX= FAKE_CREATE_READY= \
  FAKE_STUBBORN_CHILD=1 \
  FAKE_REQUIRE_OWNED_MODE=1 \
  bash "$public_dev/scripts/public-mirror-gate.sh" --public-dir "$public_repo" >"$public_signal_log" 2>&1 &
public_signal_pid=$!
owned_pids="$public_signal_pid"
public_signal_pgid="$(ps -o pgid= -p "$public_signal_pid" 2>/dev/null | tr -d ' ')"
[[ "$public_signal_pgid" == "$public_signal_pid" ]] && owned_pgids="$public_signal_pgid"
gate_test_wait_ready "$public_ready" || true
if [[ -f "$public_ready" && "$public_signal_pgid" == "$public_signal_pid" ]]; then
  read -r public_stage_pid_field public_stage_pgid_field public_cgroup_field public_cgroup_id_field \
    public_child_pid_field public_child_pgid_field public_child_cgroup_field <"$public_ready"
  public_stage_pid="${public_stage_pid_field#pid=}"
  public_stage_pgid="${public_stage_pgid_field#pgid=}"
  public_cgroup="${public_cgroup_field#cgroup=}"
  public_cgroup_id="${public_cgroup_id_field#cgroup_id=}"
  public_child_pid="${public_child_pid_field#child=}"
  public_child_pgid="${public_child_pgid_field#child_pgid=}"
  public_child_cgroup="${public_child_cgroup_field#child_cgroup=}"
  [[ "$public_stage_pid" == "$public_stage_pgid" ]] \
    && owned_pgids="$owned_pgids $public_stage_pgid $public_child_pgid"
  signal_public_fuzz_pid="$(sed -n 's/^fuzz-gate: pid=\([0-9]*\) pgid=[0-9]*$/\1/p' "$public_signal_log" | head -n1)"
  [[ "$public_stage_pid" == "$public_stage_pgid" \
    && "$signal_public_fuzz_pid" != "$public_stage_pid" \
    && "$public_cgroup" == */calyxgate-*.scope ]] \
    && pass "public nested fuzz Cargo owns a distinct scope and process group" \
    || bad "public nested fuzz stage scope ownership is invalid"
  [[ -n "$public_child_pid" && "$public_child_pgid" == "$public_child_pid" \
    && "$public_child_pgid" != "$public_stage_pgid" \
    && "$public_child_cgroup" == "$public_cgroup" ]] \
    && pass "public setsid descendant escaped the PGID but stayed in the cgroup" \
    || bad "public stubborn descendant cgroup containment is invalid"
  kill -TERM "$public_signal_pid"
  public_watchdog_marker="$work/public/signal-watchdog-fired"
  (
    sleep 8
    printf 'fired\n' >"$public_watchdog_marker"
    kill -TERM "$public_signal_pid" 2>/dev/null || true
    kill -TERM -- "-$public_stage_pgid" 2>/dev/null || true
    sleep 0.5
    kill -KILL "$public_signal_pid" 2>/dev/null || true
    kill -KILL -- "-$public_stage_pgid" 2>/dev/null || true
    [[ -n "$public_child_pid" ]] && kill -KILL "$public_child_pid" 2>/dev/null || true
  ) &
  public_watchdog=$!
  set +e
  wait "$public_signal_pid"; public_signal_rc=$?
  set -e
  kill -KILL "$public_watchdog" 2>/dev/null || true
  wait "$public_watchdog" 2>/dev/null || true
  [[ ! -e "$public_watchdog_marker" ]] \
    && pass "public TERM completed before its regression watchdog" \
    || bad "public TERM required watchdog rescue"
  public_signal_tmp="$(sed -n 's/^public-mirror-gate: TMPDIR=//p' "$public_signal_log" | head -n1)"
  public_signal_target="$(sed -n 's/^public-mirror-gate: target_dir=//p' "$public_signal_log" | head -n1)"
  [[ "$public_signal_rc" -eq 143 && ! -e "$public_signal_tmp" && ! -e "$public_signal_target" \
    && ! -e "/proc/$public_child_pid" ]] \
    && gate_test_cgroup_absent "$public_cgroup" "$public_cgroup_id" \
    && grep -Fq 'forwarding TERM to run_slice=' "$public_signal_log" \
    && pass "public PID-only TERM drains the setsid cgroup and cleans both roots" \
    || bad "public TERM rc=$public_signal_rc or owned state remained"
else
  bad "public signal case never reached a verified ready process group"
  [[ "$public_signal_pgid" == "$public_signal_pid" ]] \
    && gate_test_stop_group_bounded "$public_signal_pgid" || kill -KILL "$public_signal_pid" 2>/dev/null || true
  wait "$public_signal_pid" 2>/dev/null || true
fi
owned_pids=""
owned_pgids=""

# shellcheck source=gate-wrapper-public-signal-variants-regression.sh
source "$public_signal_variants"

# shellcheck source=gate-wrapper-public-boundary-regression.sh
source "$public_boundary"

echo
echo "=================================================================="
echo "SCENARIO 10 - explicit-target setup failure still retries cleanup"
echo "=================================================================="
public_setup_target="$(dirname "$public_dev")/target-scratch/public-setup-failure"
mkdir -p "$public_setup_target"
printf 'setup-failure-sentinel\n' >"$public_setup_target/sentinel"
public_setup_marker="$work/public/target-clean-failed-once"
public_setup_log="$work/public/target-setup-failure.log"
public_setup_stages="$work/public/target-setup-failure-stages.log"
run_public_gate "$public_dev" "$public_repo" "$public_setup_log" "$public_setup_stages" \
  0 "" "" "" "$public_setup_target" "$public_setup_marker"
public_setup_tmp="$(sed -n 's/^public-mirror-gate: TMPDIR=//p' "$public_setup_log" | head -n1)"
[[ "$PUBLIC_RC" -eq 73 && -f "$public_setup_marker" && -f "$public_setup_target/sentinel" \
  && ! -e "$public_setup_tmp" && ! -e "$public_setup_stages" ]] \
  && pass "setup rc 73 preserves the unclaimed explicit target while cleaning the run root" \
  || bad "explicit-target setup failure deleted unclaimed bytes or changed rc=$PUBLIC_RC"
rm -rf -- "$public_setup_target"

nested_explicit_target="$(dirname "$public_dev")/target-scratch/intermediate/public-target"
run_public_gate "$public_dev" "$public_repo" "$work/public/nested-target.log" \
  "$work/public/nested-target-stages.log" 0 "" "" "" "$nested_explicit_target"
nested_explicit_tmp="$(sed -n 's/.*calyx_check_tmpdir_created root=[^ ]* run=\([^ ]*\) uid=.*/\1/p' \
  "$work/public/nested-target.log" | head -n1)"
[[ "$PUBLIC_RC" -eq 2 && ! -e "$(dirname "$nested_explicit_target")" \
  && ! -e "$nested_explicit_tmp" && ! -s "$work/public/nested-target-stages.log" ]] \
  && pass "nested explicit target is rejected without leaking parent or run scratch" \
  || bad "nested explicit target was accepted or leaked scratch"


echo
