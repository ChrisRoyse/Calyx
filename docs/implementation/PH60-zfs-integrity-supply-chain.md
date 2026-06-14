# PH60 GAP — ZFS checksum+scrub (bit-rot) + reproducible builds + SBOM (#596)

| Field | Value |
|---|---|
| **Phase** | PH60 — Encryption + tenant isolation (security by construction, A33) |
| **Source** | coverage audit 2026-06-09 — unowned blindspot |
| **PRD** | `30 §1` (Tampering: ZFS checksums + scrub), `30 §2` (supply chain: reproducible builds + SBOM) |
| **Axioms** | A33 (security & privacy by construction), A16 (fail closed) |

## Capability the system needs (intent)

A real database holding everyone's data must defend two integrity surfaces the
prior PH60/PH61 work left uncovered:

1. **Data-at-rest bit-rot (Tampering).** PH60-T06 probed ZFS *encryption* only.
   Silent corruption — cosmic-ray bit flips, firmware bugs, failing NAND — is a
   distinct threat. ZFS checksums every block (fletcher4 by default) and a
   *scrub* walks every block to verify those checksums. Without a scheduled
   scrub, rot accumulates undetected until a read fails. These are single-vdev
   pools (no redundancy), so the deliverable is **detection + alerting**, not
   auto-repair.
2. **Supply-chain integrity.** PH61-T05 added `cargo audit` + content-addressed
   lens weights and an in-process `generate_sbom()` struct, but the two named
   `30 §2` defenses **reproducible builds** and an **emitted SBOM artifact** had
   no implementation. Reproducible builds let anyone confirm a shipped binary
   was built from the audited source (no implant); the SBOM is the machine-
   readable dependency inventory vulnerability scanners consume.

## What was built

### Reproducible builds (`scripts/supply-chain/`)
- `repro-build-env.sh` — canonical env: pinned `SOURCE_DATE_EPOCH`,
  `CARGO_INCREMENTAL=0`, `LC_ALL=C`, `TZ=UTC`, `--remap-path-prefix`.
- `verify-reproducible-build.sh` — builds a package twice and compares a sha256
  manifest of all artifacts; fails closed on any divergence.

### SBOM (`scripts/supply-chain/`)
- `emit-sbom.py` — deterministic CycloneDX 1.6 emitter (stdlib only).
- `emit-sbom.sh` — wrapper + independent count cross-check vs `Cargo.lock`.

### ZFS bit-rot (`infra/aiwonder/`)
- `bin/verify-zfs-integrity.sh` — read-only (no sudo) gate: checksum=on +
  `zpool status -x` healthy + scrub freshness. The recurring health check.
- `ops/enable-zfs-scrub.sh` — [OPERATOR] enable OpenZFS `zfs-scrub-monthly@`
  timers per pool (idempotent).
- `ops/fsv-zfs-bitrot.sh` — [OPERATOR] end-to-end detection proof on a
  disposable file-backed pool (flip bytes → scrub → `zpool status` reports it).

## FSV — source of truth, read back independently

| Defense | SoT | Synthetic I/O (2+2=4) |
|---|---|---|
| Reproducible build | files under `target/release/` | same source+toolchain → identical manifest sha256 across two builds |
| SBOM completeness | `dist/sbom/calyx.cdx.json` re-read | components == `grep -c '^\[\[package\]\]' Cargo.lock` |
| SBOM determinism | the SBOM bytes | fixed `SOURCE_DATE_EPOCH` → identical SBOM sha256 on re-run |
| ZFS checksums | `zfs get checksum` | every calyx dataset → `on` |
| ZFS health | `zpool status -x` | every backing pool → `is healthy`, CKSUM 0 |
| ZFS bit-rot detection | `zpool status -v` after scrub | inject 4 MiB random → CKSUM > 0 + EIO on read |

Edge cases audited (≥3 per tool) and evidence are recorded in the closing
comment on issue #596 (evidence root under `/home/croyse/calyx/data/`).

## Recurrence prevention (so this gap does not reopen)

- `verify-zfs-integrity.sh` is **fail-closed and read-only** — wire it into the
  daemon healthcheck / a cron so a disabled checksum, an unhealthy pool, or a
  stale scrub trips immediately rather than at the next failed read. (Follow-up:
  surface scrub-age + CKSUM as a Prometheus metric — tracked as a new issue.)
- `verify-reproducible-build.sh` and `emit-sbom.sh` are CI-ready (deterministic,
  no network, fail-closed) so reproducibility and SBOM freshness are gated on
  every release rather than re-audited by hand.
- SBOM and build are both reproducible, so drift is detectable by hash, not
  trust.
