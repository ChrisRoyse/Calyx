# PH27 — Agreement graph + cross-terms (lazy)

**Stage:** S5 — Loom + Assay (DDA & Bits)  ·  **Crate:** `calyx-loom`  ·
**PRD roadmap:** P4  ·  **Axioms:** A8, A9, A31

## Objective

Implement per-constellation agreement vectors and vault-wide agreement graph in
`calyx-loom`, together with all four cross-term kinds under a lazy-by-default
materialization policy. Agreement scalars (`cos(v_a,v_b)`) are always eager;
Delta, Interaction, and Concat are lazy (one matmul on demand + LRU cache) unless
Assay-gates them eager. Storage is `O(n·n_eff)` not `O(n·N²)`: only
Assay-gated pairs are persisted; every other pair remains one matmul away.

> **Honesty is load-bearing:** `C(N,2)` is reported only as an upper bound
> capped by the DPI ceiling and `n_eff` (A8). Cross-terms are derived signals,
> never new external data. Every materialized xterm is tagged `measured` (real
> input through a frozen lens) or `derived` (cross-term). The `abundance_report`
> exposes N, C(N,2), materialized count, n_eff, and the DPI ceiling so the claim
> is defensible — not a slogan.

## Dependencies

- **Phases:** PH24 (RRF/WeightedRRF fusion + Sextant, which provides ANN slot
  vectors and active-pair info), PH13 (Forge CUDA sm_120 batched matmul + SIMD
  CPU parity path used for optional CUDA agreement computation)
- **Provides for:** PH28 (KSG MI needs the agreement graph redundancy pairs),
  PH29 (n_eff uses the redundancy graph), PH30 (abundance_report), PH31
  (Lodestar takes the agreement graph as its kernel-graph seed)

## Current state

✅ **DONE / FSV-signed-off in Stage 5** (`0ada102`). `calyx-loom` now provides
`cross_term.rs`, `materialization.rs`, `lru_cache.rs`, `agreement_graph.rs`,
`blind_spot.rs`, and `abundance.rs`. `calyx-assay` provides the pair-gain gate
that controls eager non-agreement materialization. Final FSV root:
`/home/croyse/calyx/data/fsv-stage5-loom-assay-20260608-final`.

Post-sweep hardening #285 makes Loom cross-term math fail closed: zero-norm,
non-finite, mismatched-dimension, and missing-slot inputs now return cataloged
errors instead of silent `0.0`/truncated `zip` results. Agreement graph readback
retains `raw_mean_agreement`/`mean_agreement` for audit and exposes
`agreement_weight = clamp(raw_mean_agreement, 0, 1)` for nonnegative Lodestar
graph handoff; anti-agreement is preserved raw but not promoted as a positive
edge weight.

Post-sweep hardening #313 makes GPU agreement honest: default builds return
`CALYX_LOOM_FORGE_UNAVAILABLE` from `agreement_batch_gpu` instead of silently
aliasing CPU output; the `calyx-loom/cuda` feature dispatches through Forge CUDA
and has aiwonder compile/execution evidence.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `crates/calyx-loom/src/lib.rs` | Crate root; re-exports public API |
| `crates/calyx-loom/src/cross_term.rs` | `CrossTermKind` enum + `CrossTerm` value type; `agreement_scalar`, `delta_vec`, `interaction_vec`, `concat_vec`; `MaterializationPlan` |
| `crates/calyx-loom/src/materialization.rs` | `plan_cross_terms(cx, panel) -> MaterializationPlan`; lazy vs eager decision; Assay-gate hook |
| `crates/calyx-loom/src/lru_cache.rs` | LRU cache for lazy xterm results keyed `(CxId, SlotId, SlotId, CrossTermKind)`; bounded capacity; TTL eviction |
| `crates/calyx-loom/src/agreement_graph.rs` | Vault-wide agreement graph (sparse adjacency over active pairs); `weave(cx_id)`, `agreement_graph(vault, since_seq?)` |
| `crates/calyx-loom/src/blind_spot.rs` | `blind_spot_detector`: constellation high-sim in lens A / low-sim in lens B vs neighborhood → `BlindSpotAlert` |
| `crates/calyx-loom/src/abundance.rs` | `abundance_report` (N, C(N,2), materialized, n_eff, DPI ceiling, measured/derived counts) |
| `crates/calyx-assay/tests/stage5_fsv.rs` | Unit + FSV-support tests for Stage 5 readbacks |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | `CrossTermKind` types + `agreement_scalar` (eager, always) | — |
| T02 | Lazy xterm compute + LRU cache | T01 |
| T03 | `MaterializationPlan` + `plan_cross_terms` policy | T02 |
| T04 | `agreement_graph` vault-wide + `weave` | T03 |
| T05 | Blind-spot detector | T04 |
| T06 | `abundance_report` skeleton + storage O(n·n_eff) FSV | T04 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

1. **Agreement scalars eager + correct:** ingest a synthetic panel with two known
   slot vectors; call `weave(cx_id)`; read the xterm CF row on aiwonder:
   ```
   calyx readback --cf xterm --cx <id> --kind agreement
   ```
   The scalar must equal `cos(v_a, v_b)` ± 1e-4 computed offline.

2. **Lazy xterm on demand:** call `cross_term(cx_id, a, b, Delta)`; confirm it
   is absent from the xterm CF before the call; confirm it appears in the LRU
   cache after; confirm the value matches the offline delta.

3. **Only agreement is eager at low pair gain:** insert n constellations with
   N=13 lenses; read the xterm CF row count. With low Assay pair gain, eager
   materialization is exactly agreement for each pair (`n*C(13,2)` = `n*78`),
   while Delta/Interaction/Concat remain absent from persisted xterm rows and
   compute into the LRU cache on demand. Prove by:
   ```
   calyx readback --cf xterm --vault <path> --count
   ```
   Stage 5 final FSV readback: `n=50`, `xterm_rows=3900`,
   `lazy_before=3900`, `lazy_after=3900`, `lazy_cache=1`.

4. **Blind-spot fires:** plant a constellation that is cos>0.9 in lens A but
   cos<0.1 in lens B vs its k-nearest neighbors; call `blind_spots(cx_id)`;
   confirm a `BlindSpotAlert` is returned with the correct pair.

Evidence (terminal output) attached to PH27 GitHub issue.

## Risks / landmines

- **C(N,2) honesty:** never store or advertise N·(N-1)/2 rows without gating
  through n_eff and DPI ceiling. The `abundance_report` must print the four
  honest numbers from day one; no stub that shows only C(N,2).
- **LRU cache capacity:** default to `n_eff * N` entries max; do not grow
  unbounded. Bind the clock via the `Clock` trait for TTL, not `Instant::now()`.
- **Forge dispatch:** `agreement_scalar`/`agreement_batch_cpu` are the default
  agreement path. `agreement_batch_gpu` must fail closed unless
  `calyx-loom/cuda` is enabled; with that feature it uses Forge CUDA and must
  retain CPU/GPU parity evidence per A13.
- **VRAM contention:** agreement batch jobs share the RTX 5090 with TEI. Use the
  Forge VRAM budgeter (PH57, or its stub) to avoid OOM.
- **Signed vs unsigned cross-terms:** Delta `v_a − v_b` is directional; pair
  order must be canonical (lexicographic SlotId ordering) to avoid two cache
  entries for the same pair.
