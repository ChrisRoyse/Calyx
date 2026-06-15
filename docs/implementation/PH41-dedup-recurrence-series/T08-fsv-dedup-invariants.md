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

Write the five formal FSV fixtures that prove PH41's exit-gate invariants on
aiwonder with byte-level evidence. These fixtures are the primary artifact the
GitHub issue requires: (1) near-but-distinct pair is NOT merged at calibrated τ;
(2) same-content/opposite-anchor pair stays separate; (3) recurring event is
reversible byte-for-byte across all restored base rows; (4) temporal slots are
excluded from dedup agreement; (5) recurrence frequency reads back accurately.

## Implementation checkpoint and FSV sign-off (2026-06-10)

`#386` is implemented and FSV-signed-off on aiwonder at
`/home/croyse/calyx/data/fsv-issue386-dedup-invariants-20260610-5fdab01`.
The root did not exist before the trigger; after the trigger, the separate
after-read verified 164 files and BLAKE3-checked every vault/artifact row in
`BLAKE3SUMS.txt`. Artifact hashes:

- `dedup-invariants-readback.json` BLAKE3
  `f568a21145a811671c79f2cba56b08eee36b6536fa64dbd598ee73d5d527e140`
- `BLAKE3SUMS.txt` BLAKE3
  `fdda61062034e8d10c4a99e509166e7338b9bc62d6454d8ed3c66fefea33eb87`

Direct aiwonder `calyx readback` calls confirmed the base, slot, online,
recurrence, and ledger CF bytes for all five fixtures, including recurrence
tombstones for occurrence ids 0, 1, and 2 after undo.

## Build (checklist of concrete, code-level steps)

- [x] `fsv_near_but_distinct_not_merged`: create vault with `TctCosine { tau: Calibrated, action: Collapse }`. Embed two constellations whose content cosine = 0.87 (known to be below calibrated τ ≈ 0.92 from PH38 conformal calibration). Call `ingest_at` for both. Assert both return `New(CxId)` → two distinct CxIds. Call `calyx readback cx-list` and assert length = 2.
- [x] `fsv_conflicting_anchor_stays_separate`: create vault with `TctCosine { action: RecurrenceSeries }`. Ingest constellation-A with `SpeakerMatch::Speaker("alice")` and identical content. Ingest constellation-B with `SpeakerMatch::Speaker("bob")` and identical content slots (cos = 1.0). Assert second `ingest_at` returns `New(B)`, not `DedupMerge`. Assert `dedup_audit(B)` shows `anchor_conflict_blocks: [A]`. Assert both CxIds exist in CF.
- [x] `fsv_recurring_event_series_reversible`: create vault with `TctCosine { action: RecurrenceSeries }`. Ingest same content at t=1000, t=2000, t=3000. Assert: (a) first → `New(CxId-X)` and seeds recurrence occurrence `0`; (b) second → `DedupMerge { into: X, occurrence: 1 }`; (c) third → `DedupMerge { into: X, occurrence: 2 }`. Read `recurrence-series X` → `occurrences = [(1000,_), (2000,_), (3000,_)]`. Read `cx-list` → length = 1. Apply `dedup_undo(dedup_audit(X).reversal_token)`. Read `cx-list` → length = 3. `xxd` each of the 3 restored CxIds' base CF rows; compare byte-for-byte with the bytes written at each original `ingest_at` call.
- [x] `fsv_temporal_excluded_from_dedup_agreement`: ingest two constellations whose CONTENT slots cos=0.95 (above τ=0.90) but whose temporal slot cosines are 0.30 (very different — different event times). With `DedupPolicy::TctCosine { required_slots: [content_slot_only] }`. Assert dedup fires (`DedupMerge` returned) — confirming temporal slots are NOT part of the required-slots check.
- [x] `fsv_frequency_count_accurate`: 10 ingests of same content with `RecurrenceSeries`. Assert `SeriesStore::occurrence_count(CxId) == 10`. Assert `read_series(CxId).frequency == 10`.
- [x] All tests in `#[cfg(test)]`, deterministic, `FixedClock`, seeded RNG

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] `fsv_near_but_distinct_not_merged` passes (2 CxIds confirmed)
- [x] `fsv_conflicting_anchor_stays_separate` passes (anchor-conflict-blocks confirmed)
- [x] `fsv_recurring_event_series_reversible` passes (byte-for-byte reversal confirmed)
- [x] `fsv_temporal_excluded_from_dedup_agreement` passes (temporal slots not in required-slots)
- [x] `fsv_frequency_count_accurate` passes (count=10)
- [ ] #626 follow-up/property: no pair of constellations with `anchor_conflict` ever appears in the same `DedupMerge`
- [x] follow-up #620: rollup-triggered recurrence keeps active rows bounded, prunes tombstone rows from the active compacted SST, and cold-reopens with frequency intact
- [x] follow-up #622: exact WAL/crash-injection proof keeps `CALYX_DISK_PRESSURE` and reads unchanged base/recurrence/online/ledger bytes
- [ ] #628 follow-up/FSV: dedup undo after rolled recurrence summary clears stale summary state after compaction/reopen

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
  Recurrence undo is append-only: the after-read must show the logical series
  empty through `readback recurrence-series` and raw recurrence CF tombstone rows
  for the prior occurrence ids rather than deleted historical bytes.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to GitHub issue #386
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
