#!/usr/bin/env bash
# provision-zfs.sh — create Calyx ZFS datasets (PH66 T02, issue #537).
#
# [OPERATOR] Requires sudo. Idempotent: skips datasets that already exist.
# POSTURE: single-host, no off-machine replica. No HA. Durability = WAL + ZFS
#          snapshots + restic to archive/calyx-restic. Whole-host loss is the
#          accepted posture for this deployment (16 §3).
# Disk reference: reference disks by wwn-/eui- for stable IDs across reboots.
#                 Pools (hotpool, archive) already abstract the device IDs.
set -euo pipefail

create_if_absent() {
  local ds="$1" mp="$2"
  if zfs list "$ds" &>/dev/null; then
    echo "Dataset $ds already exists — skipping"
  else
    sudo zfs create "$ds" -o mountpoint="$mp"
    echo "Created $ds → $mp"
  fi
}

create_if_absent hotpool/calyx        /zfs/hot/calyx
create_if_absent archive/calyx        /zfs/archive/calyx
create_if_absent archive/calyx-restic /zfs/archive/calyx/restic
sudo chown -R croyse:croyse /zfs/hot/calyx /zfs/archive/calyx
echo "ZFS provisioning complete"
zfs list hotpool/calyx archive/calyx archive/calyx-restic
