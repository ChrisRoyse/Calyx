#!/usr/bin/env bash
# restic-restore.sh — Calyx DR drill restore (PH67 T04, issue #544).
#
# Usage: bash restic-restore.sh <snapshot-id|latest> <restore-target-dir>
# IMPORTANT: <restore-target-dir> must be inside a ZFS dataset to avoid EXDEV.
#   Recommended: /zfs/archive/calyx/dr-staging  (same pool as the restic repo)
# POSTURE: single-host. RPO = last restic snapshot. RTO = restore + verify time.
#          Whole-host loss is accepted posture for this deployment.
#
# restic recreates the snapshot's ABSOLUTE source path under --target (there is
# no --strip-components), so a vault backed up from
# /home/croyse/calyx/data/vault lands at
# <target>/home/croyse/calyx/data/vault. This script locates the restored vault
# root (the parent of the restored wal/ dir) and prints the exact
# verify-restore command.
set -euo pipefail

SNAPSHOT="${1:?Usage: $0 <snapshot-id|latest> <restore-target-dir>}"
TARGET="${2:?Usage: $0 <snapshot-id|latest> <restore-target-dir>}"
REPO="${CALYX_RESTIC_REPO:-/zfs/archive/calyx/restic}"

: "${CALYX_RESTIC_PASSWORD:?CALYX_RESTIC_PASSWORD not set}"
export RESTIC_PASSWORD="$CALYX_RESTIC_PASSWORD"
export RESTIC_REPOSITORY="$REPO"

# Target must be inside a ZFS dataset so the final rename is intra-dataset (no
# EXDEV). Reject anything outside /zfs/.
case "$TARGET" in
  /zfs/*) ;;
  *)
    echo "ERROR: TARGET must be under /zfs/ to avoid EXDEV on rename"
    exit 1
    ;;
esac

mkdir -p "$TARGET"

echo "Restoring snapshot $SNAPSHOT → $TARGET"
restic restore "$SNAPSHOT" --target "$TARGET" 2>&1

# Locate the restored vault root: the parent of the restored wal/ directory.
WAL_DIR="$(find "$TARGET" -type d -name wal | head -1 || true)"
if [[ -n "$WAL_DIR" ]]; then
  VAULT_DIR="$(dirname "$WAL_DIR")"
else
  VAULT_DIR="$TARGET"
fi

echo "Restore complete. Restored vault root: $VAULT_DIR"
ls -la "$VAULT_DIR"

echo ""
echo "Run: calyx verify-restore --vault $VAULT_DIR --json"
