# PH42 T05 - Sextant Frequency/Recency Recurrence Boost (AP-60)

| Field | Value |
|---|---|
| Phase | PH42 - Grounded Recurrence Wiring Across Engines |
| Stage | S9 - Temporal & Dedup |
| Crate | `calyx-sextant` |
| Main files | `crates/calyx-sextant/src/temporal/recurrence_boost.rs`, `crates/calyx-sextant/src/temporal/boost.rs`, `crates/calyx-sextant/src/temporal/search.rs` |
| Depends on | T01, PH40 temporal boost pipeline, PH41 recurrence series/frequency |
| Axioms | A29, A27 |
| PRD | `dbprdplans/25 section 3`, `dbprdplans/25 section 4c` |

## Goal

Sextant AP-60 search now has an optional post-retrieval recurrence term:
frequent constellations with a recent recurrence receive a bounded boost, while
content score remains dominant and zero-content hits remain zero.

The recurrence contribution is read-only. Sextant reads `recurrence.frequency`
from Aster Base CF and reads the last recurrence timestamp from Recurrence CF;
it never writes recurrence state.

## Implementation

- `RecurrenceBoostConfig` lives in `calyx-core::TemporalPolicy` as
  `recurrence_boost: Option<RecurrenceBoostConfig>`.
- Defaults are `frequency_weight=0.05`, `recency_weight=0.05`, and
  `max_recurrence_boost=0.10`.
- `None` disables recurrence boost and avoids Base/Recurrence CF reads.
- `recurrence_boost_score(cx_id, vault, query_time_secs, config)` returns the
  bounded scalar contribution.
- `recurrence_boost_evidence(...)` returns the explain fields:
  frequency, frequency bonus, frequency component, last occurrence timestamp,
  recency component, and total.
- The frequency formula matches PH42 T03:
  `ln(min(frequency, 10000) + 1) / ln(10001)`.
- Recency uses `score_e2_recency` with
  `DecayFunction::Exponential { half_life_secs: 3600 }`.
- `apply_temporal_boost_with_recurrence` adds the recurrence multiplier beside
  the existing E2/E3/E4 AP-60 multiplier.
- `temporal_search_with_recurrence` and
  `temporal_search_from_primary_with_recurrence` expose the vault-backed path
  without breaking the legacy no-vault temporal search API.
- `Hit.explain.recurrence_boost` carries the readback evidence.
- Any missing/corrupt Base CF row, missing/invalid frequency scalar, or
  recurrence-series read failure returns
  `CALYX_SEXTANT_RECURRENCE_READ_ERROR`.

## Tests

`crates/calyx-sextant/tests/recurrence_boost.rs` covers:

- `frequency=0` and no occurrences produce `total=0.0`.
- `frequency=10` and last occurrence 30 minutes before query time matches the
  hand formula.
- High-frequency/recent recurrence caps at `max_recurrence_boost=0.10`.
- Zero-content hit remains at `0.0` even when recurrence evidence is positive.
- `TemporalPolicy { recurrence_boost: None }` avoids recurrence reads and
  matches the PH40-only path.
- `u64::MAX` frequency caps frequency bonus at `1.0` without overflow.
- Missing Base CF row fails closed with `CALYX_SEXTANT_RECURRENCE_READ_ERROR`.
- Proptest in `recurrence_boost.rs` keeps generated totals inside
  `[0.0, max_recurrence_boost]`.

## FSV

Ignored trigger:

`CALYX_SEXTANT_ISSUE391_FSV_DIR=<root> cargo test -p calyx-sextant --test recurrence_boost_fsv -- --ignored`

Expected source of truth on aiwonder:

- Durable Aster vault under `<root>/vault`.
- `temporal-search-result.json`: `TemporalSearchResult` with
  `explain.recurrence_boost` for CxId-A and CxId-B.
- `base-a-readback.json` and `base-b-readback.json`: Base CF readbacks with
  raw BLAKE3 hashes and stored `recurrence.frequency`.
- `recurrence-series-readback.json`: Recurrence CF row hashes and decoded
  series summaries.
- `expected-arithmetic.json`: hand-computed A-vs-B recurrence delta and actual
  post-boost delta.
- `edge-readbacks.json`: zero-frequency, missing-base, and invalid-frequency
  before/after evidence.
- `BLAKE3SUMS.txt`: hashes for the persisted JSON evidence.

CLI readback:

`calyx readback temporal_search --explain --clock-fixed 1000000 --tz-offset 0`

The command builds a deterministic durable Aster vault with:

- CxId-A: frequency 50, content score 0.70, recent last occurrence.
- CxId-B: frequency 1, content score 0.70, old singleton occurrence.

The expected readback is A final score greater than B final score, with both
hits carrying `explain.recurrence_boost` and the score delta equal to
`0.70 * (boost_a - boost_b)` when temporal alpha is set to zero for the
readback policy.
