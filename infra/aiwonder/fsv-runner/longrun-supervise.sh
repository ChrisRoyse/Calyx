#!/usr/bin/env bash
# Reboot-resilient supervisor for long-running Calyx FSV/ingest jobs (#888).
#
# Contract: this wrapper supervises ONE idempotent, source-of-truth-aware job.
# The wrapped command MUST be safe to re-run from the start — on resume the
# supervisor simply re-invokes it, relying on the job's own SoT-aware
# idempotency (e.g. `calyx ingest ... --idempotent` skips already-ingested
# rows; the FSV synthetic job uses INSERT OR IGNORE keyed by row id). The
# supervisor adds what a bare session process lacks:
#   - a durable systemd user unit (survives session loss; with linger, reboot)
#   - a per-run, source-controlled state file (pinned exe sha, command, SoT
#     handle, completed cursor, heartbeat, next action, resume command)
#   - stale-pidfile / dead-process reclaim
#   - already-complete no-op (idempotent re-entry)
#   - executable-byte pinning re-check (#862) — fail loud on drift
#
# It does NOT mask failures: a non-zero job exit is recorded and propagated so
# the unit's Restart policy (or an operator) acts on a real signal.
#
# Usage: longrun-supervise.sh <run-dir>
#   <run-dir>/spec.json (immutable, written by longrun-register.sh) defines the
#   run. State is written to <run-dir>/state.json. All paths are absolute.
set -u

RUN_DIR="${1:?CALYX_LONGRUN_USAGE: longrun-supervise.sh <run-dir>}"
SPEC="$RUN_DIR/spec.json"
STATE="$RUN_DIR/state.json"
LOG="$RUN_DIR/supervise.log"
PIDFILE="$RUN_DIR/worker.pid"
LOCK="$RUN_DIR/run.lock"

die() { ts="$(date -u +%Y-%m-%dT%H:%M:%SZ)"; printf '[%s] FATAL %s\n' "$ts" "$*" >>"$LOG"; printf 'CALYX_LONGRUN_FAILED: %s\n' "$*" >&2; exit 1; }
note() { printf '[%s] %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$*" >>"$LOG"; }

[ -f "$SPEC" ] || die "spec not found: $SPEC"
command -v jq >/dev/null 2>&1 || die "jq is required for state management"

RUN_ID=$(jq -er '.run_id' "$SPEC")     || die "spec.run_id missing"
EXE=$(jq -er '.exe' "$SPEC")           || die "spec.exe missing"
EXE_SHA=$(jq -er '.exe_sha256' "$SPEC")|| die "spec.exe_sha256 missing"
WORKDIR=$(jq -er '.workdir' "$SPEC")   || die "spec.workdir missing"
SOT_KIND=$(jq -er '.sot.kind' "$SPEC") || die "spec.sot.kind missing"
SOT_HANDLE=$(jq -er '.sot.handle' "$SPEC") || die "spec.sot.handle missing"
RESUME_CMD=$(jq -er '.resume_command' "$SPEC") || die "spec.resume_command missing"
# Job argv is a JSON array; render to a bash array.
mapfile -t ARGV < <(jq -er '.command[]' "$SPEC") || die "spec.command must be a JSON array"
[ "${#ARGV[@]}" -ge 1 ] || die "spec.command is empty"

# --- exit-code-safe state writer (jq to a temp file, atomic mv) ---
set_state() { # set_state key=jsonvalue ...
  local tmp; tmp="$(mktemp "$RUN_DIR/.state.XXXXXX")" || die "mktemp failed"
  local filter=". "
  local args=()
  local i=0
  for kv in "$@"; do
    local k="${kv%%=*}"; local v="${kv#*=}"
    filter+="| .${k} = \$v${i}"
    args+=(--argjson "v${i}" "$v")
    i=$((i+1))
  done
  filter+=' | .last_heartbeat = $hb'
  jq "${args[@]}" --arg hb "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$filter" "$STATE" >"$tmp" \
    || { rm -f "$tmp"; die "state update failed: $*"; }
  mv -f "$tmp" "$STATE" || die "state mv failed"
}

# Initialise state.json on first ever entry.
if [ ! -f "$STATE" ]; then
  jq -n --arg id "$RUN_ID" --arg sk "$SOT_KIND" --arg sh "$SOT_HANDLE" --arg rc "$RESUME_CMD" \
     '{run_id:$id, status:"pending", attempts:0, completed_units:null, total_units:null,
       last_cursor:null, started_at:null, last_heartbeat:null, next_action:"start",
       sot:{kind:$sk, handle:$sh}, resume_command:$rc, last_exit:null}' >"$STATE" \
     || die "state init failed"
fi

# --- single-writer lock; reclaim a stale pidfile from a dead worker ---
exec 9>"$LOCK"
if ! flock -n 9; then
  note "another supervisor holds the lock; exiting (the live one owns this run)"
  exit 0
fi
if [ -f "$PIDFILE" ]; then
  OLDPID="$(cat "$PIDFILE" 2>/dev/null || true)"
  if [ -n "$OLDPID" ] && kill -0 "$OLDPID" 2>/dev/null; then
    die "worker pid $OLDPID still alive but lock free — refusing to double-run"
  fi
  note "reclaiming stale pidfile (pid=${OLDPID:-none} not alive)"
  rm -f "$PIDFILE"
fi

# --- executable-byte pinning re-check (#862): fail loud on drift ---
[ -x "$EXE" ] || die "exe not executable: $EXE"
GOT_SHA="$(sha256sum "$EXE" | awk '{print $1}')"
[ "$GOT_SHA" = "$EXE_SHA" ] || die "exe sha drift: pinned=$EXE_SHA got=$GOT_SHA ($EXE)"

# --- already-complete no-op (idempotent re-entry) ---
STATUS="$(jq -er '.status' "$STATE")"
if [ "$STATUS" = "completed" ]; then
  note "run already completed — no-op"
  set_state next_action='"none"'
  exit 0
fi

ATTEMPTS=$(( $(jq -er '.attempts' "$STATE") + 1 ))
set_state status='"running"' attempts="$ATTEMPTS" \
          started_at="$(jq -c --arg t "$(date -u +%Y-%m-%dT%H:%M:%SZ)" '.started_at // $t' "$STATE")" \
          next_action='"run"'
note "START attempt=$ATTEMPTS run_id=$RUN_ID exe_sha=$EXE_SHA cmd=${ARGV[*]}"

# --- run the wrapped (idempotent) job; record pid for stale-reclaim ---
cd "$WORKDIR" || die "workdir not found: $WORKDIR"
"${ARGV[@]}" >>"$LOG" 2>&1 &
WPID=$!
echo "$WPID" >"$PIDFILE" || die "cannot write pidfile"
note "worker pid=$WPID"

# Heartbeat + cursor while the worker runs. Cursor is read from the real SoT so
# it reflects committed progress, not the wrapper's own bookkeeping.
read_cursor() {
  case "$SOT_KIND" in
    sqlite) sqlite3 "$SOT_HANDLE" 'SELECT COUNT(*) FROM ingested;' 2>/dev/null || echo 0 ;;
    calyx-vault) "$EXE" readback --cf cx_index --vault "$SOT_HANDLE" 2>/dev/null | grep -c . || echo 0 ;;
    *) echo null ;;
  esac
}
while kill -0 "$WPID" 2>/dev/null; do
  CUR="$(read_cursor)"
  set_state last_cursor="${CUR:-null}"
  sleep 2
done
wait "$WPID"; RC=$?
rm -f "$PIDFILE"

CUR="$(read_cursor)"
if [ "$RC" -eq 0 ]; then
  set_state status='"completed"' last_exit=0 completed_units="${CUR:-null}" \
            last_cursor="${CUR:-null}" next_action='"none"'
  note "COMPLETE rc=0 cursor=$CUR"
  exit 0
fi
set_state status='"failed"' last_exit="$RC" last_cursor="${CUR:-null}" next_action='"resume"'
die "worker exited rc=$RC (state=failed; resume: $RESUME_CMD)"
