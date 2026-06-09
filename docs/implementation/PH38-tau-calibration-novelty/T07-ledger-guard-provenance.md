# PH38 · T07 — Ledger provenance for calibration and guard verdicts

| Field | Value |
|---|---|
| **Phase** | PH38 — τ Calibration (Conformal) + Novelty → New Region |
| **Stage** | S8 — Ward Gτ Guard |
| **Crate** | `calyx-ward` + `calyx-ledger` |
| **Issue** | #279 |
| **Depends on** | PH35/PH36 Ledger, PH38 calibration/guard calls, PH38 T06 |
| **PRD** | `09 §3`, `11 §1` |

**STATUS:** DONE / FSV-signed-off in #279.

## Goal

Persist real Ledger entries for Ward calibration and guard verdicts so `τ`,
corpus provenance, per-slot cosines, and pass/fail decisions are auditable and
reproducible. This is not satisfied by `CalibrationMeta` alone.

## Build

- [x] `calibrate()` or its integration wrapper appends calibration provenance:
      `τ`, corpus hash, estimator, FAR, FRR, confidence, timestamp.
- [x] `guard()` / guarded call sites append `kind=Guard` verdict entries with
      guard id, candidate id, per-slot cosines, tau, pass/fail, timestamp.
- [x] Entries are retrievable through PH36 `get_provenance` / `audit`.
- [x] Guard/Ledger FSV uses the #349-signed-off audit contract: unrelated
      quarantined rows are ignored by filtered audit, while relevant/matching
      quarantined Guard rows still fail closed.

## FSV

`calyx readback --cf ledger` (or the current PH36 readback surface) must show a
calibration entry after calibration and a `kind=Guard` verdict entry after a
guard call. Audit must list relevant rows without being confused by unrelated
quarantine rows.

Signed off on aiwonder at
`/home/croyse/calyx/data/fsv-issue279-ward-ledger-provenance-20260609-55fc1da`
with log
`/home/croyse/calyx/data/fsv-issue279-ward-ledger-provenance-20260609-55fc1da.fsv.log`.
The physical `DirectoryLedgerStore` readback found three `.ledger` rows:
calibration row seq `0`, unrelated measure row seq `1`, and guard verdict row
seq `2`. The decoded readback shows payload tags `ward_calibration_v1` and
`ward_guard_verdict_v1`, `audit(kind=Guard)` returns `[0,2]`,
`get_provenance(cx1)` returns `[2]`, and a matching quarantined Guard row fails
closed with `CALYX_LEDGER_CHAIN_BROKEN` while the unrelated quarantined Measure
row is ignored by filtered Guard audit.

Key hashes:

- `issue279-readback.json`:
  `bfa9a060e810bf13c2982468f590ddfb04db414ac3de0d1b81bf716a93510ebf`
- `audit-kind-guard-result.json`:
  `dbfac70bcc8b7f545d007c425a954559435453a3db5380ba7bb62abef6b6f217`
- `provenance-cx1-result.json`:
  `9c78cc5b3708e2db2423e2f029367f53e0f215b9b9626800780ad111b67fd21b`
- row 0 `.ledger`:
  `a6e07738c46d6ecfc4b29e96868982f52822ff5dcc5a091d0c64ed5e90819b13`
- row 2 `.ledger`:
  `f0541277944dc1830cb9a3b88d9c217bae433caf6bb079d0276cbfe3afad57a9`
- FSV log:
  `d646b9efbf214983601a031a4108d36f556790eb093a743a55ec0ac41904bcb5`
