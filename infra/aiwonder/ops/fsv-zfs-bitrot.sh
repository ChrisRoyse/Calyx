#!/usr/bin/env bash
# fsv-zfs-bitrot.sh — [OPERATOR / FSV] prove ZFS checksum DETECTION of bit-rot,
# end to end, on a disposable file-backed test pool. PRD 30 §1. Issue #596.
#
# This NEVER touches the production pools (hotpool/archive). It creates a tiny
# throwaway pool on a regular file, writes a known canary, flips bytes in the
# backing file behind ZFS's back, scrubs, and proves `zpool status` reports the
# corruption — the canonical, safe way to demonstrate bit-rot detection.
#
#   Trigger (X): flip raw bytes in the vdev backing file, then scrub.
#   Outcome (Y): zpool status shows CKSUM > 0 and a permanent error for the
#                corrupted file; reading it returns EIO. ZFS caught the rot.
#
# SoT: `zpool status -v <testpool>` CKSUM column + "errors:" list (read back
#      after the scrub), and the read() of the corrupted file.
#
# REQUIRES sudo (zpool create/scrub/destroy). Single-vdev test pool has no
# redundancy, so ZFS DETECTS but cannot repair — detection is exactly what we
# are proving (production pools are also single-vdev; detection + alert is the
# defense, per enable-zfs-scrub.sh POSTURE note).
#
# Exit codes:
#   0                                bit-rot was injected AND detected (PASS)
#   CALYX_ZFS_BITROT_NOT_DETECTED (7) corruption injected but scrub found none
#                                      (would mean the defense is not working)
set -euo pipefail

command -v zpool >/dev/null 2>&1 || { echo "CALYX_ZFS_NOT_AVAILABLE: zpool absent" >&2; exit 3; }

WORK="$(mktemp -d)"
VDEV="$WORK/vdev.img"
MNT="$WORK/mnt"
POOL="calyxbitrot$$"
CANARY_TEXT="CALYX-BITROT-CANARY-2plus2equals4"

cleanup() {
  sudo zpool destroy "$POOL" 2>/dev/null || true
  rm -rf "$WORK"
}
trap cleanup EXIT

echo "=== 1. create disposable file-backed pool ($POOL) ==="
truncate -s 256M "$VDEV"
mkdir -p "$MNT"
# checksum=on is the default; set it explicitly so the demo is self-documenting.
sudo zpool create -O checksum=on -O compression=off -m "$MNT" "$POOL" "$VDEV"
zpool status -v "$POOL"

echo
echo "=== 2. write a known canary (16 MiB of a known pattern + a marker) ==="
# A large, well-known payload so a corruption span is guaranteed to hit data
# blocks (copies=1 for user data on a single vdev).
sudo bash -c "yes '$CANARY_TEXT' | head -c 16777216 > '$MNT/canary.dat'"
sudo sync
BEFORE_SHA="$(sudo sha256sum "$MNT/canary.dat" | cut -d' ' -f1)"
echo "canary sha256 BEFORE corruption: $BEFORE_SHA"
echo "read-back BEFORE (head): $(sudo head -c 33 "$MNT/canary.dat")"

echo
echo "=== 3. export, then flip raw bytes in the backing file (behind ZFS) ==="
sudo zpool export "$POOL"
# Overwrite a 4 MiB span well past the labels/metadata at the front.
dd if=/dev/urandom of="$VDEV" bs=1M count=4 seek=80 conv=notrunc status=none
echo "injected 4 MiB of random bytes at offset 80 MiB into $VDEV"

echo
echo "=== 4. re-import and scrub ==="
sudo zpool import -d "$WORK" "$POOL"
sudo zpool scrub "$POOL"
# Wait for the scrub to finish (tiny pool — seconds).
for _ in $(seq 1 60); do
  if zpool status "$POOL" | grep -q 'scan: scrub repaired'; then break; fi
  sleep 1
done

echo
echo "=== 5. READ BACK the Source of Truth: zpool status -v ==="
STATUS="$(zpool status -v "$POOL")"
echo "$STATUS"

# CKSUM errors are the verdict. Sum the CKSUM column across vdev lines.
CKSUM_ERRORS="$(printf '%s\n' "$STATUS" \
  | awk '/CKSUM/{seen=1; next} seen && NF>=5 && $1!="" {gsub(/[^0-9]/,"",$5); if($5!="") s+=$5} END{print s+0}')"
echo
echo "CKSUM errors reported: $CKSUM_ERRORS"

echo
echo "=== 6. confirm the corrupted file is now unreadable (EIO) ==="
if sudo cat "$MNT/canary.dat" >/dev/null 2>&1; then
  AFTER="readable"
else
  AFTER="EIO (read refused — corruption confirmed)"
fi
echo "read-back AFTER corruption: $AFTER"

echo
if [ "$CKSUM_ERRORS" -gt 0 ] || printf '%s' "$STATUS" | grep -qE 'permanent errors|canary.dat'; then
  echo "PASS: ZFS DETECTED the injected bit-rot (CKSUM=$CKSUM_ERRORS)."
  exit 0
fi
echo "CALYX_ZFS_BITROT_NOT_DETECTED: scrub reported no checksum errors after injection" >&2
exit 7
