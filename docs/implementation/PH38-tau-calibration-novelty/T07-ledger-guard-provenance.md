# PH38 · T07 — Ledger provenance for calibration and guard verdicts

| Field | Value |
|---|---|
| **Phase** | PH38 — τ Calibration (Conformal) + Novelty → New Region |
| **Stage** | S8 — Ward Gτ Guard |
| **Crate** | `calyx-ward` + `calyx-ledger` |
| **Issue** | #279 |
| **Depends on** | PH35/PH36 Ledger, PH38 calibration/guard calls, PH38 T06 |
| **PRD** | `09 §3`, `11 §1` |

**STATUS:** OPEN / greenfield in #279.

## Goal

Persist real Ledger entries for Ward calibration and guard verdicts so `τ`,
corpus provenance, per-slot cosines, and pass/fail decisions are auditable and
reproducible. This is not satisfied by `CalibrationMeta` alone.

## Build

- [ ] `calibrate()` or its integration wrapper appends calibration provenance:
      `τ`, corpus hash, estimator, FAR, FRR, confidence, timestamp.
- [ ] `guard()` / guarded call sites append `kind=Guard` verdict entries with
      guard id, candidate id, per-slot cosines, tau, pass/fail, timestamp.
- [ ] Entries are retrievable through PH36 `get_provenance` / `audit`.
- [ ] Guard/Ledger FSV uses the #349-signed-off audit contract: unrelated
      quarantined rows are ignored by filtered audit, while relevant/matching
      quarantined Guard rows still fail closed.

## FSV

`calyx readback --cf ledger` (or the current PH36 readback surface) must show a
calibration entry after calibration and a `kind=Guard` verdict entry after a
guard call. Audit must list relevant rows without being confused by unrelated
quarantine rows.
