# PH66 — systemd + ZFS provisioning + Prometheus/Grafana

**Stage:** S16 — Server & Deployment  ·  **Crate:** `infra` (no Rust crate;
infra files + Prometheus metrics in `calyxd`)  ·
**PRD roadmap:** P9  ·  **Axioms:** A16, A26

## Objective

Run `calyxd` as a managed systemd service on aiwonder with the full observability
surface. The `calyxd.service` unit binds loopback, runs as the `croyse`/`leapable`
user (non-root), sets `LimitNOFILE=1048576`, and uses `ExecStartPost` to run the
healthcheck. ZFS datasets (`hotpool/calyx` → `/zfs/hot/calyx`,
`archive/calyx` → `/zfs/archive/calyx`, `archive/calyx-restic` →
`/zfs/archive/calyx/restic`) are provisioned in a single operator sudo step.
Prometheus `/metrics` is served on loopback; Grafana panels are verified via
screenshot + AI-vision (charts `read_text` cannot capture). The existing
leapable/contextgraph/PostgreSQL state is untouched throughout.

> **Operator/sudo constraint (binding, `01 §3`):** croyse has no passwordless
> sudo. Steps marked **[OPERATOR]** below require a `sudo`-capable shell run by
> the operator (not by an agent). The `calyxd.service` unit install, `systemctl
> enable/start`, `zfs create`, and `chown` of ZFS mountpoints are all
> **[OPERATOR]** steps. Dev/test never depends on them. Calyx runs from
> `CALYX_HOME` until provisioned. Do NOT touch leapable/contextgraph/PostgreSQL
> state on the box.

## Dependencies

- **Phases:** PH65 (calyxd binary + healthcheck + calyx.toml — must be passing
  FSV before this phase begins)
- **Provides for:** PH67 (restic timer + DR drill require ZFS datasets and
  running service to exist)

## Current state (build off what exists)

`infra/aiwonder/` does not exist yet. Prometheus is already running on aiwonder
at `127.0.0.1:9090`. Grafana is accessible at `ops.leapable.ai` (auto-authed via
Cloudflare Access). The PH65 calyxd binary is the deliverable being wrapped here.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `infra/aiwonder/systemd/calyxd.service` | systemd unit file (repo-owned, operator-installed to `/etc/systemd/system/`) |
| `infra/aiwonder/prometheus/calyx-scrape.yml` | Prometheus scrape config snippet for the loopback `/metrics` target |
| `infra/aiwonder/grafana/calyx-dashboard.json` | Grafana dashboard JSON (ingest/search/bits/kernel/anneal panels) |
| `infra/aiwonder/alertmanager/calyx-alerts.yml` | Alertmanager rules: tripwire breach, chain-verify failure, guard FAR drift, lens endpoint down, disk pressure |
| `crates/calyxd/src/metrics.rs` | Prometheus `/metrics` HTTP endpoint (loopback); all metric registrations |
| `infra/aiwonder/ops/provision-zfs.sh` | [OPERATOR] Script: `zfs create` + `chown`; idempotent (skips if dataset exists) |
| `infra/aiwonder/ops/install-service.sh` | [OPERATOR] Script: copy unit file, `systemctl daemon-reload`, `enable`, `start` |
| `infra/aiwonder/ops/relocate-data.sh` | Script (no sudo): `rsync` `CALYX_HOME/data/` → `/zfs/hot/calyx/`, update `calyx.toml` vault_path |
| `infra/aiwonder/secrets-loader/calyx.env.map.json` | Calyx loader-map fragment for the 13th rendered env file (`/run/leapable/secrets/calyx.env`) |
| `infra/aiwonder/bin/calyx-aiwonder-healthcheck.sh` | Wrapper called by `leapable-aiwonder-healthcheck` to run `calyx healthcheck` and write `/zfs/hot/logs/calyx-health/latest.json` |
| `infra/aiwonder/bin/install-calyx-deploy-wiring.sh` | [OPERATOR] Idempotent wiring step: merge the loader-map fragment, install the wrapper, and add the Calyx checks to the existing aggregator |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | `calyxd.service` unit + operator install script | — |
| T02 | ZFS dataset provisioning + data relocation | T01 |
| T03 | Prometheus `/metrics` endpoint (all named metrics) | — |
| T04 | Prometheus scrape config + Alertmanager rules | T03 |
| T05 | Grafana dashboard JSON + screenshot FSV | T03, T04 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

After operator steps:
1. `systemctl is-active calyxd` → `active`; `systemctl status calyxd` shows
   `ExecStartPost` (healthcheck) completed with exit 0.
1a. `/run/leapable/secrets/calyx.env` exists with mode `0400`; names-only
    readback shows `HF_HUB_TOKEN` and `HF_TOKEN`; no secret values are printed.
1b. `/zfs/hot/logs/aiwonder-health/latest.json` contains
    `calyx_healthcheck_latest_json` and `calyx_health_latest_status` checks, and
    `/zfs/hot/logs/calyx-health/latest.json` has `.status == "pass"`.
2. `curl -s http://127.0.0.1:9090/api/v1/targets` → calyx target is `up`.
3. `curl -s http://127.0.0.1:7700/metrics` → response contains all 25-hazard
   metric names + the named ingest/search/guard/anneal metrics.
4. Grafana dashboard panels read via **screenshot + AI-vision** on
   `ops.leapable.ai` — charts read_text cannot capture; screenshot attached to
   PH66 issue showing non-zero values.
5. `zfs list hotpool/calyx archive/calyx archive/calyx-restic` → all three
   datasets present with expected mountpoints.
6. `ls -la /zfs/hot/calyx/` → WAL and vault files present (relocated from
   `CALYX_HOME/data/`).

## Risks / landmines

- **`EXDEV` on ZFS rename:** `relocate-data.sh` must `rsync` into the destination
  dataset and then update the config path — never `mv` across dataset boundaries
  (`01 §4`).
- **Systemd `ExecStartPost` timeout:** if CUDA init is slow on a cold GPU, the
  healthcheck `--wait 30` must complete within systemd's `TimeoutStartSec`;
  set `TimeoutStartSec=60` in the unit to give headroom.
- **Port 7700 collision:** verify no other process binds 7700 before starting the
  unit; add a pre-start check in the operator install script.
- **Disk reference stability:** `provision-zfs.sh` must reference pools by name
  (`hotpool`, `archive`), not by device path, since `wwn-`/`eui-` names are the
  stable disk IDs — the pools themselves already abstract this (`01 §4`).
- **Grafana AI-vision FSV:** agent reads the panels via screenshot at
  `ops.leapable.ai` — auto-authed via Cloudflare Access; verify the tab
  behaviour (`31 §6b/c`).
- **No sudo in CI/tests:** all test paths run without systemd or ZFS; the operator
  scripts are shell-only artifacts never executed by `cargo test`.
