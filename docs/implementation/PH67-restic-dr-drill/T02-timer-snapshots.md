# PH67 · T02 — Restic systemd timer + ZFS auto-snapshots

| Field | Value |
|---|---|
| **Phase** | PH67 — restic backup + DR drill |
| **Stage** | S16 — Server & Deployment |
| **Crate** | `infra` (no Rust crate) |
| **Files** | `infra/aiwonder/backup/calyx-backup.timer`, `infra/aiwonder/backup/calyx-backup.service`, `infra/aiwonder/backup/zfs-snapshot.sh` |
| **Depends on** | T01 (restic-backup.sh passing FSV) |
| **Axioms** | A15 |
| **PRD** | `dbprdplans/16 §7` |

## Goal

Write the systemd timer and one-shot service units that run `restic-backup.sh`
on a 1-hour schedule (following the `leapable-restic` pattern from the existing
aiwonder service ecosystem), and the operator script that sets up ZFS
auto-snapshots on `hotpool/calyx`. The timer install is an operator/sudo step.
The auto-snapshot script configures periodic ZFS snapshots (hourly, daily,
weekly) for crash-consistent point-in-time recovery. Both are idempotent.

> **[OPERATOR] steps:** `systemctl enable/start calyx-backup.timer` and
> `zfs set com.sun:auto-snapshot=true hotpool/calyx` require sudo.

## Build (checklist of concrete, code-level steps)

- [ ] `infra/aiwonder/backup/calyx-backup.service` — one-shot service unit:
  ```ini
  [Unit]
  Description=Calyx restic backup (one-shot)
  After=network-online.target

  [Service]
  Type=oneshot
  User=croyse
  Group=croyse
  EnvironmentFile=/run/leapable/secrets/calyx.env
  ExecStart=/bin/bash /home/croyse/calyx/repo/infra/aiwonder/backup/restic-backup.sh
  StandardOutput=journal
  StandardError=journal
  SyslogIdentifier=calyx-backup
  ```
- [ ] `infra/aiwonder/backup/calyx-backup.timer` — hourly timer:
  ```ini
  [Unit]
  Description=Calyx restic backup timer
  Requires=calyx-backup.service

  [Timer]
  OnCalendar=hourly
  Persistent=true
  RandomizedDelaySec=300

  [Install]
  WantedBy=timers.target
  ```
  `Persistent=true` ensures a missed run (e.g., box rebooted during scheduled
  window) is executed on next boot. `RandomizedDelaySec=300` avoids backup-storm
  at the top of the hour with other leapable timers.
- [ ] `infra/aiwonder/backup/zfs-snapshot.sh` — [OPERATOR] snapshot setup:
  ```bash
  #!/usr/bin/env bash
  # [OPERATOR] Requires sudo. Sets up ZFS auto-snapshots on hotpool/calyx.
  # Uses zfs-auto-snapshot (or sanoid) if installed; falls back to cron.
  # POSTURE: single-host; whole-host loss accepted. Snapshots are local only.
  set -euo pipefail

  # Enable ZFS auto-snapshot property (requires zfs-auto-snapshot package):
  if command -v zfs-auto-snapshot &>/dev/null; then
    sudo zfs set com.sun:auto-snapshot=true hotpool/calyx
    sudo zfs set com.sun:auto-snapshot:hourly=24   hotpool/calyx
    sudo zfs set com.sun:auto-snapshot:daily=7     hotpool/calyx
    sudo zfs set com.sun:auto-snapshot:weekly=4    hotpool/calyx
    echo "ZFS auto-snapshot enabled on hotpool/calyx"
    zfs get com.sun:auto-snapshot hotpool/calyx
  else
    echo "zfs-auto-snapshot not installed; add manual snapshot cron instead:"
    echo "  sudo crontab -e  # add: @hourly zfs snapshot hotpool/calyx@\$(date +%Y%m%dT%H%M%S)"
    exit 1
  fi
  ```
- [ ] Operator install instructions in a `infra/aiwonder/backup/README.md`
  (≤50 lines): steps to install the timer, verify it runs, check the first
  snapshot list
- [ ] Comment in timer: "RPO = 1 hour (OnCalendar=hourly). RTO = time to restic
  restore + calyx verify-restore. No off-machine replica. See dr-drill-runbook.md."

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] shell lint: `bash -n infra/aiwonder/backup/zfs-snapshot.sh` exits 0
- [ ] unit (no sudo): `systemd-analyze verify` on both `calyx-backup.service`
  and `calyx-backup.timer` exits 0 on aiwonder
- [ ] unit: timer file contains `OnCalendar=hourly` — grep exact
- [ ] unit: timer file contains `Persistent=true` — grep exact
- [ ] unit: service file contains `Type=oneshot` — grep exact
- [ ] unit: service file contains `EnvironmentFile=` pointing to
  `/run/leapable/secrets/calyx.env` — grep exact path
- [ ] unit: timer file has `RandomizedDelaySec=` with a non-zero value — grep
- [ ] edge: `zfs-snapshot.sh` with `zfs-auto-snapshot` absent → exits 1 with
  the manual cron instruction printed (not a silent no-op)
- [ ] fail-closed: service unit does NOT have `Restart=on-failure` (it is
  one-shot; restarts are handled by the timer re-firing; assert absence)

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `systemctl list-timers | grep calyx`; `zfs get
  com.sun:auto-snapshot hotpool/calyx`; `restic snapshots` after timer fires
- **Readback (operator runs install, then waits for first timer fire):**
  ```bash
  # [OPERATOR]:
  sudo cp infra/aiwonder/backup/calyx-backup.{service,timer} /etc/systemd/system/
  sudo systemctl daemon-reload
  sudo systemctl enable --now calyx-backup.timer
  systemctl list-timers --all | grep calyx
  # Must show: calyx-backup.timer with NEXT fire time

  # After first fire:
  journalctl -u calyx-backup --since "1 hour ago" --no-pager
  # Must show backup completed, restic check passed

  # ZFS snapshots:
  bash infra/aiwonder/backup/zfs-snapshot.sh  # [OPERATOR]
  zfs list -t snapshot hotpool/calyx | head -5
  # Must show at least 1 snapshot
  ```
- **Prove:** `list-timers` shows calyx-backup.timer with a future fire time;
  journal shows first backup completed; `zfs list -t snapshot` shows ≥ 1
  snapshot. Outputs attached to PH67 issue.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH67 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
