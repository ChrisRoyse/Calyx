#!/usr/bin/env bash
# Real-Linux Full State Verification for calyx-gatebrokerd (#1455/#1457).
#
# CLI contract (intentionally strict; there are no compatibility fallbacks):
#   calyx-gatectl --config FILE --profile NAME --json health
#   calyx-gatectl --config FILE --profile NAME --json begin-run ...
#   calyx-gatectl --config FILE --profile NAME --json create-object ...
#   calyx-gatectl --config FILE --profile NAME --json exec-stage ... -- ARGV...
#   calyx-gatectl --config FILE --profile NAME --json delete-object ...
#   calyx-gatectl --config FILE --profile NAME --json finish-run ...
#   calyx-gatectl --config FILE --profile NAME --json inspect [--run-id ID]
set -Eeuo pipefail
umask 077

usage() {
  cat >&2 <<'USAGE'
Usage: gatebroker-fsv.sh --ctl PATH --config PATH --profile NAME [options]

Required:
  --ctl PATH              Installed calyx-gatectl executable
  --config PATH           Installed broker configuration
  --profile NAME          Dedicated real-Linux FSV profile

Options:
  --evidence-root PATH    New persistent evidence directory (default: /tmp/...)
  --action-timeout SEC    Per-command timeout, 5..300 (default: 30)
  --reclaim-timeout SEC   Owner-death reclaim timeout, 5..120 (default: 20)
USAGE
  exit 64
}

CTL=""
CONFIG=""
PROFILE=""
EVIDENCE_ROOT=""
ACTION_TIMEOUT=30
RECLAIM_TIMEOUT=20

while (($#)); do
  case "$1" in
    --ctl) [[ $# -ge 2 ]] || usage; CTL="$2"; shift 2 ;;
    --config) [[ $# -ge 2 ]] || usage; CONFIG="$2"; shift 2 ;;
    --profile) [[ $# -ge 2 ]] || usage; PROFILE="$2"; shift 2 ;;
    --evidence-root) [[ $# -ge 2 ]] || usage; EVIDENCE_ROOT="$2"; shift 2 ;;
    --action-timeout) [[ $# -ge 2 ]] || usage; ACTION_TIMEOUT="$2"; shift 2 ;;
    --reclaim-timeout) [[ $# -ge 2 ]] || usage; RECLAIM_TIMEOUT="$2"; shift 2 ;;
    -h|--help) usage ;;
    *) echo "gatebroker-fsv: ERROR unknown argument: $1" >&2; usage ;;
  esac
done

die() {
  echo "gatebroker-fsv: ERROR $*" >&2
  return 1
}

[[ "$(uname -s)" == Linux ]] || die "real Linux is required"
[[ -n "$CTL" && -n "$CONFIG" && -n "$PROFILE" ]] || usage
[[ "$PROFILE" =~ ^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$ ]] \
  || die "profile must be a bounded identifier"
[[ "$ACTION_TIMEOUT" =~ ^[0-9]+$ && "$ACTION_TIMEOUT" -ge 5 && "$ACTION_TIMEOUT" -le 300 ]] \
  || die "--action-timeout must be an integer from 5 through 300"
[[ "$RECLAIM_TIMEOUT" =~ ^[0-9]+$ && "$RECLAIM_TIMEOUT" -ge 5 && "$RECLAIM_TIMEOUT" -le 120 ]] \
  || die "--reclaim-timeout must be an integer from 5 through 120"

if [[ "$CTL" != */* ]]; then
  CTL="$(command -v "$CTL")" || die "calyx-gatectl is not on PATH"
fi
CTL="$(readlink -f -- "$CTL")" || die "cannot resolve --ctl"
CONFIG="$(readlink -f -- "$CONFIG")" || die "cannot resolve --config"
[[ -x "$CTL" && -f "$CTL" ]] || die "--ctl is not an executable regular file: $CTL"
[[ -r "$CONFIG" && -f "$CONFIG" ]] || die "--config is not a readable regular file: $CONFIG"

for required in awk basename bash cat date dirname find getconf grep hostname id jq kill \
  mktemp od ps python3 readlink sha256sum sleep sort stat systemctl tee timeout tr; do
  command -v "$required" >/dev/null 2>&1 || die "$required is required"
done
[[ -x /usr/bin/systemd-run ]] || die "/usr/bin/systemd-run is required"
[[ -x /usr/bin/sleep ]] || die "/usr/bin/sleep is required"
[[ -x /bin/bash ]] || die "/bin/bash is required"

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
sqlite_reader="$script_dir/gatebroker-state-read.py"
[[ -r "$sqlite_reader" ]] || die "missing SQLite readback helper: $sqlite_reader"

run_tag="$(date -u +%Y%m%dT%H%M%SZ)-$$"
if [[ -z "$EVIDENCE_ROOT" ]]; then
  EVIDENCE_ROOT="/tmp/calyx-gatebroker-fsv-$run_tag"
fi
[[ "$EVIDENCE_ROOT" == /* ]] || die "--evidence-root must be absolute"
[[ ! -e "$EVIDENCE_ROOT" ]] || die "evidence root already exists: $EVIDENCE_ROOT"
mkdir -m 0700 -- "$EVIDENCE_ROOT"
EVIDENCE_ROOT="$(readlink -f -- "$EVIDENCE_ROOT")"

exec > >(tee -a "$EVIDENCE_ROOT/fsv.log") 2>&1
echo "CALYX_GATEBROKER_FSV_V1"
echo "started_utc=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
echo "host=$(hostname) kernel=$(uname -srmo) caller_uid=$(id -u) caller_gid=$(id -g)"
echo "ctl=$CTL ctl_sha256=$(sha256sum "$CTL" | awk '{print $1}')"
echo "config=$CONFIG config_sha256=$(sha256sum "$CONFIG" | awk '{print $1}')"
echo "profile=$PROFILE evidence_root=$EVIDENCE_ROOT"
echo "contract=json-v1 distinct-uid no-fallback no-mock"

declare -A OWNER_PIDS=()
declare -A CLIENT_PIDS=()
declare -a ESCAPE_UNITS=()
cleanup_started=0

process_starttime() {
  local pid="$1" raw tail
  local -a fields=()
  [[ "$pid" =~ ^[1-9][0-9]*$ && -r "/proc/$pid/stat" ]] || return 1
  raw="$(<"/proc/$pid/stat")"
  tail="${raw##*) }"
  read -r -a fields <<<"$tail"
  [[ "${fields[19]:-}" =~ ^[1-9][0-9]*$ ]] || return 1
  printf '%s\n' "${fields[19]}"
}

cleanup() {
  local original_status=$? cleanup_status=0 pid unit expected_starttime actual_starttime
  [[ "$cleanup_started" -eq 0 ]] || exit "$original_status"
  cleanup_started=1
  trap - EXIT HUP INT TERM
  set +e
  echo "CLEANUP_BEGIN original_status=$original_status"
  for pid in "${!CLIENT_PIDS[@]}"; do
    expected_starttime="${CLIENT_PIDS[$pid]}"
    if [[ "$pid" =~ ^[1-9][0-9]*$ ]] && kill -0 "$pid" 2>/dev/null; then
      actual_starttime="$(process_starttime "$pid" 2>/dev/null || true)"
      if [[ "$actual_starttime" != "$expected_starttime" ]]; then
        echo "cleanup_client_identity_mismatch pid=$pid expected=$expected_starttime actual=${actual_starttime:-missing}"
        cleanup_status=125
        continue
      fi
      echo "cleanup_client pid=$pid action=TERM"
      kill -TERM "$pid" 2>/dev/null || cleanup_status=125
      for _ in {1..20}; do kill -0 "$pid" 2>/dev/null || break; sleep 0.05; done
      if kill -0 "$pid" 2>/dev/null; then
        kill -KILL "$pid" 2>/dev/null || cleanup_status=125
      fi
    fi
    wait "$pid" 2>/dev/null || true
  done
  for pid in "${!OWNER_PIDS[@]}"; do
    expected_starttime="${OWNER_PIDS[$pid]}"
    if [[ "$pid" =~ ^[1-9][0-9]*$ ]] && kill -0 "$pid" 2>/dev/null; then
      actual_starttime="$(process_starttime "$pid" 2>/dev/null || true)"
      if [[ "$actual_starttime" != "$expected_starttime" ]]; then
        echo "cleanup_owner_identity_mismatch pid=$pid expected=$expected_starttime actual=${actual_starttime:-missing}"
        cleanup_status=125
        continue
      fi
      echo "cleanup_owner pid=$pid action=KILL"
      kill -KILL "$pid" 2>/dev/null || cleanup_status=125
    fi
    wait "$pid" 2>/dev/null || true
  done
  for unit in "${ESCAPE_UNITS[@]}"; do
    [[ "$unit" =~ ^calyx-gbfsv-[A-Za-z0-9-]+\.service$ ]] || {
      echo "cleanup_refused_invalid_unit=$unit"; cleanup_status=125; continue;
    }
    echo "cleanup_exact_unit=$unit"
    systemctl --user stop "$unit" >/dev/null 2>&1 || true
    systemctl stop "$unit" >/dev/null 2>&1 || true
    systemctl --user reset-failed "$unit" >/dev/null 2>&1 || true
    systemctl reset-failed "$unit" >/dev/null 2>&1 || true
  done
  echo "CLEANUP_FINAL_PROCESSES"
  ps -eo pid=,ppid=,uid=,gid=,stat=,cgroup=,args= | grep -F -- "$run_tag" || true
  echo "CLEANUP_FINAL_UNITS"
  systemctl list-units --all --plain --no-legend 'calyx-gbfsv-*' 2>&1 || true
  systemctl --user list-units --all --plain --no-legend 'calyx-gbfsv-*' 2>&1 || true
  echo "CLEANUP_END cleanup_status=$cleanup_status"
  if [[ "$original_status" -eq 0 && "$cleanup_status" -ne 0 ]]; then
    exit "$cleanup_status"
  fi
  exit "$original_status"
}
trap cleanup EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

action_sequence=0
LAST_JSON=""
LAST_STDERR=""
LAST_RC=0

print_command() {
  printf 'command='
  printf '%q ' "$@"
  printf '\n'
}

run_ctl() {
  local label="$1"
  shift
  local directory output errors rc
  action_sequence=$((action_sequence + 1))
  directory="$EVIDENCE_ROOT/actions/$(printf '%03d' "$action_sequence")-$label"
  mkdir -m 0700 -p -- "$directory"
  output="$directory/stdout.json"
  errors="$directory/stderr.log"
  echo "ACTION_BEGIN label=$label sequence=$action_sequence"
  print_command "$CTL" --config "$CONFIG" --profile "$PROFILE" --json "$@"
  set +e
  timeout --foreground --signal=TERM --kill-after=5s "${ACTION_TIMEOUT}s" \
    "$CTL" --config "$CONFIG" --profile "$PROFILE" --json "$@" \
    >"$output" 2>"$errors"
  rc=$?
  set -e
  echo "action_rc=$rc stdout=$output stderr=$errors"
  echo "ACTION_STDOUT"
  cat "$output"
  echo "ACTION_STDERR"
  cat "$errors"
  jq -e 'type == "object"' "$output" >/dev/null \
    || die "action $label did not emit one JSON object"
  LAST_JSON="$output"
  LAST_STDERR="$errors"
  LAST_RC="$rc"
  echo "ACTION_END label=$label sequence=$action_sequence"
}

require_success() {
  local label="$1"
  [[ "$LAST_RC" -eq 0 ]] || die "$label returned $LAST_RC"
  jq -e '.status == "ok"' "$LAST_JSON" >/dev/null \
    || die "$label did not report status=ok"
}

require_rejected() {
  local label="$1" expected_code="$2"
  [[ "$LAST_RC" -ne 0 && "$LAST_RC" -ne 124 && "$LAST_RC" -ne 137 ]] \
    || die "$label did not return a bounded application error (rc=$LAST_RC)"
  jq -e --arg code "$expected_code" \
    '.status == "error" and .error.code == $code and (.error.message | type == "string" and length > 0)' \
    "$LAST_JSON" >/dev/null || die "$label did not emit $expected_code with a message"
}

json_string() {
  local path="$1" expression="$2" label="$3"
  jq -er "$expression | select(type == \"string\" and length > 0)" "$path" \
    || die "missing JSON string: $label"
}

json_uint() {
  local path="$1" expression="$2" label="$3"
  jq -er "$expression | select(type == \"number\" and . >= 0 and floor == .)" "$path" \
    || die "missing JSON unsigned integer: $label"
}

invoke_inspect() {
  local output="$1" errors="$2" run_id="${3:-}" rc
  local -a args=(inspect)
  [[ -z "$run_id" ]] || args+=(--run-id "$run_id")
  set +e
  timeout --foreground --signal=TERM --kill-after=5s "${ACTION_TIMEOUT}s" \
    "$CTL" --config "$CONFIG" --profile "$PROFILE" --json "${args[@]}" \
    >"$output" 2>"$errors"
  rc=$?
  set -e
  [[ "$rc" -eq 0 ]] || {
    cat "$errors" >&2
    die "inspect failed for run=${run_id:-global}: rc=$rc"
  }
  jq -e '.status == "ok"' "$output" >/dev/null \
    || die "inspect returned invalid JSON for run=${run_id:-global}"
}

tree_readback() {
  local root="$1" label="$2" output="$3"
  local file size
  {
    echo "TREE_SOURCE_OF_TRUTH label=$label root=$root"
    stat -Lc 'ROOT dev=%d ino=%i uid=%u gid=%g mode=%a size=%s type=%F path=%n' -- "$root"
    find "$root" -xdev -mindepth 1 -maxdepth 6 \
      -printf 'ENTRY dev=%D ino=%i uid=%U gid=%G mode=%m size=%s type=%y mtime=%T@ path=%p link=%l\n' \
      | LC_ALL=C sort
    while IFS= read -r -d '' file; do
      stat -Lc 'FILE dev=%d ino=%i uid=%u gid=%g mode=%a size=%s path=%n' -- "$file"
      sha256sum -- "$file"
      size="$(stat -Lc %s -- "$file")"
      if [[ "$size" -le 4096 ]]; then
        echo "FILE_HEX path=$file"
        od -Ax -tx1z -v -- "$file"
      fi
    done < <(find "$root" -xdev -mindepth 1 -maxdepth 6 -type f -print0 | LC_ALL=C sort -z)
  } >"$output"
}

BROKER_PID=""
BROKER_UID=""
BROKER_UNIT=""
BROKER_CGROUP=""
STATE_DATABASE=""
JOURNAL_DIRECTORY=""
PRIVATE_ROOT=""
SHARED_ROOT=""
OBJECT_NAME_MAX=""

capture_state() {
  local scenario="$1" phase="$2" run_id="${3:-}"
  shift 3 || true
  local directory inspect_json inspect_errors sqlite_json matches_file token
  directory="$EVIDENCE_ROOT/state/$scenario/$phase"
  mkdir -m 0700 -p -- "$directory"
  inspect_json="$directory/inspect.json"
  inspect_errors="$directory/inspect.stderr"
  invoke_inspect "$inspect_json" "$inspect_errors" "$run_id"
  matches_file="$directory/match-tokens.txt"
  : >"$matches_file"
  [[ -z "$run_id" ]] || printf '%s\n' "$run_id" >>"$matches_file"
  for token in "$@"; do
    [[ -z "$token" ]] || printf '%s\n' "$token" >>"$matches_file"
  done
  local -a sqlite_args=(--database "$STATE_DATABASE" --max-rows 200)
  while IFS= read -r token; do
    [[ -z "$token" ]] || sqlite_args+=(--match "$token")
  done <"$matches_file"
  sqlite_json="$directory/sqlite.json"
  python3 "$sqlite_reader" "${sqlite_args[@]}" >"$sqlite_json"
  tree_readback "$PRIVATE_ROOT" private-root "$directory/private-root.txt"
  tree_readback "$SHARED_ROOT" shared-root "$directory/shared-root.txt"
  tree_readback "$JOURNAL_DIRECTORY" journal-directory "$directory/journal.txt"
  {
    echo "SYSTEMD_SOURCE_OF_TRUTH unit=$BROKER_UNIT"
    systemctl show "$BROKER_UNIT" \
      --property=Id,LoadState,ActiveState,SubState,MainPID,User,UID,ControlGroup,InvocationID,FragmentPath
  } >"$directory/systemd.txt"
  {
    echo "CGROUP_SOURCE_OF_TRUTH path=/sys/fs/cgroup$BROKER_CGROUP"
    stat -Lc 'dev=%d ino=%i uid=%u gid=%g mode=%a type=%F path=%n' \
      -- "/sys/fs/cgroup$BROKER_CGROUP"
    for field in cgroup.events cgroup.procs cgroup.stat; do
      echo "CGROUP_FILE=$field"
      cat "/sys/fs/cgroup$BROKER_CGROUP/$field"
    done
    find "/sys/fs/cgroup$BROKER_CGROUP" -mindepth 1 -maxdepth 5 -type d \
      -printf 'CGROUP_CHILD dev=%D ino=%i path=%p\n' | LC_ALL=C sort
  } >"$directory/cgroup.txt"
  ps -eo pid=,ppid=,uid=,gid=,stat=,lstart=,cgroup=,args= --sort=pid \
    >"$directory/processes-all.txt"
  {
    echo "PROCESS_SOURCE_OF_TRUTH broker_pid=$BROKER_PID"
    grep -E "^[[:space:]]*$BROKER_PID[[:space:]]" "$directory/processes-all.txt" || true
    while IFS= read -r token; do
      [[ -z "$token" ]] || {
        echo "PROCESS_MATCH token=$token"
        grep -F -- "$token" "$directory/processes-all.txt" || true
      }
    done <"$matches_file"
    echo "PROC_STATUS"
    cat "/proc/$BROKER_PID/status"
    echo "PROC_CGROUP"
    cat "/proc/$BROKER_PID/cgroup"
    echo "PROC_CMDLINE"
    tr '\0' ' ' <"/proc/$BROKER_PID/cmdline"
    echo
  } >"$directory/processes.txt"
  {
    echo "SOURCE_OF_TRUTH_SNAPSHOT scenario=$scenario phase=$phase directory=$directory"
    echo "--- inspect.json ---"; cat "$inspect_json"
    echo "--- sqlite.json ---"; cat "$sqlite_json"
    echo "--- private-root.txt ---"; cat "$directory/private-root.txt"
    echo "--- shared-root.txt ---"; cat "$directory/shared-root.txt"
    echo "--- journal.txt ---"; cat "$directory/journal.txt"
    echo "--- systemd.txt ---"; cat "$directory/systemd.txt"
    echo "--- cgroup.txt ---"; cat "$directory/cgroup.txt"
    echo "--- processes.txt ---"; cat "$directory/processes.txt"
  } | tee "$directory/readback.log"
  LAST_SNAPSHOT="$directory"
}

path_readback() {
  local label="$1" path="$2" output="$3"
  {
    echo "PATH_SOURCE_OF_TRUTH label=$label path=$path"
    if [[ -e "$path" || -L "$path" ]]; then
      stat -Lc 'state=present dev=%d ino=%i uid=%u gid=%g mode=%a size=%s type=%F path=%n' -- "$path"
      if [[ -d "$path" ]]; then
        find "$path" -xdev -mindepth 1 -maxdepth 4 \
          -printf 'ENTRY dev=%D ino=%i uid=%U gid=%G mode=%m size=%s type=%y path=%p link=%l\n' \
          | LC_ALL=C sort
      elif [[ -f "$path" ]]; then
        sha256sum -- "$path"
        od -Ax -tx1z -v -- "$path"
      fi
    else
      echo "state=absent path=$path"
    fi
  } | tee "$output"
}

unit_readback() {
  local unit="$1" phase="$2" output="$3"
  {
    echo "UNIT_SOURCE_OF_TRUTH phase=$phase unit=$unit manager=system"
    systemctl show "$unit" \
      --property=Id,LoadState,ActiveState,SubState,MainPID,User,ControlGroup,InvocationID 2>&1 || true
    echo "UNIT_SOURCE_OF_TRUTH phase=$phase unit=$unit manager=user"
    systemctl --user show "$unit" \
      --property=Id,LoadState,ActiveState,SubState,MainPID,User,ControlGroup,InvocationID 2>&1 || true
  } | tee "$output"
}

assert_unit_absent() {
  local unit="$1" manager="$2" value
  if [[ "$manager" == system ]]; then
    value="$(systemctl show "$unit" --property=LoadState --value 2>/dev/null)" \
      || die "cannot query system unit $unit"
  else
    value="$(systemctl --user show "$unit" --property=LoadState --value 2>/dev/null)" \
      || die "cannot query user unit $unit"
  fi
  [[ "$value" == not-found ]] || die "$manager unit unexpectedly exists: $unit load=$value"
}

assert_sqlite_token() {
  local snapshot="$1" token="$2" label="$3"
  jq -e --arg token "$token" \
    '[.tables[].matching_rows[]? | .. | strings | select(. == $token)] | length > 0' \
    "$snapshot/sqlite.json" >/dev/null || die "$label is absent from physical SQLite rows"
}

inspect_projection() {
  jq -Sc '{run: (.run | {id,state,terminal_reason}), objects: [.objects[] | {id,state,path}], stages: [.stages[] | {id,state,exit_code,cgroup}]}' "$1"
}

start_owner() {
  /usr/bin/sleep 300 &
  NEW_OWNER_PID=$!
  NEW_OWNER_STARTTIME="$(process_starttime "$NEW_OWNER_PID")" \
    || die "cannot capture owner starttime: pid=$NEW_OWNER_PID"
  OWNER_PIDS["$NEW_OWNER_PID"]="$NEW_OWNER_STARTTIME"
  echo "OWNER_STARTED pid=$NEW_OWNER_PID starttime=$NEW_OWNER_STARTTIME cgroup=$(awk -F: '$1=="0"{print $3}' "/proc/$NEW_OWNER_PID/cgroup")"
}

stop_owner() {
  local pid="$1"
  if kill -0 "$pid" 2>/dev/null; then
    kill -TERM "$pid"
  fi
  wait "$pid" 2>/dev/null || true
  unset "OWNER_PIDS[$pid]"
  [[ ! -e "/proc/$pid" ]] || die "owner remains after stop: pid=$pid"
  echo "OWNER_ABSENT pid=$pid"
}

begin_run() {
  local label="$1" owner_pid="$2" owner_starttime="$3"
  run_ctl "begin-$label" begin-run --label "$label" \
    --owner-pid "$owner_pid" --owner-starttime "$owner_starttime"
  require_success "begin-run $label"
  NEW_RUN_ID="$(json_string "$LAST_JSON" '.run.id' 'run.id')"
  jq -e '.run.state == "ACTIVE"' "$LAST_JSON" >/dev/null \
    || die "new run is not ACTIVE: $NEW_RUN_ID"
  echo "RUN_STARTED label=$label run_id=$NEW_RUN_ID owner_pid=$owner_pid"
}

create_object() {
  local label="$1" run_id="$2" name="$3"
  run_ctl "create-$label" create-object --run-id "$run_id" --name "$name" \
    --kind directory --mode 0700
  require_success "create-object $label"
  NEW_OBJECT_ID="$(json_string "$LAST_JSON" '.object.id' 'object.id')"
  NEW_OBJECT_PATH="$(json_string "$LAST_JSON" '.object.path' 'object.path')"
  jq -e '.object.state == "PUBLISHED"' "$LAST_JSON" >/dev/null \
    || die "new object is not PUBLISHED: $NEW_OBJECT_ID"
  [[ "$NEW_OBJECT_PATH" == "$SHARED_ROOT"/* ]] \
    || die "created object escaped shared root: $NEW_OBJECT_PATH"
  [[ -d "$NEW_OBJECT_PATH" && ! -L "$NEW_OBJECT_PATH" ]] \
    || die "created object is not a physical directory: $NEW_OBJECT_PATH"
  local actual_dev actual_ino expected_dev expected_ino
  actual_dev="$(stat -Lc %d -- "$NEW_OBJECT_PATH")"
  actual_ino="$(stat -Lc %i -- "$NEW_OBJECT_PATH")"
  expected_dev="$(json_uint "$LAST_JSON" '.object.device' 'object.device')"
  expected_ino="$(json_uint "$LAST_JSON" '.object.inode' 'object.inode')"
  [[ "$actual_dev" == "$expected_dev" && "$actual_ino" == "$expected_ino" ]] \
    || die "object response identity does not match filesystem: $NEW_OBJECT_PATH"
  echo "OBJECT_PUBLISHED id=$NEW_OBJECT_ID path=$NEW_OBJECT_PATH dev=$actual_dev ino=$actual_ino"
}

finish_run() {
  local label="$1" run_id="$2"
  run_ctl "finish-$label" finish-run --run-id "$run_id"
  require_success "finish-run $label"
  jq -e --arg id "$run_id" '.run.id == $id and .run.state == "COMMITTED"' \
    "$LAST_JSON" >/dev/null || die "finish-run did not commit $run_id"
  echo "RUN_COMMITTED run_id=$run_id"
}

# Bootstrap and bind every later readback to the health response.
run_ctl boot-health health
require_success health
HEALTH_JSON="$LAST_JSON"
reported_profile="$(json_string "$HEALTH_JSON" '.profile' 'profile')"
[[ "$reported_profile" == "$PROFILE" ]] \
  || die "health profile mismatch: expected=$PROFILE actual=$reported_profile"
BROKER_PID="$(json_uint "$HEALTH_JSON" '.broker.pid' 'broker.pid')"
BROKER_UID="$(json_uint "$HEALTH_JSON" '.broker.uid' 'broker.uid')"
BROKER_UNIT="$(json_string "$HEALTH_JSON" '.broker.unit' 'broker.unit')"
BROKER_CGROUP="$(json_string "$HEALTH_JSON" '.broker.cgroup' 'broker.cgroup')"
STATE_DATABASE="$(json_string "$HEALTH_JSON" '.storage.database' 'storage.database')"
JOURNAL_DIRECTORY="$(json_string "$HEALTH_JSON" '.storage.journal_directory' 'storage.journal_directory')"
PRIVATE_ROOT="$(json_string "$HEALTH_JSON" '.storage.private_root' 'storage.private_root')"
SHARED_ROOT="$(json_string "$HEALTH_JSON" '.storage.shared_root' 'storage.shared_root')"
OBJECT_NAME_MAX="$(json_uint "$HEALTH_JSON" '.limits.object_name_bytes' 'limits.object_name_bytes')"

[[ "$BROKER_PID" -gt 1 && -d "/proc/$BROKER_PID" ]] || die "broker PID is not alive"
[[ "$BROKER_UID" -ne "$(id -u)" ]] \
  || die "broker UID is not distinct from the gate client UID: $BROKER_UID"
[[ "$(stat -Lc %u -- "/proc/$BROKER_PID")" == "$BROKER_UID" ]] \
  || die "broker PID uid does not match health"
[[ "$BROKER_UNIT" =~ ^[A-Za-z0-9_.@-]+\.service$ ]] || die "invalid broker unit"
[[ "$BROKER_CGROUP" == /* && -d "/sys/fs/cgroup$BROKER_CGROUP" ]] \
  || die "broker cgroup is absent: $BROKER_CGROUP"
actual_broker_cgroup="$(awk -F: '$1=="0"{print $3}' "/proc/$BROKER_PID/cgroup")"
[[ "$actual_broker_cgroup" == "$BROKER_CGROUP" || "$actual_broker_cgroup" == "$BROKER_CGROUP"/* ]] \
  || die "broker process cgroup mismatch: health=$BROKER_CGROUP proc=$actual_broker_cgroup"
for absolute in "$STATE_DATABASE" "$JOURNAL_DIRECTORY" "$PRIVATE_ROOT" "$SHARED_ROOT"; do
  [[ "$absolute" == /* ]] || die "health returned non-absolute storage path: $absolute"
done
[[ -f "$STATE_DATABASE" && -r "$STATE_DATABASE" ]] || die "state database is unreadable"
[[ -d "$JOURNAL_DIRECTORY" && -r "$JOURNAL_DIRECTORY" ]] || die "journal directory is unreadable"
[[ -d "$PRIVATE_ROOT" && -r "$PRIVATE_ROOT" ]] || die "private root is unreadable"
[[ -d "$SHARED_ROOT" && -r "$SHARED_ROOT" ]] || die "shared root is unreadable"
[[ ! -L "$STATE_DATABASE" && ! -L "$JOURNAL_DIRECTORY" \
  && ! -L "$PRIVATE_ROOT" && ! -L "$SHARED_ROOT" ]] \
  || die "source-of-truth paths must not be symlinks"
for canonical in "$STATE_DATABASE" "$JOURNAL_DIRECTORY" "$PRIVATE_ROOT" "$SHARED_ROOT"; do
  [[ "$(readlink -f -- "$canonical")" == "$canonical" ]] \
    || die "source-of-truth path is not canonical: $canonical"
done
[[ "$(stat -Lc %d -- "$PRIVATE_ROOT")" == "$(stat -Lc %d -- "$SHARED_ROOT")" ]] \
  || die "private quarantine and shared root are not on the same filesystem"
[[ "$(stat -Lc %u:%a -- "$PRIVATE_ROOT")" == "$BROKER_UID:700" ]] \
  || die "private root is not broker-owned mode 0700"
fs_name_max="$(getconf NAME_MAX "$SHARED_ROOT")"
[[ "$OBJECT_NAME_MAX" -ge 1 && "$OBJECT_NAME_MAX" -le "$fs_name_max" ]] \
  || die "configured object name limit is invalid: configured=$OBJECT_NAME_MAX fs=$fs_name_max"
python3 "$sqlite_reader" --database "$STATE_DATABASE" --max-rows 200 \
  >"$EVIDENCE_ROOT/bootstrap-sqlite.json"
echo "SOURCE_OF_TRUTH_BINDING database=$STATE_DATABASE journal=$JOURNAL_DIRECTORY private=$PRIVATE_ROOT shared=$SHARED_ROOT broker_pid=$BROKER_PID broker_uid=$BROKER_UID broker_unit=$BROKER_UNIT broker_cgroup=$BROKER_CGROUP"
capture_state bootstrap before "" "$run_tag"

echo
echo "=================================================================="
echo "HAPPY PATH - begin/create/execute/read/delete/finish"
echo "=================================================================="
start_owner
happy_owner="$NEW_OWNER_PID"
happy_owner_starttime="$NEW_OWNER_STARTTIME"
capture_state happy before "" "$run_tag" "$happy_owner"
begin_run "fsv-happy-$run_tag" "$happy_owner" "$happy_owner_starttime"
happy_run="$NEW_RUN_ID"
capture_state happy after-begin "$happy_run" "$run_tag"
assert_sqlite_token "$LAST_SNAPSHOT" "$happy_run" "happy run id"
create_object happy "$happy_run" "happy-$run_tag"
happy_object="$NEW_OBJECT_ID"
happy_path="$NEW_OBJECT_PATH"
path_readback happy-created "$happy_path" "$EVIDENCE_ROOT/state/happy/object-created.txt"
capture_state happy after-create "$happy_run" "$happy_object" "$happy_path"
assert_sqlite_token "$LAST_SNAPSHOT" "$happy_object" "happy object id"

happy_payload="calyx-gatebroker-fsv-happy:$run_tag"
happy_result="$happy_path/result.txt"
[[ ! -e "$happy_result" && ! -L "$happy_result" ]] || die "happy result existed before action"
run_ctl happy-exec exec-stage --run-id "$happy_run" -- \
  /bin/bash -c 'set -euo pipefail; printf "%s\n" "$1" >"$2"; sync -f "$2"' \
  _ "$happy_payload" "$happy_result"
require_success "happy exec-stage"
jq -e '.stage.state == "SUCCEEDED" and .stage.exit_code == 0' "$LAST_JSON" >/dev/null \
  || die "happy stage did not succeed"
happy_stage="$(json_string "$LAST_JSON" '.stage.id' 'stage.id')"
[[ -f "$happy_result" && ! -L "$happy_result" ]] || die "happy stage did not create its result"
expected_hash="$(printf '%s\n' "$happy_payload" | sha256sum | awk '{print $1}')"
actual_hash="$(sha256sum "$happy_result" | awk '{print $1}')"
[[ "$actual_hash" == "$expected_hash" ]] \
  || die "happy result hash mismatch: expected=$expected_hash actual=$actual_hash"
echo "HAPPY_RESULT expected_sha256=$expected_hash actual_sha256=$actual_hash"
path_readback happy-result "$happy_result" "$EVIDENCE_ROOT/state/happy/result.txt"
capture_state happy after-exec "$happy_run" "$happy_object" "$happy_stage" "$happy_result"
assert_sqlite_token "$LAST_SNAPSHOT" "$happy_stage" "happy stage id"

run_ctl happy-delete delete-object --run-id "$happy_run" --object-id "$happy_object"
require_success "happy delete-object"
jq -e --arg id "$happy_object" '.object.id == $id and .object.state == "DELETED"' \
  "$LAST_JSON" >/dev/null || die "happy object did not report DELETED"
[[ ! -e "$happy_path" && ! -L "$happy_path" ]] || die "happy object remains after deletion"
path_readback happy-deleted "$happy_path" "$EVIDENCE_ROOT/state/happy/object-deleted.txt"
capture_state happy after-delete "$happy_run" "$happy_object" "$happy_path"
assert_sqlite_token "$LAST_SNAPSHOT" "$happy_object" "deleted happy object id"
finish_run happy "$happy_run"
capture_state happy after-finish "$happy_run" "$happy_object" "$happy_stage"
jq -e '.run.state == "COMMITTED"' "$LAST_SNAPSHOT/inspect.json" >/dev/null \
  || die "happy inspect is not COMMITTED"
assert_sqlite_token "$LAST_SNAPSHOT" "$happy_run" "committed happy run id"
stop_owner "$happy_owner"

echo
echo "=================================================================="
echo "BOUNDARY EDGES - empty, invalid, exact maximum, over maximum"
echo "=================================================================="
start_owner
edge_owner="$NEW_OWNER_PID"
edge_owner_starttime="$NEW_OWNER_STARTTIME"
begin_run "fsv-edges-$run_tag" "$edge_owner" "$edge_owner_starttime"
edge_run="$NEW_RUN_ID"

capture_state empty-input before "$edge_run" "$run_tag"
empty_before="$(inspect_projection "$LAST_SNAPSHOT/inspect.json")"
run_ctl empty-name create-object --run-id "$edge_run" --name "" --kind directory --mode 0700
require_rejected "empty object name" INVALID_ARGUMENT
capture_state empty-input after "$edge_run" "$run_tag"
empty_after="$(inspect_projection "$LAST_SNAPSHOT/inspect.json")"
[[ "$empty_before" == "$empty_after" ]] || die "empty input changed run/object/stage state"

capture_state invalid-format before "$edge_run" "$run_tag"
invalid_before="$(inspect_projection "$LAST_SNAPSHOT/inspect.json")"
run_ctl invalid-object-id delete-object --run-id "$edge_run" --object-id not-a-valid-object-id
require_rejected "invalid object id" INVALID_ARGUMENT
capture_state invalid-format after "$edge_run" "$run_tag"
invalid_after="$(inspect_projection "$LAST_SNAPSHOT/inspect.json")"
[[ "$invalid_before" == "$invalid_after" ]] || die "invalid id changed run/object/stage state"

printf -v max_name '%*s' "$OBJECT_NAME_MAX" ''
max_name="${max_name// /m}"
[[ "${#max_name}" -eq "$OBJECT_NAME_MAX" ]] || die "could not synthesize exact maximum name"
capture_state maximum-input before "$edge_run" "$run_tag"
create_object maximum "$edge_run" "$max_name"
max_object="$NEW_OBJECT_ID"
max_path="$NEW_OBJECT_PATH"
[[ "$(basename -- "$max_path")" == "$max_name" ]] \
  || die "maximum name was not published byte-exactly"
path_readback maximum-created "$max_path" "$EVIDENCE_ROOT/state/maximum-input/object-created.txt"
capture_state maximum-input after-create "$edge_run" "$max_object" "$max_path"
assert_sqlite_token "$LAST_SNAPSHOT" "$max_object" "maximum object id"
run_ctl maximum-delete delete-object --run-id "$edge_run" --object-id "$max_object"
require_success "maximum delete-object"
[[ ! -e "$max_path" && ! -L "$max_path" ]] || die "maximum object remains after deletion"
capture_state maximum-input after-delete "$edge_run" "$max_object" "$max_path"

oversize_name="${max_name}x"
[[ "${#oversize_name}" -eq $((OBJECT_NAME_MAX + 1)) ]] || die "oversize synthesis failed"
capture_state over-maximum before "$edge_run" "$run_tag"
oversize_before="$(inspect_projection "$LAST_SNAPSHOT/inspect.json")"
run_ctl over-maximum-name create-object --run-id "$edge_run" --name "$oversize_name" \
  --kind directory --mode 0700
require_rejected "over-maximum object name" INVALID_ARGUMENT
capture_state over-maximum after "$edge_run" "$run_tag"
oversize_after="$(inspect_projection "$LAST_SNAPSHOT/inspect.json")"
[[ "$oversize_before" == "$oversize_after" ]] || die "over-maximum input changed state"
finish_run edges "$edge_run"
capture_state boundary-edges after-finish "$edge_run" "$max_object"
stop_owner "$edge_owner"

echo
echo "=================================================================="
echo "MANAGER ESCAPE - stage cannot create sibling user/system units"
echo "=================================================================="
escape_unit="calyx-gbfsv-${run_tag//[^A-Za-z0-9-]/-}-escape.service"
[[ ${#escape_unit} -le 240 ]] || die "escape unit name is too long"
ESCAPE_UNITS+=("$escape_unit")
mkdir -m 0700 -p "$EVIDENCE_ROOT/state/manager-escape"
unit_readback "$escape_unit" before "$EVIDENCE_ROOT/state/manager-escape/unit-before.txt"
assert_unit_absent "$escape_unit" system
assert_unit_absent "$escape_unit" user
start_owner
escape_owner="$NEW_OWNER_PID"
escape_owner_starttime="$NEW_OWNER_STARTTIME"
begin_run "fsv-manager-escape-$run_tag" "$escape_owner" "$escape_owner_starttime"
escape_run="$NEW_RUN_ID"
capture_state manager-escape before "$escape_run" "$escape_unit"
run_ctl manager-escape exec-stage --run-id "$escape_run" -- \
  /usr/bin/systemd-run --user --unit "$escape_unit" --property Type=exec \
  --collect /usr/bin/sleep 120
[[ "$LAST_RC" -ne 0 && "$LAST_RC" -ne 124 && "$LAST_RC" -ne 137 ]] \
  || die "manager escape did not fail with a bounded stage status: rc=$LAST_RC"
jq -e '.status == "error" and .stage.state == "FAILED" and (.stage.exit_code | type == "number" and . != 0)' \
  "$LAST_JSON" >/dev/null || die "manager escape did not return a failed real stage"
escape_stage="$(json_string "$LAST_JSON" '.stage.id' 'manager escape stage.id')"
unit_readback "$escape_unit" after "$EVIDENCE_ROOT/state/manager-escape/unit-after.txt"
assert_unit_absent "$escape_unit" system
assert_unit_absent "$escape_unit" user
ps -eo pid=,ppid=,uid=,cgroup=,args= | grep -F -- "$escape_unit" \
  >"$EVIDENCE_ROOT/state/manager-escape/process-after.txt" || true
[[ ! -s "$EVIDENCE_ROOT/state/manager-escape/process-after.txt" ]] \
  || die "manager escape left a process containing the exact unit token"
capture_state manager-escape after "$escape_run" "$escape_unit" "$escape_stage"
assert_sqlite_token "$LAST_SNAPSHOT" "$escape_stage" "failed manager-escape stage id"
finish_run manager-escape "$escape_run"
capture_state manager-escape after-finish "$escape_run" "$escape_stage"
stop_owner "$escape_owner"

echo
echo "=================================================================="
echo "OWNER DEATH - SIGKILL reclaims stage, object, cgroup, and run"
echo "=================================================================="
start_owner
death_owner="$NEW_OWNER_PID"
death_owner_starttime="$NEW_OWNER_STARTTIME"
capture_state owner-death before "" "$run_tag" "$death_owner"
begin_run "fsv-owner-death-$run_tag" "$death_owner" "$death_owner_starttime"
death_run="$NEW_RUN_ID"
create_object owner-death "$death_run" "owner-death-$run_tag"
death_object="$NEW_OBJECT_ID"
death_path="$NEW_OBJECT_PATH"
death_token="calyx-gatebroker-fsv-stage-$run_tag"
death_action_dir="$EVIDENCE_ROOT/actions/owner-death-running"
mkdir -m 0700 -p "$death_action_dir"
echo "OWNER_DEATH_ACTION_START run=$death_run token=$death_token"
timeout --foreground --signal=TERM --kill-after=5s "$((RECLAIM_TIMEOUT + ACTION_TIMEOUT))s" \
  "$CTL" --config "$CONFIG" --profile "$PROFILE" --json exec-stage \
  --run-id "$death_run" -- /bin/bash -c \
  'exec -a "$1" /usr/bin/sleep 120' _ "$death_token" \
  >"$death_action_dir/stdout.json" 2>"$death_action_dir/stderr.log" &
death_client=$!
death_client_starttime="$(process_starttime "$death_client")" \
  || die "cannot capture owner-death client starttime"
CLIENT_PIDS["$death_client"]="$death_client_starttime"

death_stage_pid=""
death_stage_cgroup=""
deadline=$((SECONDS + RECLAIM_TIMEOUT))
while ((SECONDS < deadline)); do
  invoke_inspect "$death_action_dir/poll.json" "$death_action_dir/poll.stderr" "$death_run"
  if jq -e '.stages | any(.state == "RUNNING")' "$death_action_dir/poll.json" >/dev/null; then
    death_stage_pid="$(jq -er '.stages | map(select(.state == "RUNNING")) | last | .pid | select(type == "number" and . > 1)' "$death_action_dir/poll.json")"
    death_stage_cgroup="$(jq -er '.stages | map(select(.state == "RUNNING")) | last | .cgroup | select(type == "string" and startswith("/"))' "$death_action_dir/poll.json")"
    break
  fi
  kill -0 "$death_client" 2>/dev/null || break
  sleep 0.1
done
[[ "$death_stage_pid" =~ ^[1-9][0-9]*$ && -d "/proc/$death_stage_pid" ]] \
  || die "owner-death stage never reached RUNNING"
[[ -d "/sys/fs/cgroup$death_stage_cgroup" ]] \
  || die "owner-death stage cgroup was not physically published"
tr '\0' ' ' <"/proc/$death_stage_pid/cmdline" | grep -Fq -- "$death_token" \
  || die "running stage process does not contain its synthetic identity token"
actual_stage_cgroup="$(awk -F: '$1=="0"{print $3}' "/proc/$death_stage_pid/cgroup")"
[[ "$actual_stage_cgroup" == "$death_stage_cgroup" ]] \
  || die "running stage process cgroup differs from inspect"
capture_state owner-death action-running "$death_run" "$death_object" "$death_stage_pid" "$death_token"
path_readback owner-death-created "$death_path" "$EVIDENCE_ROOT/state/owner-death/object-before-kill.txt"

echo "OWNER_DEATH_ACTION kill=SIGKILL owner_pid=$death_owner owner_starttime=$death_owner_starttime"
[[ "$(process_starttime "$death_owner")" == "$death_owner_starttime" ]] \
  || die "owner identity changed before SIGKILL"
kill -KILL "$death_owner"
wait "$death_owner" 2>/dev/null || true
unset "OWNER_PIDS[$death_owner]"
[[ ! -e "/proc/$death_owner" ]] || die "owner PID remains after SIGKILL"

deadline=$((SECONDS + RECLAIM_TIMEOUT))
death_terminal=0
while ((SECONDS < deadline)); do
  invoke_inspect "$death_action_dir/reclaim-poll.json" "$death_action_dir/reclaim-poll.stderr" "$death_run"
  if [[ ! -e "/proc/$death_stage_pid" && ! -e "/sys/fs/cgroup$death_stage_cgroup" \
    && ! -e "$death_path" && ! -L "$death_path" ]] \
    && jq -e '.run.state == "ABORTED" and .run.terminal_reason == "OWNER_DIED"' \
      "$death_action_dir/reclaim-poll.json" >/dev/null; then
    death_terminal=1
    break
  fi
  sleep 0.1
done
[[ "$death_terminal" -eq 1 ]] \
  || die "owner death did not reclaim process/cgroup/object/run within ${RECLAIM_TIMEOUT}s"

set +e
wait "$death_client"
death_client_rc=$?
set -e
unset "CLIENT_PIDS[$death_client]"
echo "OWNER_DEATH_EXEC_RESULT rc=$death_client_rc"
cat "$death_action_dir/stdout.json"
cat "$death_action_dir/stderr.log"
[[ "$death_client_rc" -ne 0 && "$death_client_rc" -ne 124 && "$death_client_rc" -ne 137 ]] \
  || die "owner-death exec client did not receive a bounded broker result: rc=$death_client_rc"
jq -e '.status == "error" and .stage.state == "KILLED" and (.stage.exit_code | type == "number" and . != 0)' \
  "$death_action_dir/stdout.json" >/dev/null \
  || die "owner-death exec client did not emit killed-stage JSON"
path_readback owner-death-removed "$death_path" "$EVIDENCE_ROOT/state/owner-death/object-after-kill.txt"
capture_state owner-death after "$death_run" "$death_object" "$death_stage_pid" "$death_token"
jq -e '.run.state == "ABORTED" and .run.terminal_reason == "OWNER_DIED"' \
  "$LAST_SNAPSHOT/inspect.json" >/dev/null || die "owner-death run is not durably aborted"
assert_sqlite_token "$LAST_SNAPSHOT" "$death_run" "owner-death run id"
assert_sqlite_token "$LAST_SNAPSHOT" "$death_object" "owner-death object id"

capture_state final after "" "$run_tag"
echo "completed_utc=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
echo "EVIDENCE_OF_SUCCESS root=$EVIDENCE_ROOT"
echo "ALL GATEBROKER REAL-LINUX FULL-STATE SCENARIOS PASSED"
