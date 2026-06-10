# PH42 T04 - Ward novelty and recurrence surprise

| Field | Value |
|---|---|
| **Phase** | PH42 - Grounded Recurrence Wiring Across Engines |
| **Stage** | S9 - Temporal & Dedup |
| **Crate** | `calyx-ward` |
| **Issue** | #390 |
| **Files** | `crates/calyx-ward/src/novelty.rs`, `crates/calyx-ward/src/error.rs`, `crates/calyx-ward/tests/novelty_recurrence*.rs` |
| **Depends on** | T01 (this phase), PH38, PH41 recurrence frequency |
| **Axioms** | A29, A12, A16 |

## Goal

Ward consumes PH41 recurrence frequency as a fail-closed novelty signal:

- `frequency <= 1` is `NoveltySignal::NonRecurring` and maps to a new novelty region.
- Recurring events with a known cadence become `OverdueRecurrence` when the current clock is past `last_occurrence_t + 2 * cadence_secs`.
- Retrieval anomaly scoring uses `surprise = -log2(p)` where `p = frequency / total_domain_events`.
- `SurpriseScore` is retrieval-only. It must never modify stored constellation bits, lens bits, or information scores.

## Implementation

- `Domain` groups CxIds for domain-local recurrence probability.
- `NoveltySignal` is serialized with snake-case tags: `recurring`, `non_recurring`, `overdue_recurrence`, `anomaly`.
- `classify_novelty(cx_id, vault, clock)` reads `recurrence.frequency` from the base CF through `AsterVault::get`.
- Missing frequency fails closed with `CALYX_WARD_MISSING_FREQUENCY`.
- Non-finite, negative, fractional, or oversized frequency fails closed with `CALYX_WARD_INVALID_FREQUENCY`.
- `surprise_bits(cx_id, domain, vault)` sums unique domain CxId frequencies from the base CF and returns `SurpriseScore`.
- `SurpriseScore(f32)` has a private field and validates finite non-negative values.
- `overdue_recurrence_scan(domain, vault, clock)` returns only overdue recurrence rows.
- `novelty_action_for_signal(signal)` maps `NonRecurring` and `OverdueRecurrence` to `NoveltyAction::NewRegion`; `Anomaly` maps to `NoveltyAction::Quarantine`; plain `Recurring` has no action.

## Tests

- `frequency = 0` and `frequency = 1` both classify as `NonRecurring`.
- `frequency = 10`, cadence `100s`, last occurrence `1000s`, clock `1350s` classifies as `OverdueRecurrence { expected_t: 1100, overdue_by_secs: 250 }`.
- `surprise_bits` matches hand-computed values: `frequency=1` in 100 domain events is about `6.643856`; `frequency=50` in 100 events is `1.0`.
- Empty domain returns `SurpriseScore(0.0)`.
- Missing frequency fails closed with `CALYX_WARD_MISSING_FREQUENCY`.
- Overdue scan and action mapping exercise the Ward routing surface.
- Proptest keeps `surprise_score_from_counts` finite and non-negative for all valid `u64` count pairs.

## FSV

The ignored trigger `novelty_recurrence_fsv.rs` writes a durable aiwonder artifact:

- `ward-novelty.json` v1 envelope for the `ward-novelty` readback surface.
- A durable Aster vault under the FSV root with Base, Recurrence, Ledger, and WAL bytes.
- Happy path: singleton `0101...` -> `non_recurring`; overdue recurring `0303...` -> `overdue_recurrence`; singleton surprise in a 21-event domain -> about `4.392317`.
- Edges: zero frequency, empty domain, missing frequency, invalid fractional frequency.
- Readback must separately inspect artifact fields, Base CF rows, Recurrence CF rows, WAL bytes, and BLAKE3 manifests.
- The surprise f32 hex is recorded in the artifact and must not appear as a stored base/WAL bit value.

## Done When

- aiwonder gates pass: format, line count, diff check, Ward tests, Ward clippy, CLI readback checks.
- The ignored FSV trigger is run on aiwonder, and its artifact manifests verify.
- Manual readback evidence is posted to #390.
- The issue is closed with commit, artifact root, readback values, and edge-case evidence.
