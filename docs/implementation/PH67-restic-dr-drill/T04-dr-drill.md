# PH67 · T04 — DR drill runbook + FSV byte-verify execution

| Field | Value |
|---|---|
| **Phase** | PH67 — restic backup + DR drill |
| **Stage** | S16 — Server & Deployment |
| **Crate** | `infra` (no Rust crate) |
| **Files** | `infra/aiwonder/backup/restic-restore.sh`, `infra/aiwonder/backup/dr-drill-runbook.md` |
| **Depends on** | T01 (backup script, repo has ≥ 1 snapshot), T02 (timer, ZFS snapshots exist), T03 (verify-restore tool passing) |
| **Axioms** | A15, A24 |
| **PRD** | `dbprdplans/16 §7` |

## Goal

Write the restic restore script and the DR drill runbook. Execute the full DR
drill on aiwonder and produce byte-level FSV evidence: restore a vault snapshot
from restic, run `calyx verify-restore` against it, confirm constellations/
anchors/ledger bytes are readable and the chain is intact. The drill explicitly
verifies byte contents — not a `"restored:true"` flag, not a harness assertion.
The runbook states the single-host posture honestly (no HA, RPO/RTO defined,
whole-host loss accepted).

## Build (checklist of concrete, code-level steps)

- [ ] `infra/aiwonder/backup/restic-restore.sh` — restore script for DR drill:
  ```bash
  #!/usr/bin/env bash
  # Calyx DR drill restore script.
  # Usage: bash restic-restore.sh <snapshot-id|latest> <restore-target-dir>
  # IMPORTANT: <restore-target-dir> must be inside a ZFS dataset to avoid EXDEV.
  #   Recommended: /zfs/archive/calyx/dr-staging/  (same pool as restic repo)
  # POSTURE: single-host. RPO = last restic snapshot. RTO = restore + verify time.
  # Whole-host loss is accepted posture for this deployment.
  set -euo pipefail

  SNAPSHOT="${1:?Usage: $0 <snapshot-id|latest> <restore-target-dir>}"
  TARGET="${2:?Usage: $0 <snapshot-id|latest> <restore-target-dir>}"
  REPO="/zfs/archive/calyx/restic"

  : "${CALYX_RESTIC_PASSWORD:?CALYX_RESTIC_PASSWORD not set}"
  export RESTIC_PASSWORD="$CALYX_RESTIC_PASSWORD"
  export RESTIC_REPOSITORY="$REPO"

  # Verify target is inside a ZFS dataset (avoid EXDEV cross-dataset rename)
  # Check that target is under /zfs/ (either hot or archive)
  case "$TARGET" in
    /zfs/*) ;;
    *) echo "ERROR: TARGET must be under /zfs/ to avoid EXDEV on rename"; exit 1 ;;
  esac

  mkdir -p "$TARGET"

  echo "Restoring snapshot $SNAPSHOT → $TARGET"
  restic restore "$SNAPSHOT" --target "$TARGET" 2>&1

  echo "Restore complete. Contents:"
  ls -la "$TARGET"

  echo ""
  echo "Run: calyx verify-restore --vault $TARGET --json"
  ```
- [ ] `infra/aiwonder/backup/dr-drill-runbook.md` — complete runbook (≤500 lines):

  ```markdown
  # Calyx DR Drill Runbook

  **Posture:** single-host, no off-machine replica. RPO = time since last restic
  snapshot (timer: hourly). RTO = restore time + verify time (typically < 30 min
  for a mature vault). Whole-host loss is accepted posture for this deployment.
  There is no HA claim. If off-machine RPO matters later, add WAL shipping or a
  second host (out of scope, flagged in `16 §7`).

  ## Pre-drill checklist
  - [ ] calyxd is stopped (or drill uses a snapshot, not the live vault):
        `sudo systemctl stop calyxd`
  - [ ] restic repo has ≥ 1 snapshot: `restic -r /zfs/archive/calyx/restic snapshots`
  - [ ] `calyx verify-restore` binary is built: `cargo build -p calyxd --release`
  - [ ] `CALYX_RESTIC_PASSWORD` is available (via infisical run or calyx.env)
  - [ ] Staging dir is inside a ZFS dataset: use `/zfs/archive/calyx/dr-staging/`

  ## Step 1 — List available snapshots
  ```bash
  restic -r /zfs/archive/calyx/restic snapshots --tag calyx
  # Note the snapshot ID to restore (use "latest" for most recent)
  ```

  ## Step 2 — Restore to staging dir
  ```bash
  SNAPSHOT_ID="latest"   # or a specific snapshot ID
  STAGING="/zfs/archive/calyx/dr-staging"
  mkdir -p "$STAGING"
  bash infra/aiwonder/backup/restic-restore.sh "$SNAPSHOT_ID" "$STAGING"
  # Expect: "Restore complete." and file listing
  ```

  ## Step 3 — Byte-verify the restored vault (FSV gate)
  ```bash
  source /home/croyse/calyx/repo/env.sh
  calyx verify-restore --vault "$STAGING" --json | python3 -m json.tool
  ```
  **Required output (all must be true):**
  - `"chain_intact": true`
  - `"constellation_count": <N>` where N > 0
  - `"anchor_count": <M>` where M > 0
  - `"wal_bytes_present": <B>` where B > 0
  - `"error": null`
  - `"ledger_tip_hash": "<hex>"` — non-empty, 64-character hex

  ## Step 4 — WAL byte spot-check
  ```bash
  xxd "$STAGING/wal/0000000001.wal" | head -4
  # First line must show the WAL magic bytes (not all-zero)
  ```

  ## Step 5 — Ledger chain spot-check
  ```bash
  calyx verify-restore --vault "$STAGING" --json \
    | python3 -c "import json,sys; r=json.load(sys.stdin); \
      print('chain:', r['chain_intact'], '| entries:', r['ledger_entry_count'], \
            '| tip:', r['ledger_tip_hash'][:16]+'...')"
  # Must print: chain: True | entries: <N> | tip: <hex prefix>
  ```

  ## Step 6 — Restart calyxd
  ```bash
  sudo systemctl start calyxd
  systemctl is-active calyxd  # → active
  calyx healthcheck --wait 30
  cat /zfs/hot/logs/calyx-health/latest.json | python3 -m json.tool
  # "status": "pass"
  ```

  ## Step 7 — Cleanup staging dir
  ```bash
  rm -rf "$STAGING"
  ```

  ## Evidence to attach to PH67 issue
  - Output of Step 1 (snapshot list)
  - Output of Step 3 (`verify-restore --json` result)
  - Output of Step 4 (`xxd` WAL bytes)
  - Output of Step 5 (chain summary)
  - Screenshot or output of Step 6 (calyxd restarted + health pass)
  ```
- [ ] `restic-restore.sh` validates the target path is under `/zfs/` to enforce
  same-dataset staging (EXDEV prevention, `01 §4`)

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] shell lint: `bash -n infra/aiwonder/backup/restic-restore.sh` exits 0
- [ ] unit: `restic-restore.sh` with a target outside `/zfs/` (e.g. `/tmp/dr`)
  → exits 1 with the `EXDEV` warning message
- [ ] unit: `restic-restore.sh` with `CALYX_RESTIC_PASSWORD` unset → exits 1
  with the `:?` guard error
- [ ] unit: runbook file exists and is valid Markdown; grep confirms all 5
  required `verify-restore` output fields are listed in Step 3
- [ ] unit: runbook contains the honest posture statement — grep for
  `"Whole-host loss is accepted posture"` or equivalent
- [ ] unit: runbook contains the RPO/RTO statement — grep for both `RPO` and
  `RTO` in the file
- [ ] edge: `restic-restore.sh` with `snapshot-id=latest` → restic receives
  `latest` as the snapshot argument (not empty)
- [ ] fail-closed: if `restic restore` exits non-zero, script exits non-zero
  immediately (no `|| true` bypass); assert via grep of script source

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `calyx verify-restore --json` output on the restored vault; `xxd`
  of WAL bytes; chain summary line
- **Readback — full DR drill on aiwonder:**
  ```bash
  # Step 1: snapshot list
  restic -r /zfs/archive/calyx/restic snapshots --tag calyx

  # Step 2: restore
  bash infra/aiwonder/backup/restic-restore.sh latest /zfs/archive/calyx/dr-staging

  # Step 3: byte-verify (THE FSV GATE)
  calyx verify-restore --vault /zfs/archive/calyx/dr-staging --json \
    | python3 -m json.tool

  # Step 4: WAL bytes
  xxd /zfs/archive/calyx/dr-staging/wal/0000000001.wal | head -4
  ```
- **Prove:** `verify-restore` JSON shows `chain_intact: true`,
  `constellation_count > 0`, `anchor_count > 0`, `wal_bytes_present > 0`,
  `error: null`. The `xxd` output shows non-zero WAL magic bytes in the first
  line. Both attached to PH67 issue as the Stage 16 `DEPLOY` FSV gate.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (verify-restore JSON + xxd WAL bytes + chain summary) attached
      to the PH67 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
