# PH66 · T01 — `calyxd.service` unit + operator install script

| Field | Value |
|---|---|
| **Phase** | PH66 — systemd + ZFS provisioning + Prometheus/Grafana |
| **Stage** | S16 — Server & Deployment |
| **Crate** | `infra` (no Rust crate) |
| **Files** | `infra/aiwonder/systemd/calyxd.service`, `infra/aiwonder/ops/install-service.sh` |
| **Depends on** | PH65 T06 (calyxd binary passing FSV) |
| **Axioms** | A16 |
| **PRD** | `dbprdplans/16 §2` |

## Goal

Write the `calyxd.service` systemd unit file (repo-owned, operator-installed)
and the idempotent operator install script. The unit runs as `croyse`/`leapable`
(non-root), binds loopback only, sets `LimitNOFILE=1048576`, restarts on
failure, and runs `calyx healthcheck --wait 30` as `ExecStartPost`. The install
script is the single operator step: copy the unit, reload systemd, enable,
start, and verify. All sudo-gated steps are clearly marked; the script must be
safe to re-run (idempotent).

> **[OPERATOR] steps in this card require sudo.** They are never run by
> `cargo test` or an agent in an automated pipeline.

## Build (checklist of concrete, code-level steps)

- [ ] `infra/aiwonder/systemd/calyxd.service` — exact content:
  ```ini
  [Unit]
  Description=Calyx association database (calyxd)
  After=network-online.target leapable-secrets-load.service
  Wants=leapable-secrets-load.service

  [Service]
  User=croyse
  Group=croyse
  EnvironmentFile=/run/leapable/secrets/calyx.env
  WorkingDirectory=/home/croyse/calyx/repo
  ExecStart=/home/croyse/calyx/target/release/calyxd --config /home/croyse/calyx/repo/infra/aiwonder/calyx.toml
  ExecStartPost=/home/croyse/calyx/target/release/calyx healthcheck --wait 30
  Restart=on-failure
  RestartSec=5s
  LimitNOFILE=1048576
  TimeoutStartSec=60

  [Install]
  WantedBy=multi-user.target
  ```
  Note: `User=croyse` is the actual user per `01 §3`; the PRD's `User=leapable`
  is the eventual production target when binaries are installed under
  `/opt/leapable/calyx/` — document both in an inline comment in the unit file.
- [ ] `infra/aiwonder/ops/install-service.sh` — idempotent operator script:
  ```bash
  #!/usr/bin/env bash
  set -euo pipefail
  UNIT_SRC="$(dirname "$0")/../systemd/calyxd.service"
  UNIT_DST="/etc/systemd/system/calyxd.service"

  # Pre-flight: ensure calyxd binary exists
  CALYXD_BIN="/home/croyse/calyx/target/release/calyxd"
  [[ -x "$CALYXD_BIN" ]] || { echo "ERROR: calyxd not built at $CALYXD_BIN"; exit 1; }

  # Pre-flight: no existing process on port 7700
  if ss -tlnp | grep -q ':7700'; then
    echo "WARNING: port 7700 already in use; stopping existing calyxd"
    sudo systemctl stop calyxd || true
  fi

  sudo cp "$UNIT_SRC" "$UNIT_DST"
  sudo chmod 644 "$UNIT_DST"
  sudo systemctl daemon-reload
  sudo systemctl enable calyxd
  sudo systemctl start calyxd
  sleep 5
  sudo systemctl is-active calyxd
  echo "calyxd.service installed and active"
  ```
- [ ] Add a comment block at the top of `install-service.sh` stating: "This
  script requires sudo (passwordless-sudo constraint: `01 §3`). Run as the
  operator, not via cargo test or agent automation."
- [ ] `infra/aiwonder/calyx.toml` updated from PH65: add `[service]` section
  documenting the systemd integration, and set `health_log_path =
  "/zfs/hot/logs/calyx-health/latest.json"` (the post-provisioning path)
- [ ] `infra/aiwonder/bin/install-calyx-deploy-wiring.sh` has run on aiwonder:
  it merges `infra/aiwonder/secrets-loader/calyx.env.map.json` into the
  Leapable secrets loader map, installs
  `/usr/local/sbin/calyx-aiwonder-healthcheck.sh`, and inserts the marked
  Calyx checks into `leapable-aiwonder-healthcheck`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] shell lint: `bash -n infra/aiwonder/ops/install-service.sh` exits 0 (no
  syntax errors)
- [ ] unit (no sudo): the unit file is valid INI that `systemd-analyze verify`
  would accept — run `systemd-analyze verify infra/aiwonder/systemd/calyxd.service`
  on aiwonder (does not require root, only `systemd-analyze` in userspace);
  assert exit 0
- [ ] unit: assert the unit file contains `LimitNOFILE=1048576` (grep exact
  string in file)
- [ ] unit: assert the unit file contains `Restart=on-failure`
- [ ] unit: assert the unit file contains `ExecStartPost=` with `healthcheck
  --wait 30`
- [ ] unit: assert the unit file contains `User=croyse` (or `User=leapable` for
  production layout — whichever is current; document which)
- [ ] edge: install script run when calyxd is already active → script stops old
  instance, copies new unit, restarts; does not leave two instances running

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `/etc/systemd/system/calyxd.service` after operator install; output
  of `systemctl status calyxd`; `/run/leapable/secrets/calyx.env`;
  `/zfs/hot/logs/calyx-health/latest.json`; and
  `/zfs/hot/logs/aiwonder-health/latest.json`
- **Readback (operator runs):**
  ```bash
  # [OPERATOR] on aiwonder:
  bash infra/aiwonder/ops/install-service.sh
  systemctl status calyxd --no-pager
  systemctl is-active calyxd
  # Must print: active
  journalctl -u calyxd --since "5 minutes ago" --no-pager | tail -20
  # Must contain: healthcheck pass line and startup banner
  stat -c '%a %U:%G %n' /run/leapable/secrets/calyx.env
  sed -n 's/^\([A-Z0-9_]*\)=.*/\1=<redacted>/p' /run/leapable/secrets/calyx.env
  jq '.status, [.checks[] | select(.name|startswith("calyx_")) | .name]' \
    /zfs/hot/logs/aiwonder-health/latest.json
  ```
- **Prove:** `systemctl is-active calyxd` returns `active`; `journalctl` output
  contains `ExecStartPost` completion with exit code 0; `latest.json` has
  `"status":"pass"`. Output attached to PH66 issue.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH66 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
