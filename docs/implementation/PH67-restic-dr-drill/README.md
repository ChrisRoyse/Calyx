# PH67 — restic backup + DR drill

**Stage:** S16 — Server & Deployment  ·  **Crate:** `infra` (no Rust crate;
verification tooling in `calyxd`)  ·
**PRD roadmap:** `16 §7`  ·  **Axioms:** A15, A24

## Objective

Implement the durability story for the no-redundancy single-host deployment:
a restic timer that backs up `/zfs/hot/calyx` (WAL + base + codebooks + panel
+ ledger; ANN/kernel/guard rebuildable, optional) to
`/zfs/archive/calyx/restic`; ZFS auto-snapshots on `hotpool/calyx`; and a
DR drill that proves backup integrity by restoring a vault from restic, reading
back the exact constellation/anchor/ledger bytes, and verifying the chain is
intact. The drill FSV is **not** a `"restored":true` flag — it is a byte-level
readback of real Aster data on aiwonder.

Single-host, no off-machine replica posture is stated honestly and documented:
whole-host loss is accepted. There is no HA claim. The DR drill runbook
documents how to recover from the worst-case scenario.

> **Operator/sudo constraint (binding, `01 §3`):** `systemd` timer install and
> ZFS auto-snapshot setup require operator/sudo steps. The restic backup and
> restore themselves, and all verification, run as `croyse` without sudo once
> the timer is installed. Do NOT touch leapable/contextgraph/PostgreSQL state.

## Dependencies

- **Phases:** PH66 (ZFS datasets provisioned, `calyxd` running, `/metrics` up
  — PH66 FSV gate must pass before PH67 begins)
- **Provides for:** PH67 is the final gate of Stage 16 (`DEPLOY` predicate in
  `19 §5`)

## Current state (build off what exists)

`infra/aiwonder/` was created in PH66. ZFS datasets exist post-provisioning.
`calyxd` is running. No restic repo, no backup timer, no DR runbook yet.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `infra/aiwonder/backup/restic-backup.sh` | Restic backup script: include set, excludes, run `restic backup`, `check`, log result |
| `infra/aiwonder/backup/restic-restore.sh` | Restic restore script for DR drill: restore snapshot to staging dir, verify, swap |
| `infra/aiwonder/backup/calyx-backup.timer` | systemd timer unit (operator-installed) |
| `infra/aiwonder/backup/calyx-backup.service` | systemd one-shot service unit that runs `restic-backup.sh` |
| `infra/aiwonder/backup/zfs-snapshot.sh` | [OPERATOR] ZFS auto-snapshot setup script |
| `infra/aiwonder/backup/dr-drill-runbook.md` | Runbook: step-by-step DR drill procedure + byte-verify commands |
| `crates/calyxd/src/verify.rs` | `calyx verify-restore --vault <path>`: reads back constellations/anchors/ledger bytes, verifies chain |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | Restic backup script + include/exclude set | — |
| T02 | Restic systemd timer + ZFS auto-snapshots | T01 |
| T03 | `calyx verify-restore` byte-verification tool | — |
| T04 | DR drill runbook + FSV byte-verify execution | T01, T02, T03 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

DR drill on aiwonder (manual, byte-level — no FSV harness):
1. `restic -r /zfs/archive/calyx/restic snapshots` → lists ≥ 1 snapshot.
2. `bash infra/aiwonder/backup/restic-restore.sh <snapshot-id> /tmp/calyx-dr`
   → restores vault to staging dir.
3. `calyx verify-restore --vault /tmp/calyx-dr` → reads back every stored
   constellation CxId, every anchor, and the full ledger chain; prints byte
   counts and chain hash; exits 0 only if chain is intact.
4. `xxd /tmp/calyx-dr/wal/0000000001.wal | head -4` → WAL magic bytes present
   (not an empty file, not a placeholder).
5. All outputs (snapshot list, restore log, verify output, xxd excerpt) attached
   to PH67 issue as FSV evidence.

## Risks / landmines

- **`EXDEV` on restore:** restic `--target` must be a path inside the
  destination dataset; if staging across dataset boundaries, the restore will
  fail with `EXDEV` on atomic rename. Use a staging dir inside
  `/zfs/archive/calyx/` (same dataset as the restic repo) or `/zfs/hot/calyx/`
  for the working restore (`01 §4`).
- **Restic init:** `restic init` must be run once before the first backup.
  The operator script must be idempotent (skip init if repo already exists).
- **WAL + base required; ANN/kernel/guard optional:** the include set is
  explicit. Excluding large rebuildable indexes keeps backup size bounded.
  The verify-restore tool must not require ANN indexes to be present.
- **Chain verification depth:** `calyx verify-restore` must walk the full
  ledger chain (not just the tip) — a truncated or partial chain fails.
- **Single-host honest posture:** the DR runbook explicitly states "RPO = time
  since last restic snapshot (default: 1 hour). RTO = time to restore + verify.
  No off-machine replica. Whole-host loss is accepted posture for this
  deployment." Do not paper over this.
- **restic REPO password:** stored in Infisical (`leapable-aiwonder-prod`) as
  `CALYX_RESTIC_PASSWORD`; loaded from `EnvironmentFile` in the service unit.
  Never hardcoded, never in the repo.
