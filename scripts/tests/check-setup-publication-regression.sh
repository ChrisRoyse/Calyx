#!/usr/bin/env bash
# Source-only post-create/pre-identity cleanup publication tests for check.sh.
[[ "${BASH_SOURCE[0]}" != "$0" ]] || {
  echo "source this module from check-tmpdir-regression.sh" >&2
  exit 64
}

echo
echo "=================================================================="
echo "CHECK SETUP PUBLICATION - identity failure and deferred signals"
echo "=================================================================="

setup_real_stat="$(command -v stat)"
setup_real_readlink="$(command -v readlink)"
run_check_setup_case() {
  local scope="$1" action="$2" key
  key="$scope-$action"
  local repo="$work/setup-$key/calyx/repo" home parent prefix
  local log="$work/setup-$key/gate.log" stages="$work/setup-$key/stages.log"
  local ready="$work/setup-$key/identity-ready" swap_state="$work/setup-$key/swap-state"
  local expected_rc signal_name="" helper_pid=""
  mkdir -p "$repo"
  setup_repo "$repo"
  parent="$(dirname "$repo")/tmp"
  prefix=check
  if [[ "$scope" == target ]]; then
    parent="$(dirname "$repo")/target-scratch"
  fi
  cat >"$repo/fakebin/stat" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
last="${!#}"
resolved="$("${SETUP_REAL_READLINK:?}" -f -- "$last" 2>/dev/null || printf '%s' "$last")"
if [[ "$*" == *%d:%i* \
  && "$(dirname -- "$resolved")" == "${FAKE_IDENTITY_PARENT:?}" \
  && "$(basename -- "$resolved")" == "${FAKE_IDENTITY_PREFIX:?}."* ]]; then
    printf 'pid=%s path=%s\n' "$BASHPID" "$last" >"${FAKE_IDENTITY_READY:?}"
  if [[ "${FAKE_IDENTITY_ACTION:?}" == swap ]]; then
    parked="$resolved.parked"
    original_id="$("${SETUP_REAL_STAT:?}" -c '%d:%i' -- "$resolved")"
    command -p mv -- "$resolved" "$parked"
    command -p mkdir -- "$resolved"
    chmod 700 "$resolved"
    replacement_id="$("${SETUP_REAL_STAT:?}" -c '%d:%i' -- "$resolved")"
    printf 'path=%s parked=%s original_id=%s replacement_id=%s\n' \
      "$resolved" "$parked" "$original_id" "$replacement_id" >"${FAKE_SWAP_STATE:?}"
    exit 66
  fi
    if [[ "${FAKE_IDENTITY_ACTION:?}" == fail ]]; then exit 66; fi
  sleep 30
fi
exec "${SETUP_REAL_STAT:?}" "$@"
STUB
  chmod +x "$repo/fakebin/stat"
  case "$action" in
    fail) expected_rc=125 ;;
    swap) expected_rc=125 ;;
    HUP) expected_rc=129; signal_name=HUP ;;
    INT) expected_rc=130; signal_name=INT ;;
    TERM) expected_rc=143; signal_name=TERM ;;
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
    SETUP_REAL_STAT="$setup_real_stat" SETUP_REAL_READLINK="$setup_real_readlink" \
    FAKE_IDENTITY_PARENT="$parent" \
    FAKE_IDENTITY_PREFIX="$prefix" FAKE_IDENTITY_READY="$ready" \
    FAKE_IDENTITY_ACTION="${action,,}" FAKE_SWAP_STATE="$swap_state" \
    bash "$repo/scripts/check.sh" >"$log" 2>&1 &
  setup_pid=$!
  owned_extra_pgids="$setup_pid"
  gate_test_wait_ready "$ready" || true
  helper_pid="$(sed -n 's/^pid=\([0-9]*\).*/\1/p' "$ready" | head -n1)"
  if [[ -n "$signal_name" && -s "$ready" ]]; then
    kill -s "$signal_name" "$setup_pid"
  fi
  setup_watchdog_marker="$work/setup-$key/watchdog-fired"
  (
    sleep 8
    printf 'fired\n' >"$setup_watchdog_marker"
    kill -KILL -- "-$setup_pid" 2>/dev/null || true
  ) &
  setup_watchdog=$!
  set +e
  wait "$setup_pid"; setup_rc=$?
  set -e
  kill -KILL "$setup_watchdog" 2>/dev/null || true
  wait "$setup_watchdog" 2>/dev/null || true
  owned_extra_pgids=""
  setup_tmp_remain="$(find "$(dirname "$repo")/tmp" -mindepth 1 -maxdepth 1 -name 'check.*' -print -quit 2>/dev/null)"
  setup_target_remain="$(find "$(dirname "$repo")/target-scratch" -mindepth 1 -print -quit 2>/dev/null)"
  if [[ "$action" == swap ]]; then
    local swap_path_field swap_parked_field swap_original_field swap_replacement_field
    local swap_path swap_parked swap_original_id swap_replacement_id
    read -r swap_path_field swap_parked_field swap_original_field swap_replacement_field \
      <"$swap_state" || true
    swap_path="${swap_path_field#path=}"
    swap_parked="${swap_parked_field#parked=}"
    swap_original_id="${swap_original_field#original_id=}"
    swap_replacement_id="${swap_replacement_field#replacement_id=}"
    if [[ "$setup_rc" -eq 125 && -s "$ready" && -s "$swap_state" \
      && "$helper_pid" =~ ^[1-9][0-9]*$ && ! -e "/proc/$helper_pid" \
      && -d "$swap_path" && -d "$swap_parked" \
      && "$(stat -c '%d:%i' -- "$swap_path")" == "$swap_replacement_id" \
      && "$(stat -c '%d:%i' -- "$swap_parked")" == "$swap_original_id" \
      && ! -s "$stages" ]] \
      && grep -Fq 'CALYX_CHECK_TMPDIR_PARTIAL_CREATE_RECOVERY_REQUIRED' "$log"; then
      pass "$scope publication swap preserves both original and replacement identities"
    else
      bad "$scope publication swap rc=$setup_rc mutated a pathname or reached a stage"
      sed -n '1,30p' "$log" | sed 's/^/    swap-log: /'
    fi
    rm -rf -- "$swap_path" "$swap_parked"
    return
  fi
  if [[ -s "$ready" && "$setup_rc" -eq "$expected_rc" && ! -e "$setup_watchdog_marker" \
    && "$helper_pid" =~ ^[1-9][0-9]*$ && ! -e "/proc/$helper_pid" \
    && -z "$setup_tmp_remain" && -z "$setup_target_remain" && ! -s "$stages" ]] \
    && ! gate_test_group_alive "$setup_pid"; then
    pass "$scope identity $action leaves no unpublished check root"
  else
    bad "$scope identity $action rc=$setup_rc leaked setup state or reached a stage"
    printf '    setup: ready=%s tmp=%q target=%q stages_bytes=%s watchdog=%s\n' \
      "$(test -s "$ready" && echo yes || echo no)" "$setup_tmp_remain" "$setup_target_remain" \
      "$(wc -c <"$stages" 2>/dev/null || echo missing)" "$(test -e "$setup_watchdog_marker" && echo fired || echo clear)"
    sed -n '1,25p' "$log" | sed 's/^/    setup-log: /'
  fi
}

for setup_scope in tmp target; do
  for setup_action in fail swap HUP INT TERM; do
    [[ -z "${CHECK_SETUP_CASE_FILTER:-}" \
      || "$setup_scope-$setup_action" == "$CHECK_SETUP_CASE_FILTER" ]] || continue
    run_check_setup_case "$setup_scope" "$setup_action"
  done
done
