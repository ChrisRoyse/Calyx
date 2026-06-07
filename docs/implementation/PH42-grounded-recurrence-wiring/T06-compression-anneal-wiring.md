# PH42 · T06 — Compression ratio + Anneal importance/cadence

| Field | Value |
|---|---|
| **Phase** | PH42 — Grounded Recurrence Wiring Across Engines |
| **Stage** | S9 — Temporal & Dedup |
| **Crate** | `calyx-aster` / `calyx-anneal` |
| **Files** | `crates/calyx-aster/src/dedup/compression_ratio.rs` (≤500), `crates/calyx-anneal/src/recurrence_schedule.rs` (≤500) |
| **Depends on** | T01 (this phase) · PH41 (frequency + recurrence series) |
| **Axioms** | A29, A25, A26 |
| **PRD** | `dbprdplans/25 §4c`, `dbprdplans/23 §4.4`, `dbprdplans/12 §2` |

## Goal

Wire two remaining recurrence consumers. (1) Compression: the dedup-merge count
for a constellation IS its meaning-compression ratio — N occurrences stored as
one event + N−1 occurrences is `N:1` compression, and this ratio is a grounded
signal of semantic density (A25). Expose `compression_ratio(cx_id)` as an O(1)
read. (2) Anneal: frequency drives importance weighting (frequent = reinforced =
more important) and cadence drives adaptive retention/refresh scheduling (events
expected to recur soon should be kept warm; cold events can be tiered).

## Build (checklist of concrete, code-level steps)

**Compression (`calyx-aster/src/dedup/compression_ratio.rs`):**
- [ ] Implement `compression_ratio(cx_id: CxId, vault: &Vault) -> Result<CompressionRatio, CalyxError>`:
  - read `frequency` from base CF (O(1)) — this is the total count of times this content was observed
  - `CompressionRatio { cx_id, original_count: frequency, stored_count: 1, ratio: frequency as f32 }` — if `frequency = 0` → ratio = 1.0 (no compression)
  - if `frequency = 1` (no dedup occurred) → `ratio = 1.0` (no compression gain)
- [ ] Implement `domain_compression_stats(domain: &Domain, vault: &Vault) -> Result<DomainCompressionStats, CalyxError>`:
  - for each CxId in domain: `compression_ratio`; aggregate sum of `original_count` and `stored_count = len(CxIds)`
  - `mean_ratio = total_original / total_stored`; `max_ratio = max(individual ratios)`
  - return `DomainCompressionStats { total_original, total_stored, mean_ratio, max_ratio }`
- [ ] Expose `compression_ratio` and `domain_compression_stats` from `calyx-aster` lib root

**Anneal (`calyx-anneal/src/recurrence_schedule.rs`):**
- [ ] Define `RecurrenceSchedule { cx_id: CxId, importance_weight: f32, next_expected_t: Option<EpochSecs>, refresh_priority: RefreshPriority }`
- [ ] Define `RefreshPriority` enum: `Hot` (cadence < 3600s) | `Warm` (cadence < 86400s) | `Cold` (cadence ≥ 86400s) | `OneTime` (no cadence)
- [ ] Implement `recurrence_schedule_for(cx_id: CxId, vault: &Vault, clock: &dyn Clock) -> Result<RecurrenceSchedule, CalyxError>`:
  - read `frequency`, `cadence_secs` from base CF / series
  - `importance_weight = frequency_kernel_bonus(frequency)` (reuse T03 formula)
  - `next_expected_t = last_occurrence_t + cadence_secs` if cadence is known
  - `refresh_priority` from cadence thresholds above
  - return `RecurrenceSchedule`
- [ ] Implement `anneal_retention_tier(cx_id: CxId, vault: &Vault, clock: &dyn Clock) -> Result<RetentionTier, CalyxError>`: `Hot` → keep in memtable (warm cache); `Warm` → SSTable tier 1; `Cold` → SSTable tier 2 / archive; feeds PH11 compaction tiering decisions
- [ ] `calyx-anneal` is currently a greenfield stub; initialize `lib.rs` with the `recurrence_schedule` module

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `compression_ratio` with `frequency=1` → `ratio=1.0`
- [ ] unit: `compression_ratio` with `frequency=50` → `ratio=50.0`, `stored_count=1`
- [ ] unit: `domain_compression_stats` on 3 CxIds with frequencies [1, 10, 50] → `total_original=61`, `total_stored=3`, `mean_ratio≈20.3`, `max_ratio=50.0`
- [ ] unit: `recurrence_schedule_for` with cadence=1800s → `RefreshPriority::Hot`; cadence=43200s → `Warm`; cadence=90000s → `Cold`; no cadence → `OneTime`
- [ ] unit: `importance_weight` for frequency=0 → 0.0; frequency=10_000 → 1.0
- [ ] unit: `anneal_retention_tier` Hot → `RetentionTier::Memtable`; Cold → `RetentionTier::Archive`
- [ ] proptest: `importance_weight ∈ [0.0, 1.0]` for all frequency values
- [ ] edge: `frequency=0`, `cadence=None` → `ratio=1.0`, `RefreshPriority::OneTime`, `importance_weight=0.0`
- [ ] fail-closed: `frequency` field absent from base CF → `CALYX_DEDUP_MISSING_FREQUENCY`

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `CompressionRatio` and `RecurrenceSchedule` read from the vault
- **Readback:** (1) after 50 ingests of same content: `calyx readback compression-ratio --cx-id <X>` → print ratio; (2) `calyx readback anneal-schedule --cx-id <X>` → print importance_weight, refresh_priority, next_expected_t
- **Prove:** `ratio = 50.0`; `importance_weight ≈ 0.573` (= `log(51)/log(10001)`); `refresh_priority` matches the cadence computed from the 50 occurrence timestamps

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH42 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
