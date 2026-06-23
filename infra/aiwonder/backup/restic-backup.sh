#!/usr/bin/env bash
# restic-backup.sh — Calyx restic backup (PH67 T01, issue #541).
#
# Runs as croyse (no sudo required after PH66 T02 provisioning). Password via
# $CALYX_RESTIC_PASSWORD (from the Infisical-rendered calyx.env) — NEVER
# hardcoded. Single-host posture: no off-machine replica, RPO = backup interval.
# Temp files are staged inside the destination dataset (restic writes its own
# temp under the repo), so no cross-dataset EXDEV rename occurs.
#
# INCLUDE / EXCLUDE rationale — the minimum byte-exact-restore set:
#   REQUIRED (data-bearing, NOT reconstructable from anything else):
#     wal/        write-ahead log — the durability tip; replay source
#     base/       base column-family SSTs — the materialized rows
#     codebooks/  TurboQuant codebooks — without them the quantized vectors are
#                 undecodable, so they are data, not a rebuildable index
#     panel/      panel/lens definitions — the schema of what is stored
#     ledger/     hash-chain ledger — provenance + the verify-chain root of trust
#   EXCLUDED (rebuildable from the REQUIRED set, so excluded to shrink the repo):
#     ann/        ANN/HNSW/DiskANN indexes — rebuilt from base + codebooks
#     kernel/     kernel indexes — rebuilt from base
#     guard/      guard models — retrained/regenerated
#     tmp/        scratch
#     logs/       backup + daemon logs (this log lives here; never back up logs)
#
# REPO/SOURCE default to the aiwonder production paths but are env-overridable so
# the include/exclude logic can be FSV'd against a synthetic tree. SOURCE matches
# infra/aiwonder/calyx.toml's resolved vault path; do not back up the parent data
# root, which can also contain large PH68 scale-run scratch vaults.
set -euo pipefail

REPO="${CALYX_RESTIC_REPO:-/zfs/archive/calyx/restic}"
SOURCE="${CALYX_BACKUP_SOURCE:-/home/croyse/calyx/data/vault}"
LOG="${CALYX_BACKUP_LOG:-/zfs/hot/logs/calyx/backup-$(date -u +%Y%m%dT%H%M%SZ).log}"
mkdir -p "$(dirname "$LOG")"

: "${CALYX_RESTIC_PASSWORD:?CALYX_RESTIC_PASSWORD not set}"
export RESTIC_PASSWORD="$CALYX_RESTIC_PASSWORD"
export RESTIC_REPOSITORY="$REPO"

# Initialize repo if absent (idempotent: skipped once snapshots succeeds)
if ! restic snapshots &>/dev/null; then
  echo "$(date -u +%FT%TZ) Initializing restic repo at $REPO" | tee -a "$LOG"
  restic init 2>&1 | tee -a "$LOG"
fi

# Backup with explicit include/exclude. pipefail makes a restic failure fail the
# pipeline (and set -e exits) — no silent continuation past a backup error.
echo "$(date -u +%FT%TZ) Starting backup" | tee -a "$LOG"
restic backup "$SOURCE" \
  --exclude "$SOURCE/ann" \
  --exclude "$SOURCE/kernel" \
  --exclude "$SOURCE/guard" \
  --exclude "$SOURCE/tmp" \
  --exclude "$SOURCE/logs" \
  --tag calyx \
  --json 2>&1 | tee -a "$LOG"

# Record the snapshot ID from the --json summary so the DR drill can target it.
SNAPSHOT_ID=$(
  python3 - "$LOG" <<'PY'
import json
import sys

snapshot_id = ""
with open(sys.argv[1], "r", encoding="utf-8") as handle:
    for line in handle:
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        if event.get("message_type") == "summary" and event.get("snapshot_id"):
            snapshot_id = event["snapshot_id"]
print(snapshot_id)
PY
)
echo "$(date -u +%FT%TZ) Snapshot ID: ${SNAPSHOT_ID:-<unknown>}" | tee -a "$LOG"

# Verify the snapshot just created
echo "$(date -u +%FT%TZ) Running restic check" | tee -a "$LOG"
restic --retry-lock 30s check 2>&1 | tee -a "$LOG"

echo "$(date -u +%FT%TZ) Backup complete (snapshot ${SNAPSHOT_ID:-<unknown>})" | tee -a "$LOG"
