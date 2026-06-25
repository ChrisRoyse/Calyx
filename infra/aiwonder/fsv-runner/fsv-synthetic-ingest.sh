#!/usr/bin/env bash
# Synthetic, SoT-aware, idempotent worker used to FSV the long-run supervisor
# (#888). It is REAL work against a REAL source of truth (a SQLite table), with
# the same resume contract the supervisor relies on for `calyx ingest
# --idempotent`: re-running it never duplicates a row (INSERT OR IGNORE keyed by
# id) and it continues from the committed cursor. Synthetic INPUT with a KNOWN
# expected output (exactly N rows, ids 1..N, no gaps/dupes) is what lets the FSV
# assert correctness against the database rather than a return value.
#
# Usage: fsv-synthetic-ingest.sh <sqlite-path> <total-rows> [<per-row-sleep-secs>]
set -euo pipefail
DB="${1:?usage: fsv-synthetic-ingest.sh <db> <total> [sleep]}"
TOTAL="${2:?total-rows required}"
SLEEP="${3:-1}"

sqlite3 "$DB" 'CREATE TABLE IF NOT EXISTS ingested(id INTEGER PRIMARY KEY, val TEXT NOT NULL);'
START=$(( $(sqlite3 "$DB" 'SELECT COALESCE(MAX(id),0) FROM ingested;') + 1 ))
echo "synthetic-ingest: db=$DB resume_from=$START total=$TOTAL"
for ((i=START; i<=TOTAL; i++)); do
  sqlite3 "$DB" "INSERT OR IGNORE INTO ingested(id,val) VALUES($i,'row-$i');"
  echo "ingested id=$i count=$(sqlite3 "$DB" 'SELECT COUNT(*) FROM ingested;')"
  sleep "$SLEEP"
done
echo "synthetic-ingest: done count=$(sqlite3 "$DB" 'SELECT COUNT(*) FROM ingested;')"
