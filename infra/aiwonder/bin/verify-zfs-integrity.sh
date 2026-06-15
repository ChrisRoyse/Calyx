#!/usr/bin/env bash
# verify-zfs-integrity.sh — read-only ZFS bit-rot / integrity verifier.
# PRD 30 §1 (Tampering defense: "ZFS checksums + scrub"). Issue #596.
#
# No sudo required: `zfs get` and `zpool status` are readable by any user. This
# is the recurring health gate — run it from the daemon healthcheck or a cron.
#
# SoT (read back independently, never a cached return value):
#   * checksum property : `zfs get -H -o value checksum <dataset>`  (must be on)
#   * pool health       : `zpool status -x <pool>`  (must be "... is healthy")
#   * scrub freshness   : the "scan: ... scrub ... on <date>" line in
#                         `zpool status <pool>`  (must be within SCRUB_MAX_AGE_DAYS)
#
# Trigger (X): run this verifier.
# Outcome (Y): every calyx dataset has checksums on, every backing pool is
#              healthy with zero CKSUM errors, and a scrub ran recently.
#
# Exit codes / fail-closed taxonomy:
#   0                                all checks pass
#   CALYX_ZFS_NOT_AVAILABLE     (3)  zfs/zpool not installed (dev host) — distinct
#                                    code, NOT a silent pass
#   CALYX_ZFS_CHECKSUM_DISABLED (4)  a dataset has checksum=off
#   CALYX_ZFS_POOL_UNHEALTHY    (5)  a pool is degraded / has CKSUM errors
#   CALYX_ZFS_SCRUB_STALE       (6)  no scrub within SCRUB_MAX_AGE_DAYS
#
# Usage: verify-zfs-integrity.sh [dataset ...]
#        (default datasets: hotpool/calyx archive/calyx archive/calyx-restic)
set -euo pipefail

SCRUB_MAX_AGE_DAYS="${SCRUB_MAX_AGE_DAYS:-40}"   # monthly cadence + slack
DATASETS=("$@")
if [ "${#DATASETS[@]}" -eq 0 ]; then
  DATASETS=(hotpool/calyx archive/calyx archive/calyx-restic)
fi

if ! command -v zpool >/dev/null 2>&1 || ! command -v zfs >/dev/null 2>&1; then
  echo "CALYX_ZFS_NOT_AVAILABLE: zfs/zpool not installed on this host" >&2
  exit 3
fi

fail=0

echo "=== checksum property (SoT: zfs get checksum) ==="
for ds in "${DATASETS[@]}"; do
  if ! val="$(zfs get -H -o value checksum "$ds" 2>/dev/null)"; then
    echo "  $ds : MISSING (dataset not found)"; fail=1; continue
  fi
  if [ "$val" = "off" ]; then
    echo "  $ds : checksum=OFF  <- CALYX_ZFS_CHECKSUM_DISABLED"
    fail=4
  else
    echo "  $ds : checksum=$val (on)"
  fi
done

# Unique pools backing the datasets.
mapfile -t POOLS < <(printf '%s\n' "${DATASETS[@]}" | cut -d/ -f1 | LC_ALL=C sort -u)

echo
echo "=== pool health (SoT: zpool status -x) ==="
for pool in "${POOLS[@]}"; do
  if ! status_x="$(zpool status -x "$pool" 2>&1)"; then
    echo "  $pool : ERROR reading status: $status_x"; fail=5; continue
  fi
  if printf '%s' "$status_x" | grep -q "is healthy"; then
    echo "  $pool : $status_x"
  else
    echo "  $pool : UNHEALTHY <- CALYX_ZFS_POOL_UNHEALTHY"
    zpool status -v "$pool" 2>&1 | sed 's/^/      /'
    fail=5
  fi
done

echo
echo "=== scrub freshness (SoT: zpool status scan line) ==="
now_epoch="$(date +%s)"
for pool in "${POOLS[@]}"; do
  scan_line="$(zpool status "$pool" 2>/dev/null | grep -E '^\s*scan:' || true)"
  echo "  $pool : ${scan_line:-<no scan line>}"
  # Extract the trailing "on <date>" for completed scrubs or "since <date>" for
  # an active scrub, then convert to epoch.
  scrub_date="$(printf '%s' "$scan_line" | sed -n 's/.* on \(.*\)$/\1/p')"
  if [ -z "$scrub_date" ]; then
    scrub_date="$(printf '%s' "$scan_line" | sed -n 's/.* since \(.*\)$/\1/p')"
  fi
  if [ -z "$scrub_date" ] || printf '%s' "$scan_line" | grep -q 'none requested' || ! printf '%s' "$scan_line" | grep -q 'scrub'; then
    echo "      no scrub recorded <- CALYX_ZFS_SCRUB_STALE"
    fail=6
    continue
  fi
  if scrub_epoch="$(date -d "$scrub_date" +%s 2>/dev/null)"; then
    age_days=$(( (now_epoch - scrub_epoch) / 86400 ))
    if [ "$age_days" -gt "$SCRUB_MAX_AGE_DAYS" ]; then
      echo "      last scrub ${age_days}d ago (> ${SCRUB_MAX_AGE_DAYS}d) <- CALYX_ZFS_SCRUB_STALE"
      fail=6
    else
      echo "      last scrub ${age_days}d ago (<= ${SCRUB_MAX_AGE_DAYS}d) OK"
    fi
  else
    echo "      could not parse scrub date '$scrub_date' <- CALYX_ZFS_SCRUB_STALE"
    fail=6
  fi
done

echo
if [ "$fail" -eq 0 ]; then
  echo "ZFS INTEGRITY OK: checksums on, pools healthy, scrubs fresh."
  exit 0
fi
echo "ZFS INTEGRITY FAILED (code $fail)."
exit "$fail"
