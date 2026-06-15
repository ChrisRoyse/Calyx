#!/usr/bin/env bash
# enable-zfs-scrub.sh — [OPERATOR] enable scheduled monthly ZFS scrubs for the
# calyx pools. PRD 30 §1 (Tampering: scrub catches silent bit-rot). Issue #596.
#
# OpenZFS ships templated systemd timers (`zfs-scrub-monthly@<pool>.timer`); this
# script just enables them per pool. Monthly cadence is the OpenZFS / FreeBSD
# best-practice default for general-use pools (weekly is for high-churn data);
# override with CADENCE=weekly if a pool's data changes very frequently.
#
# REQUIRES sudo (systemctl enable). Idempotent: a pool whose timer is already
# enabled is skipped with no state change.
# POSTURE: single-host, no off-machine replica (16 §3) — scrub is the only
#          bit-rot defense on these single-vdev pools (no redundancy to self-heal
#          from), so detection + alerting is the goal, not auto-repair.
#
# Verify after running:  systemctl list-timers 'zfs-scrub-*'
#                        infra/aiwonder/bin/verify-zfs-integrity.sh
set -euo pipefail

CADENCE="${CADENCE:-monthly}"   # monthly | weekly
POOLS=("$@")
if [ "${#POOLS[@]}" -eq 0 ]; then
  POOLS=(hotpool archive)
fi

case "$CADENCE" in
  monthly|weekly) ;;
  *) echo "ERROR: CADENCE must be 'monthly' or 'weekly' (got '$CADENCE')" >&2; exit 2 ;;
esac

command -v zpool >/dev/null 2>&1 || { echo "CALYX_ZFS_NOT_AVAILABLE: zpool absent" >&2; exit 3; }
command -v systemctl >/dev/null 2>&1 || { echo "ERROR: systemctl absent" >&2; exit 3; }

for pool in "${POOLS[@]}"; do
  # Confirm the pool actually exists before enabling a timer for it (fail closed).
  if ! zpool list -H -o name "$pool" >/dev/null 2>&1; then
    echo "ERROR: pool '$pool' does not exist — refusing to enable timer" >&2
    exit 4
  fi
  unit="zfs-scrub-${CADENCE}@${pool}.timer"
  if systemctl is-enabled --quiet "$unit" 2>/dev/null; then
    echo "$unit already enabled — skipping"
  else
    echo "enabling $unit ..."
    sudo systemctl enable --now "$unit"
    echo "enabled $unit"
  fi
done

echo
echo "Active scrub timers:"
systemctl list-timers 'zfs-scrub-*' --no-pager 2>/dev/null || true
