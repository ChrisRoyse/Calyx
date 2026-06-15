#!/usr/bin/env bash
# zfs-snapshot.sh — enable ZFS auto-snapshots on hotpool/calyx (PH67 T02, #542).
#
# [OPERATOR] Requires sudo. Idempotent: re-setting the properties is a no-op.
# Uses zfs-auto-snapshot if installed; otherwise prints the manual cron fallback
# and fails closed (exit 1) rather than silently leaving snapshots unconfigured.
# POSTURE: single-host; whole-host loss accepted. Snapshots are local only and
# complement (do not replace) the off-dataset restic backup.
set -euo pipefail

if command -v zfs-auto-snapshot &>/dev/null; then
  sudo zfs set com.sun:auto-snapshot=true        hotpool/calyx
  sudo zfs set com.sun:auto-snapshot:hourly=24   hotpool/calyx
  sudo zfs set com.sun:auto-snapshot:daily=7     hotpool/calyx
  sudo zfs set com.sun:auto-snapshot:weekly=4    hotpool/calyx
  echo "ZFS auto-snapshot enabled on hotpool/calyx (hourly=24 daily=7 weekly=4)"
  zfs get com.sun:auto-snapshot hotpool/calyx
else
  echo "zfs-auto-snapshot not installed; add a manual snapshot cron instead:"
  echo "  sudo crontab -e  # add: @hourly zfs snapshot hotpool/calyx@\$(date +%Y%m%dT%H%M%S)"
  exit 1
fi
