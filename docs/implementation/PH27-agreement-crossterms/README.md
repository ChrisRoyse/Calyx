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
  CPU parity path used for agreement computation)
- **Provides for:** PH28 (KSG MI needs the agreement graph redundancy pairs),
  PH29 (n_eff uses the redundancy graph), PH30 (abundance_report), PH31
  (Lodestar takes the agreement graph as its kernel-graph seed)

## Current state (build off what exists)

`calyx-loom` is a 9-line stub (`src/lib.rs` with a single `pub mod loom;`
placeholder); greenfield. `calyx-assay` is similarly a 9-line stub. Both crates
are already registered in the workspace `Cargo.toml`. Forge batched-cosine and
ANN slot vectors are complete from PH12/PH13; Sextant active-pair metadata is
available from PH24.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `crates/calyx-loom/src/lib.rs` | Crate root; re-exports public API |
| `crates/calyx-loom/src/cross_term.rs` | `CrossTermKind` enum + `CrossTerm` value type; `agreement_scalar`, `delta_vec`, `interaction_vec`, `concat_vec`; `MaterializationPlan` |
| `crates/calyx-loom/src/materialization.rs` | `plan_cross_terms(cx, panel) -> MaterializationPlan`; lazy vs eager decision; Assay-gate hook |
| `crates/calyx-loom/src/lru_cache.rs` | LRU cache for lazy xterm results keyed `(CxId, SlotId, SlotId, CrossTermKind)`; bounded capacity; TTL eviction |
| `crates/calyx-loom/src/agreement_graph.rs` | Vault-wide agreement graph (sparse adjacency over active pairs); `weave(cx_id)`, `agreement_graph(vault, since_seq?)` |
| `crates/calyx-loom/src/blind_spot.rs` | `blind_spot_detector`: constellation high-sim in lens A / low-sim in lens B vs neighborhood → `BlindSpotAlert` |
| `crates/calyx-loom/src/abundance.rs` | `abundance_report` stub (N, C(N,2), materialized, n_eff placeholder, DPI ceiling placeholder) — completed in PH30 |
| `crates/calyx-loom/src/tests.rs` | Unit + property + FSV-support tests |

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

3. **Materialized count ≪ C(N,2):** insert n constellations with N=13 lenses;
   read the xterm CF row count; it must be `n` (agreement scalars only), not
   `n·78` (all pairs). Prove by:
   ```
   calyx readback --cf xterm --vault <path> --count
   ```

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
- **Forge dispatch:** `agreement_scalar` uses Forge `batched_cosine`; vectors
  must be normalized before the call. The CPU SIMD fallback must be bit-parity
  tested (≤1e-3 vs GPU) per A13.
- **VRAM contention:** agreement batch jobs share the RTX 5090 with TEI. Use the
  Forge VRAM budgeter (PH57, or its stub) to avoid OOM.
- **Signed vs unsigned cross-terms:** Delta `v_a − v_b` is directional; pair
  order must be canonical (lexicographic SlotId ordering) to avoid two cache
  entries for the same pair.
