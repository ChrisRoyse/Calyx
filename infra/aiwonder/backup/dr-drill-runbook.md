# Calyx DR Drill Runbook

**Posture:** single-host, no off-machine replica. `RPO` = time since last restic
snapshot (timer: hourly). `RTO` = restore time + verify time (typically < 30 min
for a mature vault). **Whole-host loss is accepted posture** for this deployment.
There is no HA claim. If off-machine RPO matters later, add WAL shipping or a
second host (out of scope, flagged in `16 §7`).

## Pre-drill checklist
- [ ] calyxd is stopped (or the drill uses a snapshot, not the live vault):
      `sudo systemctl stop calyxd`
- [ ] restic repo has ≥ 1 snapshot:
      `restic -r /zfs/archive/calyx/restic snapshots`
- [ ] `calyx` (with `verify-restore`) is built:
      `cargo build -p calyxd --release` and `cargo build -p calyx-cli --release`
- [ ] `CALYX_RESTIC_PASSWORD` is available (via `infisical run` or `calyx.env`)
- [ ] Staging dir is inside a ZFS dataset: use `/zfs/archive/calyx/dr-staging`

## Step 1 — List available snapshots
```bash
restic -r /zfs/archive/calyx/restic snapshots --tag calyx
# Note the snapshot ID to restore (use "latest" for the most recent)
```

## Step 2 — Restore to a staging dir
```bash
SNAPSHOT_ID="latest"   # or a specific snapshot ID
STAGING="/zfs/archive/calyx/dr-staging"
bash infra/aiwonder/backup/restic-restore.sh "$SNAPSHOT_ID" "$STAGING"
# restic recreates the source's absolute path under the target, so the restored
# vault root is nested, e.g. $STAGING/home/croyse/calyx/data/vault. The script
# prints the exact "Restored vault root:" path and the verify-restore command —
# use that path.
```

## Step 3 — Byte-verify the restored vault (THE FSV GATE)
```bash
VAULT="<the Restored vault root printed in Step 2>"
calyx verify-restore --vault "$VAULT" --json | python3 -m json.tool
```
**Required output (all must hold):**
- `"chain_intact": true`
- `"constellation_count": <N>` with N > 0
- `"anchor_count": <M>` with M > 0
- `"wal_bytes_present": <B>` with B > 0
- `"ledger_tip_hash": "<hex>"` — non-empty, 64-character hex
- `"error": null`

## Step 4 — WAL byte spot-check
```bash
xxd "$VAULT/wal/"*.wal | head -4
# First line must show the WAL magic bytes "CXW1" (0x4358 5731), not all-zero.
```

## Step 5 — Ledger chain spot-check
```bash
calyx verify-restore --vault "$VAULT" --json \
  | python3 -c "import json,sys; r=json.load(sys.stdin); \
    print('chain:', r['chain_intact'], '| entries:', r['ledger_entry_count'], \
          '| tip:', r['ledger_tip_hash'][:16]+'...')"
# Must print: chain: True | entries: <N> | tip: <hex prefix>
# A byte-faithful restore reproduces the ORIGINAL tip hash exactly.
```

## Step 6 — Restart calyxd
```bash
sudo systemctl start calyxd
systemctl is-active calyxd            # → active
calyx healthcheck --wait 30
cat /zfs/hot/logs/calyx-health/latest.json | python3 -m json.tool   # "status":"pass"
```

## Step 7 — Cleanup staging dir
```bash
rm -rf "$STAGING"
```

## Evidence to attach to the PH67 issue
- Step 1 snapshot list
- Step 3 `verify-restore --json` (chain_intact true, counts > 0, tip hash)
- Step 4 `xxd` WAL bytes (CXW1 magic)
- Step 5 chain summary
- Step 6 calyxd restarted + health pass

## Reference drill result (synthetic vault, executed on aiwonder)

A 3-constellation seed vault, backed up and restored through these scripts,
verified **byte-identical**: `chain_intact:true`, `constellation_count:3`,
`anchor_count:3`, `ledger_entry_count:3`, `wal_bytes_present:2391`, and the
restored `ledger_tip_hash` reproduced the original
`c0c7bfa3086025c19307977f917bae388b88c69a23b949f5364218c72efc25b2` exactly —
the tip-hash match is the byte-faithfulness proof.
