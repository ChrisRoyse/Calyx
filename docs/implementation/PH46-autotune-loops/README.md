# PH46 — Autotune Loops (Index / Quant / Fusion / Materialization)

**Stage:** S10 — Anneal + Intelligence Objective J  ·  **Crate:** `calyx-anneal`  ·
**PRD roadmap:** `12 §4`, `19 §4`  ·  **Axioms:** A14

## Objective

Implement Calyx's self-optimization layer: a bandit autotuner that continuously
tunes four layers — (1) Forge kernel configs (matmul tile sizes, dtype per slot),
(2) HNSW/DiskANN index params (`ef`, `M`, beamwidth, SPANN cutoffs), (3) quant
level per slot (TurboQuant bit-width), (4) Loom materialization plan (which
cross-terms are eager vs lazy). Each parameter set is keyed `(op, shape, dtype,
device, recall_target)`; A/B comparisons run on live traffic; promotion requires
a measured win with no tripwire regression; every promotion is reversible and
Ledger-logged. The phase FSV gate is a 1e6-query soak on aiwonder showing
`p99 ↓ ≥20%`, no recall regression, no oscillation.

## Dependencies

- **Phases:** PH45 (online heads provide current fusion weights; mistake-closure
  loop must be stable before autotuning fusion), PH16 (autotune config cache —
  PH46 extends it with the bandit exploration policy and the per-shape key store)
- **Provides for:** PH47 (lens proposal uses the tuned materialization plan from
  Loom), PH48 (`J` composite uses `meaning_compression_yield` and
  `kernel_recall` which both benefit from autotuned parameters)

## Current state (build off what exists)

`calyx-anneal` crate: PH43+PH44+PH45 complete. PH16 autotune config cache
exists (per-shape best-known config, A/B logging). PH46 extends this with the
exploration bandit (ε-greedy/Thompson), the four-layer tuning scope, and the
soak harness. Greenfield for the bandit + scope modules.

**Anneal invariants (binding):**
- Every A/B comparison runs on LIVE traffic (not synthetic replay) — real
  workload determines the winner.
- Promotion only on measured win + no tripwire regression.
- Every promotion is reversible (prior config kept) + Ledger-logged.
- Bounded background budget — yields to serving + TEI.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `src/tune/bandit.rs` | ε-greedy/Thompson bandit over config candidates; hysteresis; arm selection |
| `src/tune/scope_forge.rs` | Forge kernel layer: matmul tile/dtype/batch-size tuning per `(op,shape,dtype,device)` |
| `src/tune/scope_index.rs` | Index layer: HNSW `ef`/`M`, DiskANN beamwidth, SPANN cutoffs, quant level per slot |
| `src/tune/scope_loom.rs` | Loom materialization layer: eager vs lazy cross-terms; Concat index |
| `src/tune/scope_storage.rs` | Aster storage layer: compaction cadence, hot/cold tiering, codebook refresh, prefetch |
| `src/tune/ab_runner.rs` | A/B runner on live traffic: shadow candidate vs incumbent; metric collection; promote/revert |
| `src/tune/soak_harness.rs` | 1e6-query soak: drives the A/B loop to convergence; emits p99 series + recall series |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | Bandit (ε-greedy/Thompson, hysteresis, arm selection) | — |
| T02 | Forge kernel scope tuner | T01 |
| T03 | Index + quant scope tuner | T01 |
| T04 | Loom materialization scope tuner | T01 |
| T05 | A/B runner on live traffic | T01 |
| T07 | Aster storage scope tuner + `autotune-report --scope storage` | T01 |
| T06 | 1e6-query soak + FSV (p99 ↓ ≥20%, no recall regression, no oscillation) | T01–T05 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

1e6-query soak on aiwonder: start soak with default configs; run the autotune
loop; after 1e6 queries, read the metric series from `calyx anneal autotune-
report` — `p99 ≤ 0.80 × p99_baseline` (≥20% reduction), `recall@10 ≥
recall@10_baseline` (no regression), `p99_series` shows no oscillation
(monotone improvement window ≥ last 10k queries). Read the Ledger for A/B
promotion entries — each promotion logged with before/after metrics.

Storage FSV addendum for #583: run `calyx anneal autotune-report --scope
storage --cache <json> --vault <dir>` and read the physical storage cache rows
(`op=storage`), per-shape `anneal_bandit` CF rows, and Ledger
`AutotunePromote` rows whose artifact id starts with `storage:`. The storage
scope must prove compaction cadence, hot/cold tiering, codebook refresh, and
prefetch parameters from the cache and Ledger bytes, not from a harness verdict.

## Risks / landmines

- **Live-traffic A/B**: the shadow candidate must not slow the serving path.
  Run the incumbent on the main path; the candidate runs in a parallel goroutine
  with its own budget; results are not returned to the caller (shadow only).
- **Oscillation**: hysteresis in the bandit and in tripwire (PH43 T01) prevents
  A→B→A churn. Require N consecutive wins before promote.
- **Per-shape key explosion**: `(op, shape, dtype, device)` can generate thousands
  of keys for large models. Bucket shapes to nearest power of 2 in each dim.
- **Quant tuning and recall**: tuning quant level down must always be gated on
  recall staying ≥ target. Use Assay's `bits_per_anchor` as the recall proxy.
- **soak_harness.rs** must be deterministic when seeded (for CI-like validation),
  but must also support real live-traffic mode on aiwonder.
- **Storage tradeoffs**: faster reads cannot buy hidden write-amplification,
  cache-miss, tier-temperature, codebook-staleness, or prefetch regressions.
  Storage promotions require lower p99 and non-regression on each storage metric.
