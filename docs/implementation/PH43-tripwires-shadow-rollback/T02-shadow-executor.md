# PH43 - T02 - Shadow executor (held-out replay + beat-incumbent check)

| Field | Value |
|---|---|
| Phase | PH43 - Tripwires + Shadow-First + Reversible/Rollback |
| Stage | S10 - Anneal + Intelligence Objective J |
| Crate | `calyx-anneal` |
| Files | `crates/calyx-anneal/src/shadow.rs` (<=500) |
| Depends on | T01 (`TripwireRegistry`) |
| Axioms | A14, A15 |
| PRD | `dbprdplans/12 section 6`, `dbprdplans/27 section 4` |

## Goal

Implement `ShadowExecutor`: given a candidate config/artifact and the incumbent
live artifact, run both against a seeded deterministic held-out replay set,
compare every guarded metric through `TripwireRegistry::check`, and return
`ShadowVerdict::Promote` only when the candidate passes all tripwires and does
not regress against the incumbent. Any failure returns
`ShadowVerdict::Revert { reason, metrics }`; the candidate never touches the
live path.

## Implementation

- [x] `HeldOutReplay { queries: Vec<ReplayQuery>, seed: u64 }` with seeded
  deterministic sampling through `HeldOutReplay::sample`.
- [x] `ReplayQuery` carries a query vector plus expected top-k `ReplayAnchor`
  values (`CxId`, similarity).
- [x] `build_replay(source, n, seed)` samples from a `ReplaySource`; live traffic
  order is not part of the API.
- [x] `ShadowExecutor { registry, replay, budget, clock }` receives a
  `BudgetHandle` and injected `&dyn Clock`; `shadow.rs` does not call
  `SystemTime::now()`.
- [x] `run_shadow` accepts distinct candidate and incumbent action types, runs
  both over replay, averages metric
  pairs, checks the candidate through `TripwireRegistry`, and promotes only when
  every metric passes and dominates the incumbent (`RecallAtK` higher is better;
  FAR/FRR/search p99/ingest p95 lower is better).
- [x] Budget exhaustion before replay completes returns
  `ShadowRevertReason::BudgetExhausted` without running the next query.
- [x] Empty replay returns `ShadowRevertReason::InsufficientReplay`.
- [x] Missing or non-finite metrics fail closed with explicit
  `MissingMetric`/`InvalidMetric` reasons and no partial query-count advance.
- [x] Action measurement errors fail closed with
  `MeasurementFailed { side, code }`; no failed query contributes partial
  metrics.

`AnnealAction::apply_shadow` returns `Result<ActionMetricSnapshot>`, the
single-sided metric vector for one action on one query. `MetricSnapshot` is
reserved for the shadow verdict's paired candidate/incumbent comparisons. This
keeps the public types honest: action output is one-sided; verdict output is
paired and auditable.

Artifact-backed live changes use `ArtifactReplayMeasurer::measure(key,
artifact, query)`. `AnnealSubstrate` prepares the rollback snapshot first, then
measures the candidate pointer and the exact `prior_ptr` captured by that
snapshot independently. A missing measurer returns
`CALYX_ANNEAL_SHADOW_MEASUREMENT_MISSING` through a candidate-side
`MeasurementFailed` verdict and rejects the prepared change. This is a safety
boundary, not synthetic promotion evidence: a production path can promote only
after it installs a real artifact-specific measurer for all five metrics.

## Tests

- [x] Candidate recall `0.85` vs incumbent `1.0` reverts with
  `TripwireCrossed(RecallAtK)` and retains paired metric values.
- [x] Candidate dominates incumbent on all five metrics and promotes.
- [x] Proptest checks dominance always promotes for threshold-safe values.
- [x] Empty replay, single-query equality, and budget-zero edges are covered.
- [x] Missing and invalid metric edges fail closed.
- [x] Artifact tests prove distinct candidate/incumbent pointer invocation,
  exact rollback-snapshot prior binding, measurement-error side attribution,
  and missing-measurer rejection with the incumbent pointer unchanged.

## FSV

The original T02 run predated the Ledger integration and recorded a durable
shadow verdict artifact plus a synthetic live config pointer:

- Evidence root:
  `/home/croyse/calyx/data/fsv-issue395-shadow-20260610-2244`
- `shadow-verdicts.json` records the trigger, expected outcome, happy path,
  revert path, and edge-case before/after states.
- `live-config-pointer-before.txt` and `live-config-pointer-after.txt` prove the
  candidate never touched the live path.
- `vault/.anneal/tripwire.toml` is the physical tripwire config used by the run.
- `BLAKE3SUMS.txt` seals the artifacts for byte readback.

That historical evidence is retained for provenance but does not verify a live
artifact measurer. Current verification must use real Calyx/Aster state and the
Anneal Ledger CF before/action/after readback; test fixtures and persisted JSON
are not production measurement evidence.

## Done when

- [x] `cargo check`, `cargo clippy -D warnings`, and focused tests pass on
  aiwonder.
- [x] All touched `.rs` files are <=500 lines.
- [x] aiwonder FSV reads the artifact bytes and BLAKE3 manifest, including a
  candidate recall `0.80` vs incumbent `0.95` revert with `RecallAtK` failing
  and an unchanged live config pointer.
- [x] Evidence is attached to GitHub issue #395 before close.
