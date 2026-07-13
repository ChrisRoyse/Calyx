#!/usr/bin/env bash
# Source-only bounded cleanup-prediagnostic producer failure coverage.
[[ "${BASH_SOURCE[0]}" != "$0" ]] || {
  echo "source this module from gate-final-contracts-regression.sh" >&2
  exit 64
}

echo
echo "=================================================================="
echo "CLEANUP PREDIAGNOSTIC - bounded find failure preserves exact bytes"
echo "=================================================================="
prediagnostic_root="$work/prediagnostic/calyx/tmp"
mkdir -p "$(dirname "$prediagnostic_root")"
chmod 755 "$(dirname "$prediagnostic_root")"
prediagnostic_entry="$(bash "$tmpdir_src" create "$prediagnostic_root")"
printf 'prediagnostic-preserved\n' >"$prediagnostic_entry/sentinel"
prediagnostic_sha="$(sha256sum "$prediagnostic_entry/sentinel" | awk '{print $1}')"
prediagnostic_root_id="$(stat -c '%d:%i' -- "$prediagnostic_root")"
prediagnostic_entry_id="$(stat -c '%d:%i' -- "$prediagnostic_entry")"
mkdir -p "$work/prediagnostic/fakebin"
cat >"$work/prediagnostic/fakebin/find" <<'STUB'
#!/usr/bin/env bash
exit 66
STUB
chmod +x "$work/prediagnostic/fakebin/find"
set +e
PATH="$work/prediagnostic/fakebin:$PATH" bash "$tmpdir_src" cleanup \
  "$prediagnostic_root" "$prediagnostic_entry" \
  "$prediagnostic_root_id" "$prediagnostic_entry_id" \
  >"$work/prediagnostic/failure.log" 2>&1
prediagnostic_rc=$?
set -e
[[ "$prediagnostic_rc" -eq 125 && -d "$prediagnostic_entry" \
  && "$(sha256sum "$prediagnostic_entry/sentinel" | awk '{print $1}')" == "$prediagnostic_sha" ]] \
  && grep -Fq 'cannot sample run-dir contents before cleanup' "$work/prediagnostic/failure.log" \
  && pass "bounded cleanup diagnostic producer failure returns 125 and preserves bytes" \
  || bad "cleanup diagnostic failure rc=$prediagnostic_rc mutated or accepted the entry"
bash "$tmpdir_src" cleanup "$prediagnostic_root" "$prediagnostic_entry" \
  "$prediagnostic_root_id" "$prediagnostic_entry_id" >/dev/null
[[ ! -e "$prediagnostic_entry" ]] \
  && pass "preserved cleanup entry remains retryable after diagnostic failure" \
  || bad "prediagnostic recovery retry left the entry"

for publication_scope in run target; do
  publication_home="$work/publication-overflow-$publication_scope/calyx"
  publication_capture="$work/publication-overflow-$publication_scope/capture"
  publication_log="$work/publication-overflow-$publication_scope/action.log"
  publication_probe_log="$work/publication-overflow-$publication_scope/probe.log"
  probe_capture="$work/publication-overflow-$publication_scope/probe-capture"
  mkdir -p "$publication_home"
  chmod 700 "$publication_home"
  if [[ "$publication_scope" == run ]]; then
    publication_root="$publication_home/tmp"
    publication_path="$publication_root/check.publication-overflow"
    publication_probe="$publication_root/check.publication-probe"
    publication_args=(create-at "$publication_root" "$publication_path")
    probe_args=(create-at "$publication_root" "$publication_probe")
  else
    publication_root="$publication_home/target-scratch"
    publication_path="$publication_root/check.publication-overflow"
    publication_probe="$publication_root/check.publication-probe"
    publication_args=(create-target-at "$publication_root" check "$publication_path")
    probe_args=(create-target-at "$publication_root" check "$publication_probe")
  fi
  # A fixed prefill is fragile: the PREPARED/FINAL record lengths carry dev:ino
  # digits that vary per filesystem and per inode, so a constant can leave the
  # FINAL record fitting under the 1024-byte bound (no overflow, the case NOT
  # under test). Measure the real record lengths with a throwaway successful
  # create on this exact root, then size the prefill so PREPARED still fits and
  # FINAL deterministically overflows, tolerant to +/- final_len/2 id drift.
  (
    set +e
    umask 077
    : >"$probe_capture"
    exec 9<>"$probe_capture"
    CALYX_CREATE_ID_FD=9 bash "$tmpdir_src" "${probe_args[@]}"
  ) >"$publication_probe_log" 2>&1
  publication_prepared_len="$(sed -n '1p' "$probe_capture" | wc -c)"
  publication_final_len="$(sed -n '2p' "$probe_capture" | wc -c)"
  rm -f "$probe_capture"
  rm -rf "$publication_probe"
  if ! [[ "$publication_prepared_len" =~ ^[0-9]+$ && "$publication_prepared_len" -gt 1 \
    && "$publication_final_len" =~ ^[0-9]+$ && "$publication_final_len" -gt 1 ]]; then
    bad "$publication_scope publication-overflow probe failed to measure record lengths: prepared=${publication_prepared_len:-missing} final=${publication_final_len:-missing}"
    continue
  fi
  publication_prefill=$(( 1024 - publication_prepared_len - publication_final_len / 2 ))
  if [[ "$publication_prefill" -le 0 ]]; then
    bad "$publication_scope publication-overflow prefill is non-positive: $publication_prefill"
    continue
  fi
  set +e
  (
    set +e
    umask 077
    : >"$publication_capture"
    exec 9<>"$publication_capture"
    rm -- "$publication_capture"
    dd if=/dev/zero bs="$publication_prefill" count=1 status=none >&9
    CALYX_CREATE_ID_FD=9 bash "$tmpdir_src" "${publication_args[@]}"
    publication_inner_rc=$?
    publication_size="$(stat -Lc '%s' -- "/proc/$BASHPID/fd/9")"
    printf 'publication_inner_rc=%s capture_size=%s\n' \
      "$publication_inner_rc" "$publication_size"
    exit "$publication_inner_rc"
  ) >"$publication_log" 2>&1
  publication_rc=$?
  set -e
  publication_size="$(sed -n 's/.*capture_size=\([0-9][0-9]*\).*/\1/p' \
    "$publication_log" | tail -n 1)"
  publication_remain="$(find "$publication_root" -mindepth 1 -print -quit 2>/dev/null)"
  if [[ "$publication_rc" -eq 125 && "$publication_size" =~ ^[0-9]+$ \
    && "$publication_size" -gt "$publication_prefill" && "$publication_size" -le 1024 \
    && -z "$publication_remain" && ! -e "$publication_capture" ]] \
    && ! grep -Fq 'unbound variable' "$publication_log" \
    && grep -Fq 'identity publication file exceeded its bounded append policy' \
      "$publication_log"; then
    pass "$publication_scope final-publication overflow returns 125 after exact rollback"
  else
    bad "$publication_scope final-publication overflow rc=$publication_rc size=${publication_size:-missing} remain=${publication_remain:-none} prefill=$publication_prefill"
  fi
done

capture_parent="$work/identity-capture/calyx"
mkdir -p "$capture_parent"
chmod 700 "$capture_parent"
set +e
RUNTIME_SRC="$runtime_src" CAPTURE_PARENT="$capture_parent" \
  timeout --signal=KILL 5s bash -c '
    set -euo pipefail
    source "$RUNTIME_SRC"
    calyx_gate_open_identity_capture read_fd write_fd "$CAPTURE_PARENT"
    eval "exec {held_fd}>&${write_fd}"
    printf "CALYX_CREATE_ID_V1 root=1:2 entry=3:4\n" >&"$write_fd"
    calyx_gate_close_numeric_fd "$write_fd"
    calyx_gate_read_create_identity "$read_fd" root_id entry_id
    calyx_gate_close_numeric_fd "$read_fd"
    calyx_gate_close_numeric_fd "$held_fd"
    [[ "$root_id" == 1:2 && "$entry_id" == 3:4 ]]
  ' >"$work/identity-capture/capture.log" 2>&1
capture_rc=$?
set -e
[[ "$capture_rc" -eq 0 && -z "$(find "$capture_parent" -mindepth 1 -print -quit)" ]] \
  && pass "identity publication uses an unlinked record file whose EOF cannot be held open" \
  || bad "identity capture rc=$capture_rc left linked channel state"

set +e
RUNTIME_SRC="$runtime_src" CAPTURE_PARENT="$capture_parent" bash -c '
  set -uo pipefail
  source "$RUNTIME_SRC"
  calyx_gate_open_identity_capture read_fd write_fd "$CAPTURE_PARENT"
  printf "CALYX_CREATE_ID_V1 root=1:2 entry=3:4\njunk" >&"$write_fd"
  calyx_gate_close_numeric_fd "$write_fd"
  set +e
  calyx_gate_read_create_identity "$read_fd" root_id entry_id
  parser_rc=$?
  set -e
  calyx_gate_close_numeric_fd "$read_fd"
  [[ "$parser_rc" -eq 125 ]]
' >"$work/identity-capture/trailing.log" 2>&1
trailing_rc=$?
set -e
[[ "$trailing_rc" -eq 0 ]] \
  && pass "identity publication rejects trailing non-newline bytes" \
  || bad "identity trailing-byte parser regression rc=$trailing_rc"

set +e
RUNTIME_SRC="$runtime_src" RUNNER_SRC="$runner_src" CAPTURE_PARENT="$capture_parent" \
  timeout --signal=KILL 4s bash -c '
    set -uo pipefail
    source "$RUNNER_SRC"
    calyx_gate_open_identity_capture read_fd write_fd "$CAPTURE_PARENT"
    python3 -c "import os; os.write(3, b\"CALYX_CREATE_ID_V1 root=1:2 entry=3:4\\n\\0\")" 3>&"$write_fd"
    calyx_gate_close_numeric_fd "$write_fd"
    set +e; calyx_gate_read_create_identity "$read_fd" root_id entry_id; create_rc=$?; set -e
    calyx_gate_close_numeric_fd "$read_fd"
    calyx_gate_open_fd_channel read_fd write_fd "$CAPTURE_PARENT" launch-nul
    python3 -c "import os; os.write(3, b\"CALYX_GATE_LAUNCH_ID_V1 pid=1 starttime=2\\n\\0\")" 3>&"$write_fd"
    calyx_gate_close_numeric_fd "$write_fd"
    set +e; calyx_gate_read_launch_identity "$read_fd" 1 starttime; launch_rc=$?; set -e
    calyx_gate_close_numeric_fd "$read_fd"
    [[ "$create_rc" -eq 125 && "$launch_rc" -eq 125 && "$starttime" == 2 ]]
  ' >"$work/identity-capture/nul.log" 2>&1
nul_rc=$?
set -e
[[ "$nul_rc" -eq 0 ]] && pass "identity and launch parsers reject NUL-contaminated bytes" \
  || bad "NUL-contaminated record regression rc=$nul_rc"

oversized_creator="$work/identity-capture/oversized-creator.sh"
cat >"$oversized_creator" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
for _ in {1..4096}; do printf x >&"${CALYX_CREATE_ID_FD:?}"; done
STUB
chmod +x "$oversized_creator"
set +e
RUNTIME_SRC="$runtime_src" CAPTURE_PARENT="$capture_parent" CREATOR="$oversized_creator" \
  timeout --signal=KILL 5s bash -c '
    set -uo pipefail
    source "$RUNTIME_SRC"
    calyx_gate_run() { "$@"; }
    set +e
    calyx_gate_create_owned_dir ready root_id entry_id "$CAPTURE_PARENT" \
      "$CAPTURE_PARENT" "$CAPTURE_PARENT/never" bash "$CREATOR"
    create_rc=$?
    set -e
    [[ "$create_rc" -ne 0 && "$ready" -eq 0 ]]
  ' >"$work/identity-capture/oversized.log" 2>&1
oversized_rc=$?
set -e
[[ "$oversized_rc" -eq 0 && -z "$(find "$capture_parent" -mindepth 1 -print -quit)" ]] \
  && pass "oversized creator publication is bounded and cannot deadlock its parent" \
  || bad "oversized identity publication rc=$oversized_rc blocked or left linked state"

set +e
RUNNER_SRC="$runner_src" CAPTURE_PARENT="$capture_parent" \
  timeout --signal=KILL 3s bash -c '
    set -uo pipefail
    source "$RUNNER_SRC"
    calyx_gate_record_timeout_seconds=0.1
    calyx_gate_open_fd_channel read_fd write_fd "$CAPTURE_PARENT" launch-timeout
    eval "exec {held_fd}>&${write_fd}"
    printf "CALYX_GATE_LAUNCH_ID_V1 pid=1 starttime=2\n" >&"$write_fd"
    calyx_gate_close_numeric_fd "$write_fd"
    set +e
    calyx_gate_read_launch_identity "$read_fd" 1 starttime
    read_rc=$?
    set -e
    calyx_gate_discard_fd_channel "$held_fd" "$read_fd"
    [[ "$read_rc" -eq 125 && "$starttime" == 2 ]]
  ' >"$work/identity-capture/launch-timeout.log" 2>&1
launch_timeout_rc=$?
set -e
[[ "$launch_timeout_rc" -eq 0 ]] \
  && pass "launch record EOF is bounded while preserving the published identity" \
  || bad "launch identity held-writer regression rc=$launch_timeout_rc"

set +e
RUNNER_SRC="$runner_src" timeout --signal=KILL 5s bash -c '
  set -uo pipefail
  source "$RUNNER_SRC"
  bash -c '\''kill -STOP "$BASHPID"; exec sleep 30'\'' & unrelated=$!
  calyx_gate_pid_starttime unrelated_start "$unrelated" || exit 125
  stopped=0
  for _ in {1..100}; do
    calyx_gate_pid_starttime actual_start "$unrelated" || exit 125
    state="$(awk '\''/^State:/{print $2}'\'' "/proc/$unrelated/status" 2>/dev/null)"
    [[ "$actual_start" == "$unrelated_start" ]] || exit 125
    [[ "$state" == T || "$state" == t ]] && { stopped=1; break; }
    sleep 0.01
  done
  set +e
  calyx_gate_abort_stopped_launcher "$unrelated" "" missing-record
  abort_rc=$?
  set -e
  calyx_gate_pid_starttime actual_start "$unrelated" || exit 125
  state="$(awk '\''/^State:/{print $2}'\'' "/proc/$unrelated/status" 2>/dev/null)"
  [[ "$stopped" -eq 1 && "$actual_start" == "$unrelated_start" \
    && ( "$state" == T || "$state" == t ) ]] || exit 125
  calyx_gate_kill_exact_leader_bounded "$unrelated" "$unrelated_start" || exit 125
  wait "$unrelated" 2>/dev/null || true
  [[ "$abort_rc" -eq 125 ]]
' >"$work/identity-capture/missing-launch-id.log" 2>&1
missing_launch_rc=$?
set -e
[[ "$missing_launch_rc" -eq 0 ]] \
  && pass "missing launch identity preserves the stopped current numeric PID" \
  || bad "missing launch identity wrong-process regression rc=$missing_launch_rc"
