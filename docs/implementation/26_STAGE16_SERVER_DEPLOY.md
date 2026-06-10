# Stage 16 — Server (calyxd) & Deployment (PH65–PH67)

Productionize on aiwonder: the `calyxd` daemon, systemd unit (sudo-gated),
ZFS provisioning, Prometheus/Grafana, restic backup, and a byte-verified DR
drill. Lands in `calyxd` + `infra/aiwonder/`. **Note the constraint (`01 §3`):**
sudo is password-backed and authorized through the local env var name, not
passwordless; systemd install + ZFS dataset creation are gated host actions that
must read back service/dataset state and must never print the sudo value.

---

## PH65 — calyxd daemon (loopback, healthcheck)
- **Objective.** The server daemon — same `calyx` core, served — binding
  loopback only, with a real healthcheck.
- **Deps.** PH24, PH13.
- **Deliverables.** `calyxd` (loopback bind; MCP over the gated ingress),
  `calyx healthcheck --wait` (probes CUDA init + a real read), config
  (`calyx.toml`), VRAM budget honoring resident TEI.
- **Key tasks.** fail-loud on CUDA init failure (`CALYX_FORGE_DEVICE_UNAVAILABLE`
  — no silent CPU fallback in server mode); healthcheck writes `"pass"`.
- **FSV gate.** `calyxd` starts on aiwonder, binds loopback, `healthcheck`
  returns `"pass"`; a CUDA-init failure makes it fail loud (read the health JSON
  + systemctl/log).
- **Axioms/PRD.** A18, A16, `16 §2/§4`.

## PH66 — systemd + ZFS provisioning + Prometheus/Grafana
- **Objective.** Run as a managed service with the full observability surface.
- **Deps.** PH65. **(sudo-gated steps use authorized password-backed sudo.)**
- **Deliverables.** `calyxd.service` (loopback, non-root, `LimitNOFILE`), ZFS
  datasets (`hotpool/calyx`, `archive/calyx`, restic dataset), Prometheus
  `/metrics` (ingest p95, search p99/strategy, recall tripwire, guard FAR/FRR,
  n_eff, kernel recall, Anneal A/Bs, VRAM budget, the 25-hazard metrics), a
  Grafana dashboard.
- **Key tasks.** install unit + create datasets with authorized sudo; relocate
  `CALYX_HOME/data`→`/zfs/hot/calyx`; wire Prometheus target; build the Grafana
  panels.
- **FSV gate.** (after operator steps) unit active + `/metrics` scraped;
  Grafana panels read via **screenshot + AI-vision** (charts `read_text` can't
  capture); data physically on the ZFS datasets (verify paths).
- **Axioms/PRD.** `16 §2/§3/§6`, A26 (metrics).

## PH67 — restic backup + DR drill
- **Objective.** The durability story for the no-redundancy single host, proven
  by a real restore.
- **Deps.** PH66.
- **Deliverables.** restic timer over `/zfs/hot/calyx` (WAL+base+codebooks+panel
  +ledger; ANN/kernel/guard rebuildable, optional) → `/zfs/archive/calyx/restic`;
  ZFS auto-snapshots; the DR drill runbook.
- **Key tasks.** restic include set; snapshot schedule; DR drill = restore a
  vault, read back exact constellations/anchors/ledger bytes, verify chain.
- **FSV gate.** **DR drill**: restore a vault from restic on aiwonder → byte-
  verify constellations/anchors/ledger, chain intact (read the bytes, not a
  "restored:true").
- **Axioms/PRD.** `16 §7`, A15, A24 (single-host posture, no HA claim).

---

## Stage 16 exit
`calyxd` runs as a loopback systemd service on aiwonder — Aster on hot NVMe +
cold mirror, Forge sharing the RTX 5090 with the resident TEI under a VRAM
budget, Prometheus/Grafana/restic, a byte-verified DR drill — while the existing
leapable/PostgreSQL state on the box is untouched — PRD `DEPLOY`.
