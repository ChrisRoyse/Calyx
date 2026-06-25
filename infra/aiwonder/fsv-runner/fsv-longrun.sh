#!/usr/bin/env bash
# FSV for the #888 reboot-resilient long-run supervisor. Real SoT = a SQLite
# table; synthetic input with a KNOWN expected output (ids 1..N, no gaps/dupes).
# Drives real systemd user units and real process kills; for every edge it
# prints the database state BEFORE and AFTER the action to prove the outcome.
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
SUP="$HERE/longrun-supervise.sh"
REG="$HERE/longrun-register.sh"
JOB="$HERE/fsv-synthetic-ingest.sh"
export CALYX_LONGRUN_ROOT=/home/croyse/calyx/fsv/runs
PASS=0; FAIL=0
ok(){ echo "  PASS: $*"; PASS=$((PASS+1)); }
bad(){ echo "  FAIL: $*"; FAIL=$((FAIL+1)); }
count(){ sqlite3 "$1" 'SELECT COUNT(*) FROM ingested;' 2>/dev/null || echo 0; }
dupes(){ sqlite3 "$1" 'SELECT COUNT(*) FROM (SELECT id FROM ingested GROUP BY id HAVING COUNT(*)>1);' 2>/dev/null || echo ERR; }
st(){ jq -er "$2" "$1/state.json" 2>/dev/null || echo MISSING; }
cleanup_run(){ systemctl --user stop "calyx-longrun@$1.service" 2>/dev/null; systemctl --user disable "calyx-longrun@$1.service" 2>/dev/null; systemctl --user reset-failed "calyx-longrun@$1.service" 2>/dev/null; rm -rf "$CALYX_LONGRUN_ROOT/$1"; }
wait_status(){ # wait_status <run-id> <target> <timeout>
  local d="$CALYX_LONGRUN_ROOT/$1" t=0
  while [ "$t" -lt "$3" ]; do [ "$(st "$d" .status)" = "$2" ] && return 0; sleep 1; t=$((t+1)); done
  return 1
}

echo "================ #888 long-run supervisor FSV ================"

# ---------------- HAPPY PATH ----------------
RID=fsv888-happy
cleanup_run "$RID"; DB="$CALYX_LONGRUN_ROOT/$RID/sot.db"; mkdir -p "$CALYX_LONGRUN_ROOT/$RID"
echo "[HAPPY] before: count=$(count "$DB") (db absent)"
"$REG" --run-id "$RID" --exe "$JOB" --workdir "$HERE" \
  --sot-kind sqlite --sot-handle "$DB" \
  --resume-command "$JOB $DB 5 1" -- "$JOB" "$DB" 5 1 >/dev/null || bad "register happy"
if wait_status "$RID" completed 40; then
  C=$(count "$DB"); D=$(dupes "$DB"); S=$(st "$CALYX_LONGRUN_ROOT/$RID" .status)
  echo "[HAPPY] after: count=$C dupes=$D status=$S cursor=$(st "$CALYX_LONGRUN_ROOT/$RID" .completed_units)"
  [ "$C" = 5 ] && ok "happy ingested exactly 5" || bad "happy count=$C != 5"
  [ "$D" = 0 ] && ok "happy no dupes" || bad "happy dupes=$D"
  [ "$S" = completed ] && ok "happy status=completed" || bad "happy status=$S"
else bad "happy did not reach completed"; fi

# ---------------- EDGE 1: process restart without reboot ----------------
RID=fsv888-restart
cleanup_run "$RID"; DB="$CALYX_LONGRUN_ROOT/$RID/sot.db"; mkdir -p "$CALYX_LONGRUN_ROOT/$RID"
"$REG" --run-id "$RID" --exe "$JOB" --workdir "$HERE" \
  --sot-kind sqlite --sot-handle "$DB" --resume-command "$JOB $DB 8 1" -- "$JOB" "$DB" 8 1 >/dev/null
sleep 4; B=$(count "$DB")
echo "[EDGE1 restart] before restart: count=$B status=$(st "$CALYX_LONGRUN_ROOT/$RID" .status)"
systemctl --user restart "calyx-longrun@$RID.service"
if wait_status "$RID" completed 40; then
  C=$(count "$DB"); D=$(dupes "$DB")
  echo "[EDGE1 restart] after: count=$C dupes=$D status=$(st "$CALYX_LONGRUN_ROOT/$RID" .status) attempts=$(st "$CALYX_LONGRUN_ROOT/$RID" .attempts)"
  [ "$C" = 8 ] && ok "restart resumed to 8" || bad "restart count=$C != 8"
  [ "$D" = 0 ] && ok "restart no dupes (idempotent resume)" || bad "restart dupes=$D"
  [ "$B" -lt 8 ] && ok "restart was genuinely mid-run (B=$B<8)" || bad "restart not mid-run (B=$B)"
else bad "restart did not complete"; fi

# ---------------- EDGE 2: simulated reboot (stop = power loss, then boot) ----------------
RID=fsv888-reboot
cleanup_run "$RID"; DB="$CALYX_LONGRUN_ROOT/$RID/sot.db"; mkdir -p "$CALYX_LONGRUN_ROOT/$RID"
"$REG" --run-id "$RID" --exe "$JOB" --workdir "$HERE" \
  --sot-kind sqlite --sot-handle "$DB" --resume-command "$JOB $DB 6 1" -- "$JOB" "$DB" 6 1 >/dev/null
sleep 3; B=$(count "$DB")
echo "[EDGE2 reboot] power-loss at: count=$B (worker+supervisor killed by stop)"
systemctl --user stop "calyx-longrun@$RID.service"
sleep 1; echo "[EDGE2 reboot] while down: count=$(count "$DB") status=$(st "$CALYX_LONGRUN_ROOT/$RID" .status)"
systemctl --user start "calyx-longrun@$RID.service"   # boot brings the enabled unit back
if wait_status "$RID" completed 40; then
  C=$(count "$DB"); D=$(dupes "$DB")
  echo "[EDGE2 reboot] after boot+resume: count=$C dupes=$D status=$(st "$CALYX_LONGRUN_ROOT/$RID" .status)"
  [ "$C" = 6 ] && ok "reboot resumed to 6" || bad "reboot count=$C != 6"
  [ "$D" = 0 ] && ok "reboot no dupes" || bad "reboot dupes=$D"
else bad "reboot did not complete"; fi

# ---------------- EDGE 3: stale pidfile / dead worker (direct supervisor call) ----------------
RID=fsv888-stalepid
cleanup_run "$RID"; RD="$CALYX_LONGRUN_ROOT/$RID"; DB="$RD/sot.db"; mkdir -p "$RD"
EXE_SHA=$(sha256sum "$JOB" | awk '{print $1}')
jq -n --arg id "$RID" --arg exe "$JOB" --arg sha "$EXE_SHA" --arg wd "$HERE" --arg sh "$DB" \
  --argjson cmd "$(jq -n --arg j "$JOB" --arg db "$DB" '[$j,$db,"4","1"]')" \
  '{run_id:$id,exe:$exe,exe_sha256:$sha,workdir:$wd,sot:{kind:"sqlite",handle:$sh},command:$cmd,resume_command:"manual"}' >"$RD/spec.json"
jq -n --arg id "$RID" '{run_id:$id,status:"running",attempts:1,completed_units:null,total_units:null,last_cursor:null,started_at:null,last_heartbeat:null,next_action:"run",sot:{kind:"sqlite",handle:"x"},resume_command:"manual",last_exit:null}' >"$RD/state.json"
DEADPID=999999; while kill -0 "$DEADPID" 2>/dev/null; do DEADPID=$((DEADPID+1)); done
echo "$DEADPID" >"$RD/worker.pid"
echo "[EDGE3 stalepid] before: pidfile=$(cat "$RD/worker.pid") (dead) status=$(st "$RD" .status) count=$(count "$DB")"
"$SUP" "$RD" >/dev/null 2>&1
C=$(count "$DB"); D=$(dupes "$DB"); P="exists"; [ -f "$RD/worker.pid" ] || P="removed"
echo "[EDGE3 stalepid] after: count=$C dupes=$D status=$(st "$RD" .status) pidfile=$P"
grep -q "reclaiming stale pidfile" "$RD/supervise.log" 2>/dev/null && ok "stale pidfile detected+reclaimed" || bad "no stale-pid reclaim logged"
[ "$C" = 4 ] && ok "stalepid ran to 4 after reclaim" || bad "stalepid count=$C != 4"
[ "$D" = 0 ] && ok "stalepid no dupes" || bad "stalepid dupes=$D"

# ---------------- EDGE 4: already-complete no-op ----------------
RID=fsv888-happy   # reuse the completed happy run
RD="$CALYX_LONGRUN_ROOT/$RID"; DB="$RD/sot.db"
B=$(count "$DB"); echo "[EDGE4 noop] before re-entry: count=$B status=$(st "$RD" .status)"
OUT="$("$SUP" "$RD" 2>&1)"; RC=$?
A=$(count "$DB")
echo "[EDGE4 noop] after re-entry: count=$A rc=$RC next_action=$(st "$RD" .next_action)"
[ "$RC" = 0 ] && ok "noop exited 0" || bad "noop rc=$RC"
[ "$A" = "$B" ] && ok "noop added no rows ($A==$B)" || bad "noop changed count $B->$A"
echo "$OUT$(cat "$RD/supervise.log" 2>/dev/null)" | grep -q "already completed" && ok "noop logged already-completed" || bad "noop did not detect completed"

echo "================ RESULT: PASS=$PASS FAIL=$FAIL ================"
# cleanup
for r in fsv888-happy fsv888-restart fsv888-reboot fsv888-stalepid; do cleanup_run "$r"; done
[ "$FAIL" = 0 ] && echo "FSV-888-GREEN" || echo "FSV-888-RED"
