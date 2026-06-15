# PH24 — RRF/WeightedRRF/SingleLens fusion + provenance hits

**Stage:** S4 — Sextant Search & Navigation  ·  **Crate:** `calyx-sextant`  ·
**PRD roadmap:** P3  ·  **Axioms:** A15, A16, A17

## Objective

Multi-lens fusion that beats single-lens recall, with every hit carrying its
full lineage. Three strategies ship: `SingleLens`, `RRF` (`Σ weight_i/(rank_i+60)`),
and `WeightedRRF` with named profiles. Every `Hit` carries `cx_id`, `fused_score`,
`per_lens[(slot, rank, raw_score, weight, contribution)]`, `cross_terms_used`,
`guard`, `provenance: LedgerRef`, and `freshness` (`10 §5`). `explain=true`
makes the breakdown queryable. The FSV gate is multi-lens recall@10 ≥
single-lens + Δ (≥15%) on real qrels (BEIR SciFact subset on aiwonder) plus
every Hit carrying stored non-zero `Constellation.provenance` when available,
with deterministic `stub_ledger` fallback until PH35 real Ledger.

Completing PH24 + a migration shadow is the recommended first demo (`19 §2`).
Note this in every demo-prep step.

## Dependencies

- **Phases:** PH23 (per-slot HNSW — `SlotIndexMap`, `Index` trait),
  PH35 (Ledger hash-chain — `LedgerRef` type; a stub until PH35 is built,
  real after; use the stub from `calyx-core` in the interim)
- **Provides for:** PH25 (sparse lens adds another fusion participant), PH26
  (planner calls the fusion layer), PH40 (temporal boost post-fusion), PH27
  (Loom agreement graph uses per-lens scores), PH62 (CLI `search` command)

## Current state (build off what exists)

`calyx-sextant` has a `SlotIndexMap` and `Index` trait from PH23 plus the
completed fusion stack. `LedgerRef` is a stub in `calyx-core` until PH35. The
top-level `SearchEngine::search` currently fan-outs by calling each selected
slot index and then fusing results; #299 records this as
`per_slot_cpu_index_calls`, not a Forge grouped GPU fan-out.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `crates/calyx-sextant/src/fusion/mod.rs` | `FusionStrategy` enum + `fuse()` dispatcher |
| `crates/calyx-sextant/src/fusion/rrf.rs` | `RRF` and `WeightedRRF` implementations |
| `crates/calyx-sextant/src/fusion/single.rs` | `SingleLens` pass-through |
| `crates/calyx-sextant/src/hit.rs` | `Hit` struct + `FreshnessTag` + `PerLensEntry` |
| `crates/calyx-sextant/src/query.rs` | `Query` struct + `FusionProfile` enum |
| `crates/calyx-sextant/src/search.rs` | top-level `search(query) -> Vec<Hit>` |
| `crates/calyx-sextant/tests/stage4_real_qrels_fsv.rs` | multi-lens vs single-lens recall on BEIR SciFact qrels |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | `Hit` struct + `Query` struct | — |
| T02 | `SingleLens` fusion strategy | T01 |
| T03 | RRF `Σ w/(rank+60)` fusion | T02 |
| T04 | `WeightedRRF` profiles (14 ContextGraph defaults) | T03 |
| T05 | Provenance: attach `LedgerRef` + freshness to every `Hit` | T04 |
| T06 | `explain=true` per-lens breakdown | T05 |
| T07 | Multi-lens recall FSV on real qrels (BEIR SciFact) | T06 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

Run `cargo test -p calyx-sextant --test stage4_real_qrels_fsv beir_scifact_rrf_beats_single_lens_qrels -- --ignored --nocapture` on
aiwonder against the BEIR SciFact qrels subset. The output must print
`single_lens_recall@10=NNN multi_lens_recall@10=NNN delta=NNN` where delta ≥
0.15 (15 percentage points). Additionally, inspect one `Hit` from the result
set and confirm `hit.provenance` is non-zero. Real hash-chain provenance remains
PH35/Stage 7. Both printed values + the `Hit` provenance hex are attached to the
PH24 GitHub issue.

## Risks / landmines

- **LedgerRef stub**: until PH35 ships, use a `LedgerRef::stub(cx_id, seq)` that
  encodes the cx_id + current WAL seq as a placeholder; the stub must have the
  same wire format as the real entry so PH35 can swap it without changing `Hit`
  serialization.
- **qrels dataset**: BEIR SciFact subset must be on aiwonder before the FSV run;
  coordinate with PH69 (Dataset acquisition) but do not block on it — use a
  synthetic 1000-query qrel file for dev, real data for FSV.
- **RRF constant k=60**: this is the standard Cormack et al. value; do not make
  it configurable at this stage (planner will tune later via Anneal in PH46).
- **`explain` overhead**: the spec says ≤ 3 ms overhead for `explain=true`
  (`10 §8`); verify on aiwonder and assert in the FSV test.
