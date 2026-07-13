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

capture_parent="$work/identity-capture/calyx"
capture_flag="$work/identity-capture/path-mktemp-invoked"
mkdir -p "$capture_parent" "$work/identity-capture/fakebin"
chmod 700 "$capture_parent"
real_mktemp="$(command -v mktemp)"
cat >"$work/identity-capture/fakebin/mktemp" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
path="$("${REAL_MKTEMP:?}" "$@")"
mv -- "$path" "$path.parked"
printf 'unrelated-replacement\n' >"$path"
printf 'invoked\n' >"${CAPTURE_FLAG:?}"
printf '%s\n' "$path"
STUB
chmod +x "$work/identity-capture/fakebin/mktemp"
set +e
PATH="$work/identity-capture/fakebin:$PATH" REAL_MKTEMP="$real_mktemp" \
  CAPTURE_FLAG="$capture_flag" RUNTIME_SRC="$runtime_src" CAPTURE_PARENT="$capture_parent" \
  bash -c '
    set -euo pipefail
    source "$RUNTIME_SRC"
    calyx_gate_open_identity_capture read_fd write_fd "$CAPTURE_PARENT"
    printf "anonymous-capture\n" >&"$write_fd"
    calyx_gate_close_numeric_fd "$write_fd"
    IFS= read -r record <&"$read_fd"
    calyx_gate_close_numeric_fd "$read_fd"
    wait "$calyx_gate_identity_capture_pid"
    [[ "$record" == anonymous-capture ]]
  ' >"$work/identity-capture/capture.log" 2>&1
capture_rc=$?
set -e
[[ "$capture_rc" -eq 0 && ! -e "$capture_flag" \
  && -z "$(find "$capture_parent" -mindepth 1 -print -quit)" ]] \
  && pass "identity publication uses an anonymous pipe with no pathname unlink window" \
  || bad "identity capture rc=$capture_rc invoked pathname mktemp or left linked state"
