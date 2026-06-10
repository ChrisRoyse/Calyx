# PH41 · T08 — FSV: near-but-distinct NOT merged; conflicting-anchor stays separate; recurring → series (reversible)

| Field | Value |
|---|---|
| **Phase** | PH41 — DedupPolicy TctCosine + Recurrence Series + Signature |
| **Stage** | S9 — Temporal & Dedup |
| **Crate** | `calyx-aster` / `calyx-loom` |
| **Files** | `crates/calyx-aster/src/dedup/*_tests.rs` (≤500 each), `crates/calyx-loom/src/recurrence/tests.rs` (≤500) |
| **Depends on** | T07 (this phase) |
| **Axioms** | A28, A3, A26 |
| **PRD** | `dbprdplans/25 §5`, `dbprdplans/25 §4c` |

## Goal

Write the three formal FSV tests that prove PH41's exit-gate invariants on
aiwonder with byte-level evidence. These tests are the primary artifact the
GitHub issue requires: (1) near-but-distinct pair is NOT merged at calibrated τ;
(2) same-content/opposite-anchor pair stays separate; (3) recurring event →
one constellation + time series, reversible byte-for-byte.

## Build (checklist of concrete, code-level steps)

- [ ] `fsv_near_but_distinct_not_merged`: create vault with `TctCosine { tau: Calibrated, action: Collapse }`. Embed two constellations whose content cosine = 0.87 (known to be below calibrated τ ≈ 0.92 from PH38 conformal calibration). Call `ingest_at` for both. Assert both return `New(CxId)` → two distinct CxIds. Call `calyx readback cx-list` and assert length = 2.
- [ ] `fsv_conflicting_anchor_stays_separate`: create vault with `TctCosine { action: RecurrenceSeries }`. Ingest constellation-A with `SpeakerMatch::Speaker("alice")` and identical content. Ingest constellation-B with `SpeakerMatch::Speaker("bob")` and identical content slots (cos = 1.0). Assert second `ingest_at` returns `New(B)`, not `DedupMerge`. Assert `dedup_audit(B)` shows `anchor_conflict_blocks: [A]`. Assert both CxIds exist in CF.
- [ ] `fsv_recurring_event_series_reversible`: create vault with `TctCosine { action: RecurrenceSeries }`. Ingest same content at t=1000, t=2000, t=3000. Assert: (a) first → `New(CxId-X)` and seeds recurrence occurrence `0`; (b) second → `DedupMerge { into: X, occurrence: 1 }`; (c) third → `DedupMerge { into: X, occurrence: 2 }`. Read `recurrence-series X` → `occurrences = [(1000,_), (2000,_), (3000,_)]`. Read `cx-list` → length = 1. Apply `dedup_undo(dedup_audit(X).reversal_token)`. Read `cx-list` → length = 3. `xxd` each of the 3 restored CxIds' base CF rows; compare byte-for-byte with the bytes written at each original `ingest_at` call.
- [ ] `fsv_temporal_excluded_from_dedup_agreement`: ingest two constellations whose CONTENT slots cos=0.95 (above τ=0.90) but whose temporal slot cosines are 0.30 (very different — different event times). With `DedupPolicy::TctCosine { required_slots: [content_slot_only] }`. Assert dedup fires (`DedupMerge` returned) — confirming temporal slots are NOT part of the required-slots check.
- [ ] `fsv_frequency_count_accurate`: 10 ingests of same content with `RecurrenceSeries`. Assert `SeriesStore::occurrence_count(CxId) == 10`. Assert `read_series(CxId).frequency == 10`.
- [ ] All tests in `#[cfg(test)]`, deterministic, `FixedClock`, seeded RNG

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] `fsv_near_but_distinct_not_merged` passes (2 CxIds confirmed)
- [ ] `fsv_conflicting_anchor_stays_separate` passes (anchor-conflict-blocks confirmed)
- [ ] `fsv_recurring_event_series_reversible` passes (byte-for-byte reversal confirmed)
- [ ] `fsv_temporal_excluded_from_dedup_agreement` passes (temporal slots not in required-slots)
- [ ] `fsv_frequency_count_accurate` passes (count=10)
- [ ] proptest: no pair of constellations with `anchor_conflict` ever appears in the same `DedupMerge` (property holds for 1000 random pairs)
- [ ] edge: `fsv_recurring_event_series_reversible` with rollup triggered (>10_000 occurrences) → frequency still accurate, rollup_summary present
- [ ] fail-closed: `dedup_undo` on a partial reversal (crash mid-undo) → Ledger chain detects incomplete undo, full undo retried safely

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** the Aster base/slot CF bytes, recurrence-series CF rows, Ledger rows,
  and readback artifacts under the issue-specific aiwonder FSV root.
- **Readback:** after triggering ingest/merge/undo, run `calyx readback cx-list`,
  `calyx readback recurrence-series`, `calyx readback dedup-audit`, and raw
  CF/`xxd` reads for the affected CxIds; record BLAKE3 hashes for every
  artifact and vault file.
- **Prove:** tests may trigger the scenario, but the verdict is the separate
  byte readback: near-distinct has two base CF rows, conflicting anchors remain
  separate with an audit block record, recurring events store one Cx plus the
  expected occurrence rows, and undo restores three byte-identical Cx rows.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH41 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
