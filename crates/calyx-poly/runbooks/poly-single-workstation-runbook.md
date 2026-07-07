# Poly Single-Workstation Runbook

Issue: #128

## Scope And Boundary

Poly runs as a local-only intelligence system on one workstation. This runbook covers local process
startup, vault/data layout, backups, and restore verification. It does not authorize Polymarket
trading, order signing, bankroll management, or browser/site operation.

## Workstation Layout

- Repository: `C:\code\poly`
- Calyx workspace: `C:\code\poly\Calyx`
- FSV evidence: `C:\code\poly\Calyx\target\fsv`
- Runtime data, caches, backfills, output, and FSV evidence must stay on `C:` under `C:\code\poly`
  or an explicitly named C-drive Poly/Calyx data root. Never use `D:`. If `C:` is low on space, stop
  and wait for space to be freed instead of moving work to `D:`.
- Local vault/data roots: keep under an explicitly named Poly/Calyx data directory, never under a
  temporary download folder.
- Secrets: load from the approved secret source only; do not place plaintext secrets in backup
  scripts, runbook notes, logs, or FSV evidence.

## Startup Checklist

1. Confirm the repo branch and dirty-worktree state.
2. Run `cargo check -p calyx-poly` from `C:\code\poly\Calyx`.
3. Start only local read-only ingest/forecast processes needed for the task.
4. Confirm logs and FSV output paths are writable.
5. Confirm every runtime/output path resolves to `C:` and not `D:`.
6. Confirm no process has a trading/order-signing configuration enabled.

## Backup Policy

Use restic as the primary portable backup for vault/data roots and FSV evidence that must survive
host loss. Use ZFS snapshots only when the data root is on a ZFS dataset; snapshots are a fast local
rollback layer, not a substitute for restic.

Minimum policy:

- RPO: one hour for active vault/data roots.
- RTO: restic restore time plus restore-verify drill time.
- Backup scope: vault/data roots, config files needed to reopen those roots, and runbook evidence.
- Exclusions: build artifacts that can be reproduced, plaintext secrets, browser profiles, and any
  trading/session material.
- Retention: keep hourly snapshots for short-term recovery and daily snapshots for operator-error
  recovery, subject to available disk.

## Backup Command Template

Run from PowerShell with restic already configured through environment variables or an approved
secret loader:

```powershell
restic backup C:\path\to\poly-data --tag poly-vault --tag workstation
restic snapshots --tag poly-vault
```

If the vault/data root lives on ZFS, take a local snapshot before high-risk maintenance:

```powershell
zfs snapshot pool/poly-data@pre-maintenance-YYYYMMDD-HHMM
```

## Restore-Verify Drill

Run the drill into a staging directory. Never restore over the active vault/data root.

1. Pick the snapshot: `restic snapshots --tag poly-vault`.
2. Restore to staging: `restic restore <snapshot-id> --target C:\restore-staging\poly`.
3. Locate the restored vault/data root under staging.
4. Run the read-only verifier appropriate for the restored root, for example
   `calyx verify-restore --vault <restored-vault> --json` when the Calyx CLI is available.
5. Persist verifier JSON plus a `BLAKE3SUMS.txt` for the restored evidence directory.
6. Compare the restored ledger tip/counts or report hash against the pre-backup evidence.
7. Mark the drill passed only when the verifier reports an intact chain/readback and expected rows.

## Fail-Closed Cases

Stop and record evidence if any of these occur:

- restic repository, password, or key material is missing.
- Latest backup is older than the RPO.
- Restore target resolves inside the active vault/data root.
- Restored verifier reports a missing vault, broken ledger chain, missing WAL bytes, row-count
  mismatch, or unreadable artifact.
- ZFS snapshot command fails or targets the wrong dataset.
- Any command would include secrets, browser profiles, orders, positions, or bankroll material.

## Evidence To Save

- `restic snapshots` output for the selected snapshot.
- Backup command log without secrets.
- Restore command log.
- Restore verifier JSON.
- `BLAKE3SUMS.txt` for restored evidence.
- A short note naming the exact data root, snapshot id, verifier command, verifier result, and drill
  timestamp.
