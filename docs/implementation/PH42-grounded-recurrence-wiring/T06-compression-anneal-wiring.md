# PH42 T06 - Compression ratio + Anneal importance/cadence

| Field | Value |
|---|---|
| **Phase** | PH42 - Grounded Recurrence Wiring Across Engines |
| **Stage** | S9 - Temporal & Dedup |
| **Crate** | `calyx-aster` / `calyx-anneal` |
| **Files** | `crates/calyx-aster/src/dedup/compression_ratio.rs` (<=500), `crates/calyx-anneal/src/recurrence_schedule.rs` (<=500) |
| **Depends on** | T01 (this phase), PH41 (frequency + recurrence series) |
| **Axioms** | A29, A25, A26 |
| **PRD** | `dbprdplans/25`, `dbprdplans/23`, `dbprdplans/12` |

## Goal

Wire two remaining recurrence consumers.

1. Compression: the dedup-merge count is the meaning-compression ratio. N occurrences stored as one event plus N-1 occurrences is `N:1` compression, and this ratio is a grounded signal of semantic density (A25). Expose `compression_ratio(cx_id)` as an O(1) read.
2. Anneal: frequency drives importance weighting (frequent = reinforced = more important) and cadence drives adaptive retention/refresh scheduling. Events expected to recur soon should be kept warm; cold events can be tiered.

## Build

**Compression (`calyx-aster/src/dedup/compression_ratio.rs`):**

- [x] Implement `compression_ratio(cx_id: CxId, vault: &Vault) -> Result<CompressionRatio, CalyxError>`.
- [x] Read `frequency` from the Base CF O(1); this is the total count of times this content was observed.
- [x] Return `CompressionRatio { cx_id, original_count: frequency, stored_count: 1, ratio: frequency as f32 }`; `frequency=0` and `frequency=1` both produce `ratio=1.0`.
- [x] Implement `domain_compression_stats(domain: &Domain, vault: &Vault) -> Result<DomainCompressionStats, CalyxError>`.
- [x] Aggregate `total_original`, `total_stored`, `mean_ratio`, and `max_ratio` across CxIds.
- [x] Expose `compression_ratio` and `domain_compression_stats` from the `calyx-aster` lib root.

**Anneal (`calyx-anneal/src/recurrence_schedule.rs`):**

- [x] Define `RecurrenceSchedule { cx_id, importance_weight, next_expected_t, refresh_priority }`.
- [x] Define `RefreshPriority`: `Hot` (cadence < 3600s), `Warm` (cadence < 86400s), `Cold` (cadence >= 86400s), and `OneTime` (no cadence).
- [x] Implement `recurrence_schedule_for(cx_id, vault, clock)`.
- [x] Read frequency from the Base CF through the Aster compression path and cadence/last occurrence from the recurrence series.
- [x] Use the T03 frequency-kernel bonus formula for `importance_weight`.
- [x] Compute `next_expected_t = last_occurrence_t + cadence_secs` when cadence is known.
- [x] Implement `anneal_retention_tier(cx_id, vault, clock)`: `Hot -> Memtable`, `Warm -> SstableTier1`, `Cold/OneTime -> Archive`.
- [x] Initialize `calyx-anneal` with the `recurrence_schedule` module.

## Tests

- [x] Unit: `compression_ratio` with `frequency=1` -> `ratio=1.0`.
- [x] Unit: `compression_ratio` with `frequency=50` -> `ratio=50.0`, `stored_count=1`.
- [x] Unit: `domain_compression_stats` on frequencies [1, 10, 50] -> `total_original=61`, `total_stored=3`, `mean_ratio` near `20.333334`, `max_ratio=50.0`.
- [x] Unit: `recurrence_schedule_for` with cadence 1800s -> `Hot`, 43200s -> `Warm`, 90000s -> `Cold`, no cadence -> `OneTime`.
- [x] Unit: `importance_weight` for `frequency=0` -> 0.0 and `frequency=10_000` -> 1.0.
- [x] Unit: `anneal_retention_tier` maps `Hot -> Memtable` and `Cold -> Archive`.
- [x] Proptest: `importance_weight` stays within `[0.0, 1.0]` for all frequency values.
- [x] Edge: `frequency=0`, `cadence=None` -> `ratio=1.0`, `RefreshPriority::OneTime`, `importance_weight=0.0`.
- [x] Fail-closed: missing Base CF frequency -> `CALYX_DEDUP_MISSING_FREQUENCY`.

## FSV

**Source of truth:** aiwonder vault bytes under `/home/croyse/calyx/data/fsv-issue392-compression-anneal-20260610-2107/vault`, plus persisted PH42 artifact envelopes and CLI readback files.

**Happy path readbacks:**

- CxId: `39393939393939393939393939393939`
- `compression-ratio.json`: `original_count=50`, `stored_count=1`, `ratio=50.0`; artifact BLAKE3 `2c505f4a33bedd032c18a7e018271af0e7b9b278c655a46002dd356791e3f09b`
- `anneal-schedule.json`: `frequency=50`, `cadence_secs=1800.0`, `importance_weight=0.4268878996372223`, `refresh_priority=Hot`, `next_expected_t=1090000`; artifact BLAKE3 `627e61ebf8cff7b53a7941143083b0b4c304b7c05caccc67b17bc2e43fe9b33c`
- CLI readback `compression-ratio --field ratio`: `50.0`
- CLI readback `anneal-schedule --field next_expected_t`: `1090000`

**Manual edge readbacks:**

- Zero frequency: Base CF frequency scalar `0.0`; compression `ratio=1.0`; anneal `importance_weight=0.0`, `refresh_priority=OneTime`.
- Missing frequency: Base CF row exists without `recurrence.frequency`; compression and anneal both fail closed with `CALYX_DEDUP_MISSING_FREQUENCY`.
- Cold cadence: Base CF frequency scalar `2.0`, cadence `90000.0`; anneal returns `refresh_priority=Cold`, `retention_tier=Archive`.

**Additional bytes read:**

- `base-cf-readback.txt` confirms Base CF rows for the happy path and the three edge CxIds.
- `recurrence-series-readback.json` confirms 50 occurrences from `1000000` through `1088200` with cadence `1800.0`.
- `wal-readback.txt` confirms persisted WAL entries for the Base and Recurrence CF writes.
- `vault-tree.txt` confirms persisted `cf/base`, `cf/recurrence`, `cf/ledger`, and WAL files.
- `b3sum-verify.txt` verifies every artifact and vault file in `BLAKE3SUMS.txt`.

## Done when

- [x] `cargo check`, `clippy -D warnings`, and tests green on aiwonder.
- [x] File line-count gate passes (`scripts/linecount.sh`).
- [x] FSV evidence attached to GitHub issue #392.
- [x] No anti-pattern: no flattening, no `C(N,2)` past DPI, nothing "trusted" without grounding, no frozen-lens mutation, no harness-as-FSV.
