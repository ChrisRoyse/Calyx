#!/usr/bin/env bash
# Source-only real-cgroup handoff containment regression for gate-stage-runner.
[[ "${BASH_SOURCE[0]}" != "$0" ]] || {
  echo "source this module from gate-runner-contract-regression.sh" >&2
  exit 64
}

handoff_read_pid_starttime() {
  local output_var="$1" pid="$2" raw tail value
  local -a fields=()
  IFS= read -r raw <"/proc/$pid/stat" 2>/dev/null || return 1
  tail="${raw##*) }"
  [[ "$tail" != "$raw" ]] || return 1
  read -r -a fields <<<"$tail"
  value="${fields[19]:-}"
  [[ "$value" =~ ^[1-9][0-9]*$ ]] || return 1
  printf -v "$output_var" '%s' "$value"
}

handoff_read_process_cgroup() {
  local output_var="$1" pid="$2" value
  value="$(awk -F: '$1=="0"{print $3}' "/proc/$pid/cgroup" 2>/dev/null)" || return 1
  [[ "$value" == /* ]] || return 1
  printf -v "$output_var" '%s' "$value"
}

handoff_read_unit_field() {
  local output_var="$1" unit="$2" field="$3" value
  value="$(timeout --signal=KILL 2s systemctl --user show "$unit" \
    --property="$field" --value 2>/dev/null)" || return 1
  printf -v "$output_var" '%s' "$value"
}

handoff_kill_exact_bounded() {
  local pid="$1" expected_starttime="$2"
  [[ "$pid" =~ ^[1-9][0-9]*$ && "$expected_starttime" =~ ^[1-9][0-9]*$ ]] \
    || return 125
  python3 - "$pid" "$expected_starttime" <<'PY'
import os
import select
import signal
import sys

pid = int(sys.argv[1])
expected = sys.argv[2].encode("ascii")
try:
    pidfd = os.pidfd_open(pid, 0)
except ProcessLookupError:
    raise SystemExit(0)
except OSError:
    raise SystemExit(125)
try:
    try:
        raw = open(f"/proc/{pid}/stat", "rb").read(8192)
    except FileNotFoundError:
        raise SystemExit(0)
    except OSError:
        raise SystemExit(125)
    end = raw.rfind(b") ")
    fields = raw[end + 2 :].split() if end >= 0 else []
    if len(fields) <= 19:
        raise SystemExit(125)
    if fields[19] != expected:
        raise SystemExit(3)
    signal.pidfd_send_signal(pidfd, signal.SIGKILL)
    poller = select.poll()
    poller.register(pidfd, select.POLLIN)
    if not poller.poll(2000):
        raise SystemExit(125)
finally:
    os.close(pidfd)
PY
}

handoff_stage="$runner_contract_root/handoff-proof-stage.sh"
handoff_ready="$runner_contract_root/handoff-proof-ready"
handoff_migrated="$runner_contract_root/handoff-proof-migrated"
handoff_log="$runner_contract_root/handoff-proof.log"
handoff_escape_log="$runner_contract_root/handoff-escape.log"
cat >"$handoff_stage" <<'STUB'
#!/usr/bin/env bash
set -uo pipefail
trap '' HUP INT TERM
pid="$BASHPID"
IFS= read -r raw <"/proc/$pid/stat"
tail="${raw##*) }"
read -r -a fields <<<"$tail"
control="$(awk -F: '$1=="0"{print $3}' "/proc/$pid/cgroup")"
printf 'pid=%s start=%s control=%s identity=%s\n' "$pid" "${fields[19]}" \
  "$control" "$(stat -c '%d:%i' -- "/sys/fs/cgroup$control")" >"${HANDOFF_READY:?}"
while :; do sleep 60; done
STUB
chmod +x "$handoff_stage"

if ! command -v busctl >/dev/null 2>&1; then
  bad "real handoff regression requires busctl"
  return 0
fi

handoff_escape_unit="calyxgate-handoff-u$(id -u)-p${BASHPID}-e${RANDOM}.scope"
setsid -- systemd-run --user --scope --quiet --collect --slice=app.slice \
  --property=Delegate=yes --unit="$handoff_escape_unit" \
  /usr/bin/tail -f /dev/null >"$handoff_escape_log" 2>&1 &
handoff_escape_sentinel=$!
owned_extra_pgids="$handoff_escape_sentinel"
handoff_escape_control="" handoff_escape_invocation="" handoff_escape_identity=""
handoff_escape_start="" handoff_escape_id="" handoff_escape_active=""
handoff_escape_delegate="" handoff_escape_actual="" handoff_escape_setup=0
handoff_read_pid_starttime handoff_escape_start "$handoff_escape_sentinel" || true
for _ in {1..200}; do
  if handoff_read_unit_field handoff_escape_control "$handoff_escape_unit" ControlGroup \
    && handoff_read_unit_field handoff_escape_id "$handoff_escape_unit" Id \
    && handoff_read_unit_field handoff_escape_active "$handoff_escape_unit" ActiveState \
    && handoff_read_unit_field handoff_escape_delegate "$handoff_escape_unit" Delegate \
    && handoff_read_unit_field handoff_escape_invocation "$handoff_escape_unit" InvocationID; then
    handoff_escape_identity="$(stat -c '%d:%i' -- "/sys/fs/cgroup$handoff_escape_control" 2>/dev/null || true)"
    handoff_read_process_cgroup handoff_escape_actual "$handoff_escape_sentinel" || handoff_escape_actual=""
    [[ "$handoff_escape_id" == "$handoff_escape_unit" \
      && "$handoff_escape_active" == active && "$handoff_escape_delegate" == yes \
      && "$handoff_escape_control" == */app.slice/"$handoff_escape_unit" \
      && "$handoff_escape_actual" == "$handoff_escape_control" \
      && "$handoff_escape_identity" =~ ^[0-9]+:[0-9]+$ \
      && "$handoff_escape_start" =~ ^[1-9][0-9]*$ \
      && "$handoff_escape_invocation" =~ ^[0-9a-f]{32}$ ]] && {
      handoff_escape_setup=1
      break
    }
  fi
  sleep 0.01
done

handoff_start_us="${EPOCHREALTIME/./}"
handoff_rc=125
if [[ "$handoff_escape_setup" -eq 1 ]]; then
  set +e
  env -u CALYX_GATE_RUN_SLICE -u CALYX_GATE_RUN_SLICE_OWNER_PID \
    -u CALYX_GATE_RUN_SLICE_OWNER_STARTTIME RUNNER_SRC="$runner_src" \
    HANDOFF_STAGE="$handoff_stage" HANDOFF_READY="$handoff_ready" \
    HANDOFF_MIGRATED="$handoff_migrated" HANDOFF_LOG="$handoff_log" \
    HANDOFF_ESCAPE_UNIT="$handoff_escape_unit" \
    HANDOFF_ESCAPE_CONTROL="$handoff_escape_control" \
    HANDOFF_ESCAPE_IDENTITY="$handoff_escape_identity" \
    HANDOFF_ESCAPE_INVOCATION="$handoff_escape_invocation" \
    HANDOFF_ESCAPE_SENTINEL_PID="$handoff_escape_sentinel" \
    HANDOFF_ESCAPE_SENTINEL_START="$handoff_escape_start" \
    CALYX_GATE_SIGNAL_GRACE_SECONDS=0.1 \
    timeout --signal=TERM --kill-after=1s 6s bash -c '
      set -uo pipefail
      source "$RUNNER_SRC"
      trap : HUP
      calyx_gate_signals_defer
      owner=$BASHPID
      (
        gate_test_leader="" gate_test_start="" gate_test_original=""
        gate_test_original_id="" log_pid=""
        for _ in {1..200}; do
          if [[ -s "$HANDOFF_READY" ]]; then
            read -r pid_field start_field control_field identity_field <"$HANDOFF_READY" || exit 125
            gate_test_leader="${pid_field#pid=}"
            gate_test_start="${start_field#start=}"
            gate_test_original="${control_field#control=}"
            gate_test_original_id="${identity_field#identity=}"
            log_pid="$(sed -n "s/.*owned_stage_started pid=\([0-9]*\).*/\1/p" "$HANDOFF_LOG" | tail -n1)"
            [[ "$log_pid" == "$gate_test_leader" ]] && break
          fi
          sleep 0.01
        done
        [[ "$gate_test_leader" =~ ^[1-9][0-9]*$ \
          && "$gate_test_start" =~ ^[1-9][0-9]*$ \
          && "$gate_test_original" == */calyxgate-*.scope \
          && "$gate_test_original_id" =~ ^[0-9]+:[0-9]+$ \
          && "$HANDOFF_ESCAPE_CONTROL" != "$gate_test_original" ]] || exit 125
        calyx_gate_pid_starttime actual_start "$gate_test_leader" || exit 125
        calyx_gate_read_process_cgroup actual_control "$gate_test_leader" || exit 125
        actual_original_id="$(stat -c "%d:%i" -- "/sys/fs/cgroup$gate_test_original" 2>/dev/null)" \
          || exit 125
        [[ "$actual_start" == "$gate_test_start" \
          && "$actual_control" == "$gate_test_original" \
          && "$actual_original_id" == "$gate_test_original_id" ]] || exit 125
        python3 - "$gate_test_leader" "$gate_test_start" <<PY
import os
import signal
import sys

pid = int(sys.argv[1])
expected = sys.argv[2].encode("ascii")
try:
    pidfd = os.pidfd_open(pid, 0)
except OSError:
    raise SystemExit(125)
try:
    try:
        with open(f"/proc/{pid}/stat", "rb") as source:
            raw = source.read(8192)
    except OSError:
        raise SystemExit(125)
    end = raw.rfind(b") ")
    fields = raw[end + 2 :].split() if end >= 0 else []
    if len(fields) <= 19 or fields[19] != expected:
        raise SystemExit(125)
    signal.pidfd_send_signal(pidfd, signal.SIGSTOP)
finally:
    os.close(pidfd)
PY
        [[ "$?" -eq 0 ]] || exit 125
        stopped=0
        for _ in {1..100}; do
          state="$(awk '\''/^State:/{print $2}'\'' "/proc/$gate_test_leader/status" 2>/dev/null)"
          [[ "$state" == T || "$state" == t ]] && { stopped=1; break; }
          sleep 0.01
        done
        calyx_gate_pid_starttime actual_start "$gate_test_leader" || exit 125
        calyx_gate_read_process_cgroup actual_control "$gate_test_leader" || exit 125
        actual_original_id="$(stat -c "%d:%i" -- "/sys/fs/cgroup$gate_test_original" 2>/dev/null)" \
          || exit 125
        calyx_gate_read_unit_field escape_id "$HANDOFF_ESCAPE_UNIT" Id || exit 125
        calyx_gate_read_unit_field escape_active "$HANDOFF_ESCAPE_UNIT" ActiveState || exit 125
        calyx_gate_read_unit_field escape_delegate "$HANDOFF_ESCAPE_UNIT" Delegate || exit 125
        calyx_gate_read_unit_field escape_control "$HANDOFF_ESCAPE_UNIT" ControlGroup || exit 125
        calyx_gate_read_unit_field current_invocation "$HANDOFF_ESCAPE_UNIT" InvocationID || exit 125
        current_identity="$(stat -c "%d:%i" -- "/sys/fs/cgroup$HANDOFF_ESCAPE_CONTROL" 2>/dev/null)" \
          || exit 125
        calyx_gate_pid_starttime sentinel_start "$HANDOFF_ESCAPE_SENTINEL_PID" || exit 125
        calyx_gate_read_process_cgroup sentinel_control "$HANDOFF_ESCAPE_SENTINEL_PID" || exit 125
        [[ "$stopped" -eq 1 && "$actual_start" == "$gate_test_start" \
          && "$actual_control" == "$gate_test_original" \
          && "$actual_original_id" == "$gate_test_original_id" \
          && "$escape_id" == "$HANDOFF_ESCAPE_UNIT" && "$escape_active" == active \
          && "$escape_delegate" == yes \
          && "$escape_control" == */app.slice/"$HANDOFF_ESCAPE_UNIT" \
          && "$escape_control" == "$HANDOFF_ESCAPE_CONTROL" \
          && "$current_invocation" == "$HANDOFF_ESCAPE_INVOCATION" \
          && "$current_identity" == "$HANDOFF_ESCAPE_IDENTITY" \
          && "$sentinel_start" == "$HANDOFF_ESCAPE_SENTINEL_START" \
          && "$sentinel_control" == "$HANDOFF_ESCAPE_CONTROL" ]] || exit 125
        busctl --user call org.freedesktop.systemd1 /org/freedesktop/systemd1 \
          org.freedesktop.systemd1.Manager AttachProcessesToUnit ssau \
          "$HANDOFF_ESCAPE_UNIT" "" 1 "$gate_test_leader" >/dev/null || exit 125
        migrated_stopped=0
        for _ in {1..100}; do
          calyx_gate_pid_starttime actual_start "$gate_test_leader" || exit 125
          [[ "$actual_start" == "$gate_test_start" ]] || exit 125
          state="$(awk "/^State:/{print \$2}" "/proc/$gate_test_leader/status" 2>/dev/null)"
          [[ "$state" == T || "$state" == t ]] && { migrated_stopped=1; break; }
          sleep 0.01
        done
        calyx_gate_read_process_cgroup actual_control "$gate_test_leader" || exit 125
        calyx_gate_pid_starttime sentinel_start "$HANDOFF_ESCAPE_SENTINEL_PID" || exit 125
        calyx_gate_read_process_cgroup sentinel_control "$HANDOFF_ESCAPE_SENTINEL_PID" || exit 125
        calyx_gate_read_unit_field current_invocation "$HANDOFF_ESCAPE_UNIT" InvocationID || exit 125
        current_identity="$(stat -c "%d:%i" -- "/sys/fs/cgroup$HANDOFF_ESCAPE_CONTROL" 2>/dev/null)" || exit 125
        [[ "$actual_start" == "$gate_test_start" \
          && "$actual_control" == "$HANDOFF_ESCAPE_CONTROL" \
          && "$sentinel_start" == "$HANDOFF_ESCAPE_SENTINEL_START" \
          && "$sentinel_control" == "$HANDOFF_ESCAPE_CONTROL" \
          && "$current_invocation" == "$HANDOFF_ESCAPE_INVOCATION" \
          && "$current_identity" == "$HANDOFF_ESCAPE_IDENTITY" ]] || exit 125
        calyx_gate_pid_starttime actual_start "$gate_test_leader" || exit 125
        calyx_gate_read_process_cgroup actual_control "$gate_test_leader" || exit 125
        [[ "$migrated_stopped" -eq 1 && "$actual_start" == "$gate_test_start" \
          && "$actual_control" == "$HANDOFF_ESCAPE_CONTROL" ]] || exit 125
        printf "leader=%s start=%s original=%s original_id=%s escape=%s escape_id=%s sentinel=%s invocation=%s\n" \
          "$gate_test_leader" "$gate_test_start" "$gate_test_original" \
          "$gate_test_original_id" "$actual_control" "$current_identity" \
          "$HANDOFF_ESCAPE_SENTINEL_PID" "$current_invocation" >"$HANDOFF_MIGRATED"
        kill -HUP "$owner"
      ) &
      notifier=$!
      set +e
      calyx_gate_run bash "$HANDOFF_STAGE"
      rc=$?
      wait "$notifier" 2>/dev/null || rc=125
      calyx_gate_signals_publish
      calyx_gate_finalize || rc=125
      exit "$rc"
    ' >"$handoff_log" 2>&1
  handoff_rc=$?
  set -e
fi
handoff_end_us="${EPOCHREALTIME/./}"
handoff_elapsed_ms=$(((handoff_end_us - handoff_start_us) / 1000))

handoff_pid_field="" handoff_start_field="" handoff_control_field="" handoff_identity_field=""
migrated_leader="" migrated_start="" migrated_original="" migrated_original_id=""
migrated_escape="" migrated_escape_id="" migrated_sentinel="" migrated_invocation=""
read -r handoff_pid_field handoff_start_field handoff_control_field handoff_identity_field \
  <"$handoff_ready" 2>/dev/null || true
handoff_pid="${handoff_pid_field#pid=}"
handoff_start="${handoff_start_field#start=}"
handoff_control="${handoff_control_field#control=}"
handoff_identity="${handoff_identity_field#identity=}"
read -r migrated_leader migrated_start migrated_original migrated_original_id \
  migrated_escape migrated_escape_id migrated_sentinel migrated_invocation \
  <"$handoff_migrated" 2>/dev/null || true

handoff_leader_gone=0 handoff_original_gone=0 handoff_sentinel_live=0 handoff_unit_same=0
if [[ ! -e "/proc/$handoff_pid" ]]; then
  handoff_leader_gone=1
elif handoff_read_pid_starttime handoff_actual_start "$handoff_pid" \
  && [[ "$handoff_actual_start" != "$handoff_start" ]]; then
  handoff_leader_gone=1
fi
gate_test_cgroup_absent "$handoff_control" "$handoff_identity" && handoff_original_gone=1
if handoff_read_pid_starttime handoff_actual_sentinel_start "$handoff_escape_sentinel" \
  && handoff_read_process_cgroup handoff_actual_sentinel_control "$handoff_escape_sentinel" \
  && [[ "$handoff_actual_sentinel_start" == "$handoff_escape_start" \
    && "$handoff_actual_sentinel_control" == "$handoff_escape_control" ]]; then
  handoff_sentinel_live=1
fi
handoff_read_unit_field handoff_current_id "$handoff_escape_unit" Id || handoff_current_id=""
handoff_read_unit_field handoff_current_invocation "$handoff_escape_unit" InvocationID \
  || handoff_current_invocation=""
handoff_read_unit_field handoff_current_control "$handoff_escape_unit" ControlGroup \
  || handoff_current_control=""
handoff_read_unit_field handoff_current_active "$handoff_escape_unit" ActiveState \
  || handoff_current_active=""
handoff_read_unit_field handoff_current_delegate "$handoff_escape_unit" Delegate \
  || handoff_current_delegate=""
handoff_current_identity="$(stat -c '%d:%i' -- "/sys/fs/cgroup$handoff_escape_control" 2>/dev/null || true)"
if [[ "$handoff_current_id" == "$handoff_escape_unit" \
  && "$handoff_current_invocation" == "$handoff_escape_invocation" \
  && "$handoff_current_control" == "$handoff_escape_control" \
  && "$handoff_current_active" == active && "$handoff_current_delegate" == yes \
  && "$handoff_current_identity" == "$handoff_escape_identity" ]]; then
  handoff_unit_same=1
fi

handoff_contract_ok=0
if [[ "$handoff_escape_setup" -eq 1 && "$handoff_rc" -eq 125 \
  && "$handoff_elapsed_ms" -lt 4000 && "$handoff_leader_gone" -eq 1 \
  && "$handoff_original_gone" -eq 1 && "$handoff_sentinel_live" -eq 1 \
  && "$handoff_unit_same" -eq 1 && "$migrated_leader" == "leader=$handoff_pid" \
  && "$migrated_start" == "start=$handoff_start" \
  && "$migrated_original" == "original=$handoff_control" \
  && "$migrated_original_id" == "original_id=$handoff_identity" \
  && "$migrated_escape" == "escape=$handoff_escape_control" \
  && "$migrated_escape_id" == "escape_id=$handoff_escape_identity" \
  && "$migrated_sentinel" == "sentinel=$handoff_escape_sentinel" \
  && "$migrated_invocation" == "invocation=$handoff_escape_invocation" ]] \
  && grep -Fq 'deferred ownership proof failed: reason=cgroup-mismatch' "$handoff_log"; then
  handoff_contract_ok=1
fi

handoff_cleanup_ok=1
handoff_escape_shape_ok=0
[[ "$handoff_escape_control" == */app.slice/"$handoff_escape_unit" \
  && "$handoff_escape_identity" =~ ^[0-9]+:[0-9]+$ ]] && handoff_escape_shape_ok=1
if [[ "$handoff_escape_setup" -eq 1 && "$handoff_unit_same" -eq 1 ]]; then
  timeout --signal=KILL 2s systemctl --user kill --kill-whom=all --signal=KILL "$handoff_escape_unit" \
    >/dev/null 2>&1 || handoff_cleanup_ok=0
  if ! timeout --signal=KILL 2s systemctl --user stop "$handoff_escape_unit" \
    >/dev/null 2>&1; then
    gate_test_cgroup_absent "$handoff_escape_control" "$handoff_escape_identity" \
      || handoff_cleanup_ok=0
  fi
elif [[ "$handoff_escape_setup" -eq 1 && "$handoff_escape_shape_ok" -eq 1 \
  && "$handoff_current_identity" == "$handoff_escape_identity" ]]; then
  printf '1\n' >"/sys/fs/cgroup$handoff_escape_control/cgroup.kill" 2>/dev/null \
    || handoff_cleanup_ok=0
else
  handoff_cleanup_ok=0
fi
handoff_sentinel_gone=0
if handoff_read_pid_starttime handoff_cleanup_start "$handoff_escape_sentinel"; then
  if [[ "$handoff_escape_start" =~ ^[1-9][0-9]*$ \
    && "$handoff_cleanup_start" == "$handoff_escape_start" ]]; then
    set +e
    handoff_kill_exact_bounded "$handoff_escape_sentinel" "$handoff_escape_start"
    handoff_exact_kill_rc=$?
    set -e
    [[ "$handoff_exact_kill_rc" -eq 0 || "$handoff_exact_kill_rc" -eq 3 ]] \
      || handoff_cleanup_ok=0
  elif [[ "$handoff_escape_start" =~ ^[1-9][0-9]*$ ]]; then
    handoff_sentinel_gone=1
  fi
else
  [[ ! -e "/proc/$handoff_escape_sentinel" ]] && handoff_sentinel_gone=1
fi
for _ in {1..200}; do
  if ! handoff_read_pid_starttime handoff_cleanup_start "$handoff_escape_sentinel"; then
    if [[ ! -e "/proc/$handoff_escape_sentinel" ]]; then
      handoff_sentinel_gone=1
      break
    fi
    handoff_cleanup_ok=0
    break
  elif [[ "$handoff_escape_start" =~ ^[1-9][0-9]*$ \
    && "$handoff_cleanup_start" != "$handoff_escape_start" ]]; then
    handoff_sentinel_gone=1
    break
  fi
  sleep 0.01
done
if [[ "$handoff_sentinel_gone" -eq 1 ]]; then
  wait "$handoff_escape_sentinel" 2>/dev/null || true
else
  handoff_cleanup_ok=0
fi
for _ in {1..200}; do
  gate_test_cgroup_absent "$handoff_escape_control" "$handoff_escape_identity" && break
  sleep 0.01
done
gate_test_cgroup_absent "$handoff_escape_control" "$handoff_escape_identity" \
  || handoff_cleanup_ok=0
if [[ "$handoff_control" == */calyxgate-*.scope \
  && "$handoff_identity" =~ ^[0-9]+:[0-9]+$ \
  && -e "/sys/fs/cgroup$handoff_control" \
  && "$(stat -c '%d:%i' -- "/sys/fs/cgroup$handoff_control" 2>/dev/null || true)" == "$handoff_identity" ]]; then
  printf '1\n' >"/sys/fs/cgroup$handoff_control/cgroup.kill" 2>/dev/null \
    || handoff_cleanup_ok=0
fi
for _ in {1..200}; do
  gate_test_cgroup_absent "$handoff_control" "$handoff_identity" && break
  sleep 0.01
done
gate_test_cgroup_absent "$handoff_control" "$handoff_identity" || handoff_cleanup_ok=0
[[ "$handoff_sentinel_gone" -eq 0 ]] || owned_extra_pgids=""

if [[ "$handoff_contract_ok" -eq 1 && "$handoff_cleanup_ok" -eq 1 ]]; then
  pass "deferred handoff kills the escaped leader while preserving its unrelated cgroup sentinel"
else
  bad "real deferred handoff rc=$handoff_rc elapsed_ms=$handoff_elapsed_ms leader_gone=$handoff_leader_gone original_gone=$handoff_original_gone sentinel_live=$handoff_sentinel_live unit_same=$handoff_unit_same cleanup=$handoff_cleanup_ok"
  tail -30 "$handoff_log" | sed 's/^/    handoff-log: /'
  tail -15 "$handoff_escape_log" | sed 's/^/    escape-log: /'
fi
