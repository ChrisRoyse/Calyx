#!/usr/bin/env bash
# Source-only post-rename identity-publication tests for fuzz/public gates.
[[ "${BASH_SOURCE[0]}" != "$0" ]] || {
  echo "source this module from gate-wrapper-tmpdir-regression.sh" >&2
  exit 64
}

echo
echo "=================================================================="
echo "WRAPPER SETUP PUBLICATION - post-rename rollback and deferred signals"
echo "=================================================================="

inject_wrapper_post_rename_hook() {
  local creator="$1"
  local signal_hook="$creator.signal-hook" post_hook="$creator.post-hook" patched="$creator.patched"
  cat >"$signal_hook" <<'PY'
        hook_parent = os.environ.get("CALYX_CREATOR_TEST_PARENT")
        hook_prefix = os.environ.get("CALYX_CREATOR_TEST_PREFIX", "")
        action = os.environ.get("CALYX_CREATOR_TEST_ACTION", "")
        if hook_parent == root and name.startswith(f"{hook_prefix}.") and action in ("hup", "int", "term"):
            import time

            ready = os.environ["CALYX_CREATOR_TEST_READY"]
            with open(ready, "w", encoding="ascii") as output:
                output.write(f"pid={os.getpid()} path={os.path.join(root, staging)} phase=pre-rename\n")
            release = os.environ["CALYX_CREATOR_TEST_RELEASE"]
            while not os.path.exists(release):
                time.sleep(0.01)
PY
  cat >"$post_hook" <<'PY'
            hook_parent = os.environ.get("CALYX_CREATOR_TEST_PARENT")
            hook_prefix = os.environ.get("CALYX_CREATOR_TEST_PREFIX", "")
            action = os.environ.get("CALYX_CREATOR_TEST_ACTION", "")
            if hook_parent == root and name.startswith(f"{hook_prefix}.") and action == "fail":
                import time

                ready = os.environ["CALYX_CREATOR_TEST_READY"]
                with open(ready, "w", encoding="ascii") as output:
                    output.write(f"pid={os.getpid()} path={os.path.join(root, name)}\n")
                if action == "fail":
                    raise CreateError("injected post-rename failure")
                release = os.environ["CALYX_CREATOR_TEST_RELEASE"]
                while not os.path.exists(release):
                    time.sleep(0.01)
PY
  awk -v signal_hook="$signal_hook" -v post_hook="$post_hook" '
    { print }
    $0 == "        # From this point the parent can recover the exact staging identity." {
      while ((getline line < signal_hook) > 0) print line
      close(signal_hook)
    }
    $0 == "            rename_noreplace(root_fd, staging, name)" {
      while ((getline line < post_hook) > 0) print line
      close(post_hook)
    }
  ' "$creator" >"$patched"
  mv -T -- "$patched" "$creator"
  chmod 755 "$creator"
  rm -- "$signal_hook" "$post_hook"
}

run_wrapper_setup_case() {
  local wrapper="$1" scope="$2" action="$3" key
  key="$wrapper-$scope-$action"
  local home="$work/wrapper-setup-$key/calyx" root public="" prefix parent
  local log="$work/wrapper-setup-$key/gate.log" stages="$work/wrapper-setup-$key/stages.log"
  local ready="$work/wrapper-setup-$key/identity-ready" release="$work/wrapper-setup-$key/identity-release"
  local expected_rc signal_name="" wrapper_helper_pid="" helper_alive_before_release=1
  if [[ "$wrapper" == fuzz ]]; then
    root="$home/repo"
    mkdir -p "$root/scripts" "$root/fuzz" "$root/home/.cargo" "$home/target-scratch"
    chmod 755 "$home" "$root"
    chmod 700 "$home/target-scratch"
    cp "$helper_src" "$root/scripts/check-tmpdir.sh"
    cp "$fuzz_src" "$root/scripts/fuzz-gate.sh"
    cp "$runner_src" "$root/scripts/gate-stage-runner.sh"
    cp "$runtime_src" "$root/scripts/gate-runtime-state.sh"
    cp "$capture_src" "$root/scripts/gate-identity-capture.sh"
    cp "$safe_helper_src" "$creator_src" "$record_exec_src" "$root/scripts/"
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
  inject_wrapper_post_rename_hook "$root/scripts/gate_owned_dir_create.py"
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
    CALYX_CREATOR_TEST_PARENT="$parent" CALYX_CREATOR_TEST_PREFIX="$prefix" \
    CALYX_CREATOR_TEST_READY="$ready" CALYX_CREATOR_TEST_RELEASE="$release" \
    CALYX_CREATOR_TEST_ACTION="${action,,}" \
    "${wrapper_command[@]}" >"$log" 2>&1 &
  wrapper_setup_pid=$!
  owned_pids="$wrapper_setup_pid" owned_pgids="$wrapper_setup_pid"
  gate_test_wait_ready "$ready" || true
  if [[ -s "$ready" ]]; then
    wrapper_helper_pid="$(sed -n 's/^pid=\([0-9]*\).*/\1/p' "$ready" | head -n1)"
  fi
  if [[ -n "$signal_name" && -s "$ready" ]]; then
    kill -s "$signal_name" "$wrapper_setup_pid"
    helper_alive_before_release=0
    sleep 0.25
    kill -0 "$wrapper_helper_pid" 2>/dev/null && helper_alive_before_release=1
    : >"$release"
  fi
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
  staging_remain="$(find "$home" -name '.calyx-create-*' -print -quit 2>/dev/null || true)"
  [[ -s "$ready" && "$wrapper_setup_rc" -eq "$expected_rc" \
    && ! -e "$wrapper_watchdog_marker" && "$wrapper_helper_pid" =~ ^[1-9][0-9]*$ \
    && ! -e "/proc/$wrapper_helper_pid" && "$helper_alive_before_release" -eq 1 \
    && -z "$tmp_remain" && -z "$target_remain" && -z "$staging_remain" \
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
