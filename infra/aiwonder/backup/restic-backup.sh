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
# the include/exclude logic can be FSV'd against a synthetic tree.
set -euo pipefail

REPO="${CALYX_RESTIC_REPO:-/zfs/archive/calyx/restic}"
SOURCE="${CALYX_BACKUP_SOURCE:-/zfs/hot/calyx}"
LOG="$SOURCE/logs/backup-$(date -u +%Y%m%dT%H%M%SZ).log"
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
SNAPSHOT_ID=$(grep -o '"snapshot_id":"[a-f0-9]*"' "$LOG" | tail -1 | cut -d'"' -f4 || true)
echo "$(date -u +%FT%TZ) Snapshot ID: ${SNAPSHOT_ID:-<unknown>}" | tee -a "$LOG"

# Verify the snapshot just created
echo "$(date -u +%FT%TZ) Running restic check" | tee -a "$LOG"
restic check 2>&1 | tee -a "$LOG"

echo "$(date -u +%FT%TZ) Backup complete (snapshot ${SNAPSHOT_ID:-<unknown>})" | tee -a "$LOG"
