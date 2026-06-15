#!/usr/bin/env bash
# relocate-data.sh — move CALYX_HOME/data/ → /zfs/hot/calyx/ (PH66 T02, #537).
#
# No sudo required (operates on paths already owned by croyse). Idempotent: a
# re-run is a checksum-verified rsync no-op and the sed is a no-op once
# vault_path is already correct. Temp files are staged INSIDE the destination
# dataset so a cross-dataset rename (EXDEV) can never occur.
# POSTURE: single-host, no off-machine replica; whole-host loss is the accepted
#          posture for this deployment (16 §3).
set -euo pipefail

SRC="${CALYX_HOME:-/home/croyse/calyx}/data/"
DST="/zfs/hot/calyx/"
TOML="${CALYX_HOME:-/home/croyse/calyx}/repo/infra/aiwonder/calyx.toml"

[[ -d "$DST" ]] || {
  echo "ERROR: $DST not mounted (run provision-zfs.sh first)"
  exit 1
}
[[ -d "$SRC" ]] || {
  echo "ERROR: $SRC not found"
  exit 1
}

# rsync with --checksum; temp dir INSIDE DST to avoid EXDEV cross-dataset rename
TMPDIR="$DST/.rsync-tmp"
mkdir -p "$TMPDIR"
rsync -av --checksum --temp-dir="$TMPDIR" "$SRC" "$DST"
rm -rf "$TMPDIR"

# Update calyx.toml vault_path (idempotent: no-op once already /zfs/hot/calyx)
sed -i "s|^vault_path = .*|vault_path = \"/zfs/hot/calyx\"|" "$TOML"
echo "Relocation complete. vault_path updated in calyx.toml"
echo "Verify: ls -la $DST"
ls -la "$DST"
