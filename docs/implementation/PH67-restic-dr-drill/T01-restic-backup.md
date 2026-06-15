# PH67 · T01 — Restic backup script + include/exclude set

| Field | Value |
|---|---|
| **Phase** | PH67 — restic backup + DR drill |
| **Stage** | S16 — Server & Deployment |
| **Crate** | `infra` (no Rust crate) |
| **Files** | `infra/aiwonder/backup/restic-backup.sh` |
| **Depends on** | PH66 T02 (ZFS datasets provisioned; `/zfs/hot/calyx/` exists) |
| **Axioms** | A15 |
| **PRD** | `dbprdplans/16 §7` |

## Goal

Write the restic backup script that backs up `/zfs/hot/calyx` to
`/zfs/archive/calyx/restic` with the correct include/exclude set: WAL, base CF,
codebooks, panel, and ledger are required; ANN indexes, kernel indexes, and
guard models are excluded as rebuildable. The script initializes the restic repo
if it does not exist, runs `restic backup`, verifies the new snapshot with
`restic check`, and writes a structured log entry. The restic repo password is
read from the environment (`$CALYX_RESTIC_PASSWORD`) — never hardcoded.

## Build (checklist of concrete, code-level steps)

- [ ] `infra/aiwonder/backup/restic-backup.sh`:
  ```bash
  #!/usr/bin/env bash
  # Calyx restic backup — runs as croyse (no sudo required after provisioning).
  # Password via $CALYX_RESTIC_PASSWORD (from Infisical calyx.env).
  # Single-host posture: no off-machine replica. RPO = backup interval.
  # Temp files staged inside destination dataset (avoid EXDEV on ZFS rename).
  set -euo pipefail

  REPO="/zfs/archive/calyx/restic"
  SOURCE="/zfs/hot/calyx"
  LOG="/zfs/hot/calyx/logs/backup-$(date -u +%Y%m%dT%H%M%SZ).log"
  mkdir -p "$(dirname "$LOG")"

  : "${CALYX_RESTIC_PASSWORD:?CALYX_RESTIC_PASSWORD not set}"
  export RESTIC_PASSWORD="$CALYX_RESTIC_PASSWORD"
  export RESTIC_REPOSITORY="$REPO"

  # Initialize repo if absent (idempotent)
  if ! restic snapshots &>/dev/null; then
    echo "$(date -u +%FT%TZ) Initializing restic repo at $REPO" | tee -a "$LOG"
    restic init 2>&1 | tee -a "$LOG"
  fi

  # Backup with explicit include/exclude
  # REQUIRED (data bearing):   wal/, base/, codebooks/, panel/, ledger/
  # EXCLUDED (rebuildable):     ann/, kernel/, guard/, tmp/
  echo "$(date -u +%FT%TZ) Starting backup" | tee -a "$LOG"
  restic backup "$SOURCE" \
    --exclude "$SOURCE/ann" \
    --exclude "$SOURCE/kernel" \
    --exclude "$SOURCE/guard" \
    --exclude "$SOURCE/tmp" \
    --exclude "$SOURCE/logs" \
    --tag calyx \
    --json 2>&1 | tee -a "$LOG"

  # Verify the snapshot just created
  echo "$(date -u +%FT%TZ) Running restic check" | tee -a "$LOG"
  restic check 2>&1 | tee -a "$LOG"

  echo "$(date -u +%FT%TZ) Backup complete" | tee -a "$LOG"
  ```
- [ ] Include/exclude rationale documented in a comment block: which paths are
  required vs rebuildable, and why WAL + base + codebooks + panel + ledger are
  the minimum set needed for a byte-exact restore
- [ ] The script must log the snapshot ID output by `restic backup --json` so
  the DR drill can reference a specific snapshot ID
- [ ] Script exits non-zero on any `restic` command failure; no silent
  continuation past a backup error

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] shell lint: `bash -n infra/aiwonder/backup/restic-backup.sh` exits 0
- [ ] unit: script source contains `--exclude "$SOURCE/ann"` and
  `--exclude "$SOURCE/kernel"` — grep for exact flags
- [ ] unit: script source contains `--exclude "$SOURCE/guard"` — grep
- [ ] unit: script does NOT contain any hardcoded password string — grep for
  no literals matching common password patterns in the script body
- [ ] unit: `CALYX_RESTIC_PASSWORD` unset → script exits 1 with the `:?` error
  (assert the `set -u` + `:?` guard fires)
- [ ] unit: `restic init` line is present and inside an `if ! restic snapshots`
  guard (idempotent init) — grep for the guard pattern
- [ ] edge: script is re-run when repo already exists → init is skipped, backup
  proceeds (mock `restic snapshots` returning 0 in a test stub)
- [ ] fail-closed: `restic backup` exits non-zero → script exits non-zero (not
  swallowed by a `|| true` or similar)

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `restic -r /zfs/archive/calyx/restic snapshots` listing at least one
  snapshot tagged `calyx` after the first run
- **Readback:**
  ```bash
  # On aiwonder (with CALYX_RESTIC_PASSWORD loaded from infisical):
  infisical run --projectId=c2d7e44c-d7d1-4b27-aa23-2ed5a97fa0b5 --env=prod -- \
    bash infra/aiwonder/backup/restic-backup.sh

  restic -r /zfs/archive/calyx/restic \
    --password-command "echo $CALYX_RESTIC_PASSWORD" \
    snapshots --tag calyx
  # Must list ≥ 1 snapshot with timestamp and tag "calyx"

  # Verify WAL is in the snapshot (not excluded):
  restic -r /zfs/archive/calyx/restic \
    --password-command "..." \
    ls latest | grep "wal/"
  # Must show WAL entries
  ```
- **Prove:** snapshot list shows ≥ 1 entry tagged `calyx`; `restic ls latest`
  shows `wal/` entries present; `ann/` and `kernel/` entries absent. Outputs
  attached to PH67 issue.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH67 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
