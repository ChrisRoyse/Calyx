#!/usr/bin/env bash
# Source-only post-helper/pre-identity publication tests for fuzz/public gates.
[[ "${BASH_SOURCE[0]}" != "$0" ]] || {
  echo "source this module from gate-wrapper-tmpdir-regression.sh" >&2
  exit 64
}

echo
echo "=================================================================="
echo "WRAPPER SETUP PUBLICATION - identity failures and deferred signals"
echo "=================================================================="

wrapper_setup_real_stat="$(command -v stat)"
wrapper_setup_real_readlink="$(command -v readlink)"
write_delayed_identity_stat() {
  local path="$1"
  cat >"$path" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
last="${!#}"
resolved="$("${WRAPPER_SETUP_REAL_READLINK:?}" -f -- "$last" 2>/dev/null || printf '%s' "$last")"
if [[ "$*" == *%d:%i* \
  && "$(dirname -- "$resolved")" == "${FAKE_IDENTITY_PARENT:?}" \
  && "$(basename -- "$resolved")" == "${FAKE_IDENTITY_PREFIX:?}."* ]]; then
  printf 'pid=%s path=%s\n' "$BASHPID" "$last" >"${FAKE_IDENTITY_READY:?}"
  [[ "${FAKE_IDENTITY_ACTION:?}" != fail ]] || exit 66
  sleep 30
fi
exec "${WRAPPER_SETUP_REAL_STAT:?}" "$@"
STUB
  chmod +x "$path"
}

run_wrapper_setup_case() {
  local wrapper="$1" scope="$2" action="$3" key
  key="$wrapper-$scope-$action"
  local home="$work/wrapper-setup-$key/calyx" root public="" prefix parent
  local log="$work/wrapper-setup-$key/gate.log" stages="$work/wrapper-setup-$key/stages.log"
  local ready="$work/wrapper-setup-$key/identity-ready" expected_rc signal_name="" wrapper_helper_pid=""
  if [[ "$wrapper" == fuzz ]]; then
    root="$home/repo"
    mkdir -p "$root/scripts" "$root/fuzz" "$root/home/.cargo" "$home/target-scratch"
    chmod 755 "$home" "$root"
    chmod 700 "$home/target-scratch"
    cp "$helper_src" "$root/scripts/check-tmpdir.sh"
    cp "$fuzz_src" "$root/scripts/fuzz-gate.sh"
    cp "$runner_src" "$root/scripts/gate-stage-runner.sh"
    cp "$runtime_src" "$root/scripts/gate-runtime-state.sh"
    cp "$safe_helper_src" "$creator_src" "$root/scripts/"
    cp "$cgroup_src" "$root/scripts/gate-cgroup-runtime.sh"
    cp "$cgroup_scope_src" "$root/scripts/gate-cgroup-scope.sh"
    cp "$ofd_query_src" "$root/scripts/gate_ofd_lock_query.py"
    write_target_clean_stub "$root/scripts/cargo-target-clean.sh"
    write_fake_cargo "$root"
    : >"$root/home/.cargo/env"
    printf 'export CALYX_HOME=%q\n' "$home" >"$root/env.sh"
    printf '[workspace]\nmembers=[]\n' >"$root/fuzz/Cargo.toml"
    prefix=fuzz-gate
  else
    root="$home/dev"
    public="$home/public-mirror"
    setup_public_fixture "$root" "$public"
    prefix=public-mirror-gate
  fi
  parent="$home/tmp"
  [[ "$scope" != target ]] || parent="$home/target-scratch"
  [[ "$scope" != tmp ]] || prefix=check
  write_delayed_identity_stat "$root/fakebin/stat"
  case "$action" in
    fail) expected_rc=125 ;;
    HUP) expected_rc=129; signal_name=HUP ;;
    INT) expected_rc=130; signal_name=INT ;;
    TERM) expected_rc=143; signal_name=TERM ;;
  esac
  wrapper_command=(bash "$root/scripts/fuzz-gate.sh")
  [[ "$wrapper" != public ]] || wrapper_command=(bash "$root/scripts/public-mirror-gate.sh" --public-dir "$public")
  setsid env --default-signal=HUP,INT,TERM -u CALYX_FSV_ROOT -u CALYX_GATE_RUN_SLICE -u CALYX_GATE_RUN_SLICE_OWNER_PID \
    -u CALYX_GATE_RUN_SLICE_OWNER_STARTTIME \
    -u CALYX_GATE_TMPDIR -u CALYX_GATE_TMP_ROOT -u CARGO_TARGET_DIR -u TMPDIR \
    HOME="$root/home" PATH="$root/fakebin:$PATH" \
    CALYX_TMP_ROOT="$root/global-tmp" CALYX_TMP_MIN_FREE_INODE_PERCENT=0 \
    CALYX_TMP_MIN_FREE_BLOCK_PERCENT=0 FAKE_TMP_LOG="$stages" \
    FAKE_DELETE_BASELINE=0 FAKE_FAIL_STAGE= FAKE_BLOCK_STAGE= FAKE_READY_FILE= \
    FAKE_REQUIRE_OWNED_MODE=1 FAKE_TARGET_CLEAN_FAIL_ONCE_FILE= \
    FAKE_TARGET_CLEAN_LOG= FAKE_DELAY_CREATE_PREFIX= FAKE_CREATE_READY= \
    WRAPPER_SETUP_REAL_STAT="$wrapper_setup_real_stat" \
    WRAPPER_SETUP_REAL_READLINK="$wrapper_setup_real_readlink" FAKE_IDENTITY_PARENT="$parent" \
    FAKE_IDENTITY_PREFIX="$prefix" FAKE_IDENTITY_READY="$ready" \
    FAKE_IDENTITY_ACTION="${action,,}" \
    "${wrapper_command[@]}" >"$log" 2>&1 &
  wrapper_setup_pid=$!
  owned_pids="$wrapper_setup_pid" owned_pgids="$wrapper_setup_pid"
  gate_test_wait_ready "$ready" || true
  wrapper_helper_pid="$(sed -n 's/^pid=\([0-9]*\).*/\1/p' "$ready" | head -n1)"
  [[ -z "$signal_name" || ! -s "$ready" ]] || kill -s "$signal_name" "$wrapper_setup_pid"
  wrapper_watchdog_marker="$work/wrapper-setup-$key/watchdog-fired"
  ( sleep 8; printf 'fired\n' >"$wrapper_watchdog_marker"; kill -KILL -- "-$wrapper_setup_pid" 2>/dev/null || true ) &
  wrapper_watchdog=$!
  set +e
  wait "$wrapper_setup_pid"; wrapper_setup_rc=$?
  set -e
  kill -KILL "$wrapper_watchdog" 2>/dev/null || true
  wait "$wrapper_watchdog" 2>/dev/null || true
  owned_pids="" owned_pgids=""
  tmp_remain="$(find "$home/tmp" -mindepth 1 -maxdepth 1 -name 'check.*' -print -quit 2>/dev/null || true)"
  target_remain="$(find "$home/target-scratch" -mindepth 1 -print -quit 2>/dev/null || true)"
  [[ -s "$ready" && "$wrapper_setup_rc" -eq "$expected_rc" \
    && ! -e "$wrapper_watchdog_marker" && "$wrapper_helper_pid" =~ ^[1-9][0-9]*$ \
    && ! -e "/proc/$wrapper_helper_pid" && -z "$tmp_remain" && -z "$target_remain" \
    && ! -s "$stages" ]] \
    && ! gate_test_group_alive "$wrapper_setup_pid" \
    && pass "$wrapper $scope identity $action leaves no unpublished root" \
    || bad "$wrapper $scope identity $action rc=$wrapper_setup_rc leaked or reached a stage"
}

for wrapper_setup_wrapper in fuzz public; do
  for wrapper_setup_scope in tmp target; do
    for wrapper_setup_action in fail HUP INT TERM; do
      run_wrapper_setup_case "$wrapper_setup_wrapper" "$wrapper_setup_scope" "$wrapper_setup_action"
    done
  done
done
