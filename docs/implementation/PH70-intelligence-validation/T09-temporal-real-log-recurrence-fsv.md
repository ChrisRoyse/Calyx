# PH70 - T09 - Temporal Real-Log Recurrence FSV

| Field | Value |
|---|---|
| **Phase** | PH70 - Intelligence validation on real corpora |
| **Stage** | S18 - Datasets & Intelligence FSV |
| **Issue** | #610 |
| **Crate** | `calyx-cli` |
| **Files** | `crates/calyx-cli/src/temporal_log_recurrence_readback/` |
| **Depends on** | PH41 recurrence series, PH62 readback, PH69 temporal log acquisition |

## Goal

Prove the temporal/recurrence intelligence predicate on a real timestamped log:
same-content events at distinct real-log timestamps must fire the recurrence
signature, persist a recurrence series, detect the observed cadence, and let the
Oracle predict the next occurrence.

## Readback Surface

```text
calyx readback temporal-log-recurrence \
  --log <csv> \
  --vault <dir> \
  --out <json> \
  --rows <n> \
  --expected-cadence-secs <secs> \
  --confidence-ceiling <f>
```

The command parses a real CSV whose first column is `YYYY-MM-DD HH:MM:SS`,
ingests the same event content at each selected timestamp through
`dedup::ingest_at` under `DedupAction::RecurrenceSeries`, cold-opens the vault,
then writes a `ph70.temporal-real-log-recurrence.v1` artifact only after the
SoT readback checks pass.

## FSV Source Of Truth

- Real log file bytes and BLAKE3 hash.
- Cold-open Aster `base`, `recurrence`, `online`, and `ledger` CF rows.
- Ledger payloads with `recurrence_signature=true` for every merge after the
  first row.
- `recurrence_series` readback with `cadence_secs` and `periodic_fit`.
- Oracle `predict_next_occurrence` readback with `t_hat` and interval.

## Required Edges

- Empty or too-short log fails closed with `CALYX_TEMPORAL_LOG_EMPTY` and writes
  no artifact.
- Bad timestamp fails closed with `CALYX_TEMPORAL_LOG_BAD_TIMESTAMP` and writes
  no artifact.
- Non-monotonic timestamp fails closed with `CALYX_TEMPORAL_LOG_NON_MONOTONIC`
  and writes no artifact.
- Cadence mismatch fails closed with `CALYX_TEMPORAL_LOG_CADENCE_MISMATCH` and
  writes no artifact.

## Done When

- `cargo check`, focused tests, full `calyx-cli` tests, and clippy pass on
  aiwonder.
- Every touched Rust file is under 500 lines.
- FSV is run on aiwonder against a real NAB machine-temperature log under
  `/zfs/archive/calyx/temporal_logs/`.
- The GitHub issue includes the artifact path, raw CF/WAL readback summary,
  hashes, happy-path values, and edge before/after artifact-absence proof.
