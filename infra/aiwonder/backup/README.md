# Calyx backup timer + ZFS snapshots (PH67 T02)

Two durability layers for the configured Calyx vault
(`/home/croyse/calyx/data/vault` → `/zfs/hot/calyx/vault`), both single-host
(no off-machine replica; whole-host loss is the accepted posture):

1. **restic** (off-dataset, to `/zfs/archive/calyx/restic`) — hourly via systemd timer.
2. **ZFS auto-snapshots** (in-pool, crash-consistent point-in-time) — hourly/
   daily/weekly.

## Install the hourly restic timer (operator, sudo)

```bash
sudo cp infra/aiwonder/backup/calyx-backup.{service,timer} /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now calyx-backup.timer
systemctl list-timers --all | grep calyx     # shows NEXT fire time
```

After the first fire, confirm the run and the snapshot:

```bash
journalctl -u calyx-backup --since "1 hour ago" --no-pager   # backup + check OK
restic -r /zfs/archive/calyx/restic snapshots --tag calyx     # >= 1 snapshot
```

`RPO = 1 hour`; `RTO = restic restore + calyx verify-restore` (see the PH67 T04
DR-drill runbook).

## Enable ZFS auto-snapshots (operator, sudo)

```bash
bash infra/aiwonder/backup/zfs-snapshot.sh    # sets com.sun:auto-snapshot props
zfs list -t snapshot hotpool/calyx | head -5  # >= 1 snapshot once a cycle fires
```

If `zfs-auto-snapshot` is not installed the script exits 1 and prints the manual
`cron` fallback — snapshots are never left silently unconfigured.
