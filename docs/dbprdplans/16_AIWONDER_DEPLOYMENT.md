# 16 — aiwonder Deployment & Operations

How `calyxd` lives on the datacenter box. Grounded in live readback (2026-06-05) and `leapablememory/docs2/aiwonder-system.md`.

## 0. Reaching the box (setup)

SSH credentials for `aiwonder` setup live in **`~/.config/aiwonder.env`** (operator machine, mode `0600`, outside any repo): `AIWONDER_HOST`/`IP`/`SSH_PORT`/`USER`, `AIWONDER_SSH_PASSWORD`, `AIWONDER_SUDO_PASSWORD`, VPN creds, Infisical UA bootstrap (`INFISICAL_UA_CLIENT_ID`/`SECRET`, `INFISICAL_PROJECT_ID`). Preflight + connect:
```bash
set -a; . ~/.config/aiwonder.env; set +a
getent hosts "$AIWONDER_HOST"; ping -c1 -W2 "$AIWONDER_HOST"      # VPN route up?
timeout 5 bash -lc "</dev/tcp/$AIWONDER_HOST/22" && echo ssh_open || echo ssh_closed
ssh "$AIWONDER_USER@$AIWONDER_HOST"   # password = $AIWONDER_SSH_PASSWORD; non-interactive: SSH_ASKPASS recipe
```
Never copy a secret value from `aiwonder.env` into a repo, issue, PR, or chat — reference env-var names only. Everything beyond bootstrap is in Infisical (`leapable-aiwonder-prod`). Full map: `leapablememory/docs2/aiwonder-system.md` §3–§4.

## 1. Hardware mapping

| Resource (live) | Calyx use |
|---|---|
| Ryzen 9 9950X 16c/32t | ingest batching, CPU SIMD Forge fallback, graph algos (Lodestar), background Anneal |
| 128 GB DDR5 (~84 GiB free) | memtables, ANN frontier in RAM, SPANN centroids, kernel index, Forge host buffers |
| RTX 5090 32 GB sm_120 | Forge matmul/distance/MI/quantize; shared with resident TEI (budgeted) |
| `hotpool` NVMe ~1.5 TB | `/zfs/hot/calyx/`: WAL, base CF, active quantized slots, ANN graphs, kernel/guard, online state |
| `archive` HDD mirror ~8.5 TB | `/zfs/archive/calyx/`: raw f32 sidecars, retired slots, old panels, ledger archive, restic source |
| PostgreSQL 18.4 + PgBouncer | **UNTOUCHED** — stays as the control plane (customers, billing, creators, queries, outbox). `calyxd` does not connect to it. |

VRAM coexistence: 3 resident TEI containers (general/legal/reranker) + dcgm-exporter already use the GPU. Forge runs under a **soft VRAM budget** (config), yields to TEI/marketplace; Anneal capped (`12 §6`). Honor `leapable-gpu-max-power.service` (600 W cap).

## 2. systemd unit

> **[SUPERSEDED — current dev path]** Rust (via rustup) and CUDA 13.3 are
> installed on aiwonder; **build natively**. The project is self-contained under
> `CALYX_HOME=/home/croyse/calyx` as user `croyse`. The `/opt/leapable/calyx`
> paths, the `leapable` user, and the `/run/leapable/secrets` rendering shown
> below were written for integration into the leapable box layout and are
> **superseded** by the self-contained layout (`docs/implementation/01_AIWONDER_ENVIRONMENT.md`).
> The unit below is illustrative; the production systemd install (S16 / PH66) is
> sudo-gated and its final paths are TBD.

```
# /etc/systemd/system/calyxd.service  (repo-owned, in infra/aiwonder/systemd/)
[Unit]
Description=Calyx association database (calyxd)
After=network-online.target leapable-secrets-load.service
Wants=leapable-secrets-load.service
[Service]
User=leapable
Group=leapable
EnvironmentFile=/run/leapable/secrets/calyx.env        # rendered by leapable-secrets-load
ExecStart=/opt/leapable/calyx/bin/calyxd --config /etc/leapable/calyx.toml
ExecStartPost=/opt/leapable/calyx/bin/calyx healthcheck --wait 30
Restart=on-failure
# bind loopback only; ingress via Caddy + Cloudflare Tunnel
LimitNOFILE=1048576
[Install]
WantedBy=multi-user.target
```

- Runs as `leapable` from `/opt/leapable/calyx/`; reads secrets from `/run/leapable/secrets/calyx.env` (add to `secrets-loader/secrets-map.json` → 13th file).
- **Binds loopback only**; Cloudflare Tunnel + Caddy are the sole ingress (existing pattern). Add `calyx.env` to Infisical (`leapable-aiwonder-prod`).
- **Native build (superseded the "no `rustc`" note):** Rust via rustup is installed on aiwonder, so `calyxd`/`calyx` are **built natively** there (CUDA 13.3, sm_120). A cross-built static binary + `.deb` is retained only as an optional minimal-deploy artifact, not the dev path.

## 3. Storage provisioning (ZFS)

```
zfs create hotpool/calyx           -o mountpoint=/zfs/hot/calyx
zfs create archive/calyx           -o mountpoint=/zfs/archive/calyx
zfs create archive/calyx-restic    -o mountpoint=/zfs/archive/calyx/restic
```
- Reference disks by `wwn-`/`eui-` (names flap across boots); pools auto-import (no fstab).
- Stage temp files **inside the destination dataset** (avoid `EXDEV` on cross-dataset rename) — Aster compaction/migration must obey this.
- `hotpool` has **no redundancy**: durability = WAL + ZFS auto-snapshots + restic to `/zfs/archive/calyx/restic`. Whole-host loss is accepted posture (single-host) — document it, don't pretend HA.

## 4. GPU/driver hygiene (from gotchas)

- Driver/userspace skew after unattended-upgrades → `nvidia-smi` mismatch; cure is reboot. Calyx `healthcheck` MUST probe CUDA init and fail loud (A16), not silently CPU-fallback in server mode.
- CUDA 13.3 from NVIDIA toolkit packages; Forge links `/usr/local/cuda` (currently `/usr/local/cuda-13.3`). Ship sm_120 cubin + PTX JIT fallback (`13`).

## 5. Networking & security

- VPN-only box; UFW default-deny; SSH from current Cisco subnet only. **Never** change UFW/sshd without a second live session (lockout risk).
- `calyxd` loopback + Cloudflare Access-gated route if any HTTP surface is exposed. **Do not** point `mcp.leapable.ai` at Calyx (intentionally 503; no per-user public MCP).
- Secrets: only via Infisical-rendered `/run/leapable/secrets/calyx.env` (mode 0400, `leapable`-owned). Never a hard-coded constant; never a secret in repo/issue/chat.

## 5b. Secrets — Infisical

All secrets are managed in **Infisical** (`leapable-aiwonder-prod`, project `c2d7e44c-d7d1-4b27-aa23-2ed5a97fa0b5`, env `prod`); bootstrap UA creds in `~/.config/aiwonder.env` (`§0`). Full catalog: `leapablememory/docs2/infisical-secrets-guide.md`.

- **The only secret Calyx needs today:** `hf_hub_token` (`HF_HUB_TOKEN`/`HF_TOKEN`) — pull/host embedder models + gated HF datasets (lenses, `05`; datasets, `28`). Already live in the vault.
- **Add later via CLI** if needed (no new code/values in repo): `infisical secrets set kaggle_username=… kaggle_key=…` for Kaggle datasets; any future model/registry token the same way.
- **Loading:** prefer `infisical run --projectId=… --env=prod -- <cmd>` (secrets enter the child env in memory, never disk); on the box, `leapable-secrets-load.service` renders `/run/leapable/secrets/calyx.env` (mode 0400, `leapable`-owned) from `infra/aiwonder/secrets-loader/calyx.env.map.json` — merge with `infra/aiwonder/bin/install-calyx-deploy-wiring.sh`. The current `calyx.env` map exports only `HF_HUB_TOKEN` and `HF_TOKEN`, both backed by the existing `hf_hub_token` Infisical secret name.
- **Discipline (binding):** never write a secret *value* into a repo file, issue, PR, comment, or chat — env-var **names** or `<REDACTED:LABEL>` only (`AICodingAgentSuperPrompt.md` §3.16). Most likely Calyx needs **only** the HF token; add others only when a concrete need appears.

## 6. Observability

| Signal | Export |
|---|---|
| `calyxd` metrics | Prometheus `/metrics` (loopback): ingest p95, search p99 per strategy, recall tripwire, guard FAR/FRR, n_eff, kernel recall ratio, Anneal A/Bs, VRAM budget use |
| GPU | existing dcgm-exporter |
| Health | `calyx healthcheck` → `/zfs/hot/logs/calyx-health/latest.json` (`.status` literal `"pass"`), called by `infra/aiwonder/bin/calyx-aiwonder-healthcheck.sh` and wired into `leapable-aiwonder-healthcheck` by `infra/aiwonder/bin/install-calyx-deploy-wiring.sh` |
| Grafana | a Calyx dashboard (ingest/search/bits/kernel/anneal panels) in `infra/aiwonder/grafana/`. Agents open it (`ops.leapable.ai`) via a **new tab in the one main Chrome** (auto-authed, `31 §6b`) and read the panels with **screenshot + AI-vision** (`31 §6c`) — charts that `read_text` can't capture. |
| Alerts | Alertmanager rules: tripwire breach, chain-verify failure, guard FAR drift, lens endpoint down, disk pressure on hotpool |

## 7. Backup & DR

- restic timer (`leapable-restic` pattern) includes `/zfs/hot/calyx` (WAL + base + codebooks + panel + ledger; ANN/kernel/guard rebuildable, optional) → `/zfs/archive/calyx/restic`.
- ZFS auto-snapshots on `hotpool/calyx`.
- DR drill (manual, FSV): restore a vault from restic, read back exact constellations/anchors/ledger bytes, verify chain intact. No FSV harness (banned).
- Single-host: no off-machine replica today; if RPO matters later, add Litestream-style WAL shipping or a second box (out of scope, flagged in `17`).

## 8. Rollout on the box (ordered)

1. Provision ZFS datasets; add `calyx.env` to Infisical + secrets-map.
2. Build `calyxd` + `calyx` CLI natively on aiwonder under `CALYX_HOME` (or sync a cross-built artifact for a minimal deploy); the final install location is sudo-gated/TBD (see the superseded-path note above).
3. Install `calyxd.service` (loopback); `healthcheck` green; Prometheus target up.
4. Run embedded-vault migration shadow (L0, `15`) against a real vault on the box.
5. (Optional) Stand up `calyxd` as a published/Discover **Vault host** (V3, `15`) — serves Vaults only; **does not connect to PostgreSQL**, which remains the untouched control plane.
6. Wire Grafana + alerts; restic; DR drill.
7. Flip per `15` phases with FSV at each gate (`19`).

## 9. Operating rules (inherited, binding)

- Verify source-of-truth directly after every op (`systemctl`, `zpool status`, `nvidia-smi`, actual Aster bytes) — a 200/return is a claim.
- Fail closed; no fallback that hides failure.
- Don't start throwaway TEI/Redis/cloudflared; reuse resident services.
- Don't reintroduce retired infra (Fly/Tigris/B2/S3); Aster is POSIX-on-ZFS only.
- Production synthetic test data must be cleanup-tagged and provably inert before the turn ends.

**One sentence:** `calyxd` deploys as a loopback systemd service on aiwonder — cross-built static binary, Aster on hot NVMe + cold mirror, Forge sharing the RTX 5090 with the resident TEI lenses under a VRAM budget, Infisical secrets, Prometheus/Grafana/restic — to host published/Discover Vaults, while the PostgreSQL control plane on the box is left entirely untouched.
