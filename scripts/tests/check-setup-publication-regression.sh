#!/usr/bin/env bash
# Source-only post-rename identity-publication tests for check.sh.
[[ "${BASH_SOURCE[0]}" != "$0" ]] || {
  echo "source this module from check-tmpdir-regression.sh" >&2
  exit 64
}

echo
echo "=================================================================="
echo "CHECK SETUP PUBLICATION - post-rename rollback and deferred signals"
echo "=================================================================="

inject_post_rename_hook() {
  local creator="$1"
  local pre_hook="$creator.pre-hook" signal_hook="$creator.signal-hook"
  local post_hook="$creator.post-hook" patched="$creator.patched"
  cat >"$pre_hook" <<'PY'
        hook_parent = os.environ.get("CALYX_CREATOR_TEST_PARENT")
        hook_prefix = os.environ.get("CALYX_CREATOR_TEST_PREFIX", "")
        action = os.environ.get("CALYX_CREATOR_TEST_ACTION", "")
        if hook_parent == root and name.startswith(f"{hook_prefix}.") and action == "pre_fail":
            ready = os.environ["CALYX_CREATOR_TEST_READY"]
            starttime = open(f"/proc/{os.getpid()}/stat", encoding="ascii").read().rsplit(") ", 1)[1].split()[19]
            with open(ready, "w", encoding="ascii") as output:
                output.write(f"pid={os.getpid()} starttime={starttime} path={os.path.join(root, staging)} phase=pre-identity\n")
            raise OSError(errno.EMFILE, "injected pre-identity open failure")
PY
  cat >"$signal_hook" <<'PY'
        hook_parent = os.environ.get("CALYX_CREATOR_TEST_PARENT")
        hook_prefix = os.environ.get("CALYX_CREATOR_TEST_PREFIX", "")
        action = os.environ.get("CALYX_CREATOR_TEST_ACTION", "")
        if hook_parent == root and name.startswith(f"{hook_prefix}.") and action in ("hup", "int", "term", "hang"):
            import time

            ready = os.environ["CALYX_CREATOR_TEST_READY"]
            starttime = open(f"/proc/{os.getpid()}/stat", encoding="ascii").read().rsplit(") ", 1)[1].split()[19]
            with open(ready, "w", encoding="ascii") as output:
                output.write(f"pid={os.getpid()} starttime={starttime} path={os.path.join(root, staging)} phase=pre-rename\n")
            release = os.environ["CALYX_CREATOR_TEST_RELEASE"]
            while not os.path.exists(release):
                time.sleep(0.01)
PY
  cat >"$post_hook" <<'PY'
            hook_parent = os.environ.get("CALYX_CREATOR_TEST_PARENT")
            hook_prefix = os.environ.get("CALYX_CREATOR_TEST_PREFIX", "")
            action = os.environ.get("CALYX_CREATOR_TEST_ACTION", "")
            if hook_parent == root and name.startswith(f"{hook_prefix}.") and action in ("fail", "swap"):
                import time

                ready = os.environ["CALYX_CREATOR_TEST_READY"]
                starttime = open(f"/proc/{os.getpid()}/stat", encoding="ascii").read().rsplit(") ", 1)[1].split()[19]
                with open(ready, "w", encoding="ascii") as output:
                    output.write(f"pid={os.getpid()} starttime={starttime} path={os.path.join(root, name)}\n")
                if action == "swap":
                    parked = f"{name}.parked"
                    os.rename(name, parked, src_dir_fd=root_fd, dst_dir_fd=root_fd)
                    os.mkdir(name, 0o700, dir_fd=root_fd)
                    replacement = named_identity(root_fd, name)
                    with open(os.environ["CALYX_CREATOR_TEST_SWAP_STATE"], "w", encoding="ascii") as output:
                        output.write(
                            f"path={os.path.join(root, name)} parked={os.path.join(root, parked)} "
                            f"original_id={staging_identity} replacement_id={replacement}\n"
                        )
                    raise CreateError("injected post-rename substitution")
                if action == "fail":
                    raise CreateError("injected post-rename failure")
                release = os.environ["CALYX_CREATOR_TEST_RELEASE"]
                while not os.path.exists(release):
                    time.sleep(0.01)
PY
  awk -v pre_hook="$pre_hook" -v signal_hook="$signal_hook" -v post_hook="$post_hook" '
    { print }
    $0 == "        # Staging creation is now committed for rollback." {
      while ((getline line < pre_hook) > 0) print line
      close(pre_hook)
    }
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
  rm -- "$pre_hook" "$signal_hook" "$post_hook"
}

inject_capture_pause_hook() {
  local runtime="$1" hook patched
  hook="$runtime.channel-hook"
  patched="$runtime.patched"
  cat >"$hook" <<'SH'
  calyx_gate_test_capture_count=$(( ${calyx_gate_test_capture_count:-0} + 1 ))
  if [[ "${CALYX_CREATOR_TEST_ACTION:-}" == channel \
    && "$calyx_gate_test_capture_count" -eq "${CALYX_CREATOR_TEST_CHANNEL_OPEN:?}" ]]; then
    IFS= read -r channel_stat <"/proc/$BASHPID/stat"
    channel_tail="${channel_stat##*) }"
    read -r -a channel_fields <<<"$channel_tail"
    printf 'pid=%s starttime=%s phase=records-buffered\n' \
      "$BASHPID" "${channel_fields[19]}" >"${CALYX_CREATOR_TEST_READY:?}"
    while [[ ! -e "${CALYX_CREATOR_TEST_RELEASE:?}" ]]; do sleep 0.01 || true; done
  fi
SH
  awk -v hook="$hook" '
    $0 == "  # Both atomic ownership records now reside in parent-owned unlinked bounded file bytes." {
      print
      while ((getline line < hook) > 0) print line
      close(hook)
      next
    }
    { print }
  ' "$runtime" >"$patched"
  mv -T -- "$patched" "$runtime"
  rm -- "$hook"
}

read_setup_pid_starttime() {
  local output_var="$1" pid="$2" raw tail value
  local -a fields=()
  IFS= read -r raw <"/proc/$pid/stat" 2>/dev/null || return 1
  tail="${raw##*) }"
  [[ "$tail" != "$raw" ]] || return 1
  read -r -a fields <<<"$tail"
  value="${fields[19]:-}"
  [[ "${fields[0]:-}" =~ ^[A-Z]$ && "$value" =~ ^[1-9][0-9]*$ ]] || return 1
  printf -v "$output_var" '%s' "$value"
}

run_check_setup_case() {
  local scope="$1" action="$2" key
  key="$scope-$action"
  local repo="$work/setup-$key/calyx/repo" home parent prefix capture_fixture
  local log="$work/setup-$key/gate.log" stages="$work/setup-$key/stages.log"
  local ready="$work/setup-$key/identity-ready" release="$work/setup-$key/identity-release"
  local swap_state="$work/setup-$key/swap-state"
  local expected_rc signal_name="" helper_pid="" helper_starttime="" setup_starttime=""
  local helper_alive_before_release=1 helper_identity_before_signal=1 helper_identity_gone=1
  local setup_identity_gone=0 setup_finished=0 setup_signal_sent=1
  local channel_open=1 actual_starttime=""
  local signal_start_us=0 signal_end_us=0 signal_elapsed_ms=0 hang_path_proven=1
  local channel_proven=1
  mkdir -p "$repo"
  setup_repo "$repo"
  inject_post_rename_hook "$repo/scripts/gate_owned_dir_create.py"
  if [[ "$action" == CHANNEL ]]; then
    capture_fixture="$repo/scripts/gate-runtime-state.sh"
    grep -Fq 'gate-identity-capture.sh' "$capture_fixture" \
      && capture_fixture="$repo/scripts/gate-identity-capture.sh"
    inject_capture_pause_hook "$capture_fixture"
  fi
  parent="$(dirname "$repo")/tmp"
  prefix=check
  if [[ "$scope" == target ]]; then
    parent="$(dirname "$repo")/target-scratch"
    channel_open=2
  fi
  case "$action" in
    fail) expected_rc=125 ;;
    pre_fail) expected_rc=125 ;;
    swap) expected_rc=125 ;;
    HANG) expected_rc=129; signal_name=HUP ;;
    HUP) expected_rc=129; signal_name=HUP ;;
    INT) expected_rc=130; signal_name=INT ;;
    TERM) expected_rc=143; signal_name=TERM ;;
    CHANNEL) expected_rc=129; signal_name=HUP ;;
    *) return 64 ;;
  esac
  setsid env --default-signal=HUP,INT,TERM -u CALYX_FSV_ROOT -u CALYX_GATE_RUN_SLICE -u CALYX_GATE_RUN_SLICE_OWNER_PID \
    -u CALYX_GATE_RUN_SLICE_OWNER_STARTTIME \
    HOME="$repo/home" PATH="$repo/fakebin:$PATH" \
    CALYX_TMP_ROOT="$repo/global-tmp" CALYX_TMP_MIN_FREE_INODE_PERCENT=0 \
    CALYX_TMP_MIN_FREE_BLOCK_PERCENT=0 \
    CALYX_TARGET_SCRATCH_ROOT="$(dirname "$repo")/target-scratch" \
    CALYX_GATE_SIGNAL_GRACE_SECONDS=0.5 FAKE_TMP_LOG="$stages" \
    FAKE_REQUIRE_OWNED_MODE=1 FAKE_FAIL_STAGE= FAKE_DELETE_BASELINE=0 \
    FAKE_BLOCK_STAGE= FAKE_READY_FILE= FAKE_STUBBORN_CHILD=0 FAKE_LEAK_DESCENDANT=0 \
    CALYX_CREATOR_TEST_PARENT="$parent" CALYX_CREATOR_TEST_PREFIX="$prefix" \
    CALYX_CREATOR_TEST_READY="$ready" CALYX_CREATOR_TEST_RELEASE="$release" \
    CALYX_CREATOR_TEST_CHANNEL_OPEN="$channel_open" \
    CALYX_CREATOR_TEST_ACTION="${action,,}" CALYX_CREATOR_TEST_SWAP_STATE="$swap_state" \
    bash "$repo/scripts/check.sh" >"$log" 2>&1 &
  setup_pid=$!
  gate_test_pid_starttime setup_starttime "$setup_pid" || setup_starttime=""
  owned_extra_pgids="$setup_pid"
  gate_test_wait_ready "$ready" || true
  if [[ -s "$ready" ]]; then
    helper_pid="$(sed -n 's/^pid=\([0-9]*\).*/\1/p' "$ready" | head -n1)"
    helper_starttime="$(sed -n 's/.* starttime=\([0-9]*\).*/\1/p' "$ready" | head -n1)"
  fi
  if [[ -n "$signal_name" && -s "$ready" ]]; then
    helper_identity_before_signal=0
    if read_setup_pid_starttime actual_starttime "$helper_pid" \
      && [[ "$actual_starttime" == "$helper_starttime" ]]; then
      helper_identity_before_signal=1
      [[ "$action" != CHANNEL ]] || channel_proven=1
    fi
    signal_start_us="${EPOCHREALTIME/./}"
    setup_signal_sent=0
    gate_test_signal_exact_bounded "$setup_pid" "$setup_starttime" "$signal_name" 0 \
      && setup_signal_sent=1
    if [[ "$action" != HANG ]]; then
      helper_alive_before_release=0
      sleep 0.25
      if read_setup_pid_starttime actual_starttime "$helper_pid" \
        && [[ "$actual_starttime" == "$helper_starttime" ]]; then
        helper_alive_before_release=1
      fi
      : >"$release"
    fi
  fi
  setup_watchdog_marker="$work/setup-$key/watchdog-fired"
  for _ in {1..160}; do
    if gate_test_pid_identity_gone "$setup_pid" "$setup_starttime"; then
      setup_finished=1
      break
    fi
    setup_state="$(awk '/^State:/{print $2}' "/proc/$setup_pid/status" 2>/dev/null || true)"
    [[ "$setup_state" != Z ]] || { setup_finished=1; break; }
    sleep 0.05
  done
  if [[ "$setup_finished" -eq 0 ]]; then
    printf 'fired\n' >"$setup_watchdog_marker"
    gate_test_signal_exact_bounded "$setup_pid" "$setup_starttime" TERM 500 || true
    gate_test_pid_identity_gone "$setup_pid" "$setup_starttime" \
      || gate_test_kill_exact_bounded "$setup_pid" "$setup_starttime" || true
  fi
  set +e
  wait "$setup_pid"; setup_rc=$?
  set -e
  gate_test_pid_identity_gone "$setup_pid" "$setup_starttime" && setup_identity_gone=1
  if [[ "$action" == HANG ]]; then
    signal_end_us="${EPOCHREALTIME/./}"
    if [[ "$signal_start_us" =~ ^[0-9]+$ && "$signal_end_us" =~ ^[0-9]+$ \
      && "$signal_end_us" -ge "$signal_start_us" ]]; then
      signal_elapsed_ms=$(((signal_end_us - signal_start_us) / 1000))
    else
      hang_path_proven=0
    fi
    if ((signal_elapsed_ms < 300 || signal_elapsed_ms > 3000)) \
      || ! grep -Fq 'deferred ownership handoff exceeded signal grace' "$log" \
      || ! grep -Fq 'prepared_create_recovered' "$log"; then
      hang_path_proven=0
    fi
  fi
  setup_tmp_remain="$(find "$(dirname "$repo")/tmp" -mindepth 1 -maxdepth 1 -name 'check.*' -print -quit 2>/dev/null)"
  setup_target_remain="$(find "$(dirname "$repo")/target-scratch" -mindepth 1 -print -quit 2>/dev/null)"
  setup_staging_remain="$(find "$(dirname "$repo")" -name '.calyx-create-*' -print -quit 2>/dev/null)"
  helper_identity_gone=0
  [[ "$helper_pid" =~ ^[1-9][0-9]*$ && "$helper_starttime" =~ ^[1-9][0-9]*$ ]] \
    && gate_test_pid_identity_gone "$helper_pid" "$helper_starttime" && helper_identity_gone=1
  if [[ "$action" == swap ]]; then
    local swap_path_field="" swap_parked_field="" swap_original_field="" swap_replacement_field=""
    local swap_path="" swap_parked="" swap_original_id="" swap_replacement_id=""
    local swap_root_id="" swap_preserved=0 swap_cleanup=0
    read -r swap_path_field swap_parked_field swap_original_field swap_replacement_field \
      <"$swap_state" || true
    swap_path="${swap_path_field#path=}"
    swap_parked="${swap_parked_field#parked=}"
    swap_original_id="${swap_original_field#original_id=}"
    swap_replacement_id="${swap_replacement_field#replacement_id=}"
    swap_root_id="$(stat -c '%d:%i' -- "$parent" 2>/dev/null || true)"
    if [[ "$setup_rc" -eq 125 && "$setup_identity_gone" -eq 1 \
      && "$helper_identity_gone" -eq 1 && -s "$ready" && -s "$swap_state" \
      && "$swap_root_id" =~ ^[0-9]+:[0-9]+$ \
      && "$swap_path" == "$parent"/check.* && "$swap_parked" == "$swap_path.parked" \
      && "$(basename -- "$swap_path")" =~ ^check\.[0-9]+\.[0-9]+$ \
      && -d "$swap_path" && -d "$swap_parked" \
      && "$(stat -c '%d:%i' -- "$swap_path")" == "$swap_replacement_id" \
      && "$(stat -c '%d:%i' -- "$swap_parked")" == "$swap_original_id" \
      && ! -s "$stages" ]] \
      && grep -Fq 'exact rollback failed' "$log"; then swap_preserved=1; fi
    if [[ "$swap_preserved" -eq 1 ]] \
      && python3 "$repo/scripts/gate_safe_rmtree.py" --root "$parent" --root-id "$swap_root_id" \
        --relative "$(basename -- "$swap_path")" --entry-id "$swap_replacement_id" \
        --kind directory --uid "$(id -u)" --forbid-mode-mask 0022 \
        --root-uid "$(id -u)" --root-forbid-mode-mask 0022 --tag setup-swap-replacement \
      && python3 "$repo/scripts/gate_safe_rmtree.py" --root "$parent" --root-id "$swap_root_id" \
        --relative "$(basename -- "$swap_parked")" --entry-id "$swap_original_id" \
        --kind directory --uid "$(id -u)" --forbid-mode-mask 0022 \
        --root-uid "$(id -u)" --root-forbid-mode-mask 0022 --tag setup-swap-original \
      && [[ ! -e "$swap_path" && ! -L "$swap_path" && ! -e "$swap_parked" && ! -L "$swap_parked" ]]; then
      swap_cleanup=1
    fi
    if [[ "$swap_preserved" -eq 1 && "$swap_cleanup" -eq 1 ]]; then
      pass "$scope publication swap preserves both original and replacement identities"
    else
      bad "$scope publication swap rc=$setup_rc mutated a pathname or reached a stage"
      sed -n '1,30p' "$log" | sed 's/^/    swap-log: /'
    fi
    [[ "$setup_identity_gone" -eq 1 && "$swap_cleanup" -eq 1 ]] && owned_extra_pgids=""
    return
  fi
  if [[ "$action" == CHANNEL ]] && grep -Fq 'prepared_create_recovered' "$log"; then
    channel_proven=0
  fi
  if [[ -s "$ready" && "$setup_rc" -eq "$expected_rc" && ! -e "$setup_watchdog_marker" \
    && "$helper_pid" =~ ^[1-9][0-9]*$ && "$helper_identity_before_signal" -eq 1 \
    && "$helper_identity_gone" -eq 1 && "$setup_identity_gone" -eq 1 \
    && "$setup_signal_sent" -eq 1 \
    && "$helper_alive_before_release" -eq 1 && -z "$setup_tmp_remain" \
    && -z "$setup_target_remain" && -z "$setup_staging_remain" && ! -s "$stages" \
    && "$hang_path_proven" -eq 1 \
    && "$channel_proven" -eq 1 ]]; then
    pass "$scope identity $action leaves no unpublished check root"
  else
    bad "$scope identity $action rc=$setup_rc leaked setup state or reached a stage"
    printf '    setup: ready=%s setup_start=%s setup_gone=%s signal_sent=%s helper_pid=%s helper_start=%s helper_alive=%s identity_before=%s identity_gone=%s tmp=%q target=%q staging=%q stages_bytes=%s watchdog=%s signal_elapsed_ms=%s hang_path_proven=%s channel_proven=%s\n' \
      "$(test -s "$ready" && echo yes || echo no)" "$setup_starttime" \
      "$setup_identity_gone" "$setup_signal_sent" "$helper_pid" \
      "$helper_starttime" "$(test -e "/proc/$helper_pid" && echo yes || echo no)" \
      "$helper_identity_before_signal" "$helper_identity_gone" \
      "$setup_tmp_remain" "$setup_target_remain" "$setup_staging_remain" \
      "$(wc -c <"$stages" 2>/dev/null || echo missing)" "$(test -e "$setup_watchdog_marker" && echo fired || echo clear)" \
      "$signal_elapsed_ms" "$hang_path_proven" "$channel_proven"
    sed -n '1,25p' "$log" | sed 's/^/    setup-log: /'
  fi
  [[ "$setup_identity_gone" -eq 0 ]] || owned_extra_pgids=""
}

for setup_scope in tmp target; do
  for setup_action in pre_fail fail swap HUP INT TERM HANG CHANNEL; do
    [[ -z "${CHECK_SETUP_CASE_FILTER:-}" \
      || "$setup_scope-$setup_action" == "$CHECK_SETUP_CASE_FILTER" ]] || continue
    run_check_setup_case "$setup_scope" "$setup_action"
  done
done
