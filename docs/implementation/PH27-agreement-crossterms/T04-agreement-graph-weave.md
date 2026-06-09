# PH27 · T04 — `agreement_graph` vault-wide + `weave`

| Field | Value |
|---|---|
| **Phase** | PH27 — Agreement graph + cross-terms (lazy) |
| **Stage** | S5 — Loom + Assay (DDA & Bits) |
| **Crate** | `calyx-loom` |
| **Files** | `crates/calyx-loom/src/agreement_graph.rs` (≤500) |
| **Depends on** | T03 (MaterializationPlan) · T01 (agreement_scalar, agreement_batch_cpu) |
| **Axioms** | A8, A9, A15, A31 |
| **PRD** | `dbprdplans/06 §5`, `06 §8` |

## Goal

Implement `weave(cx_id)` — the per-constellation entry point that computes the
agreement vector for all active pairs and executes the materialization plan —
and `agreement_graph(vault, since_seq?)` — the vault-wide sparse adjacency over
active pairs used by Lodestar (kernel-graph seed, PH31) and Assay (redundancy
graph, n_eff). The graph retains the raw mean agreement scalar across all
constellations that activated both slots and exposes a separate nonnegative
`agreement_weight = clamp(raw_mean_agreement, 0, 1)` for graph consumers that
require `[0,1]` edge weights.

## Build (checklist of concrete, code-level steps)

- [x] Define `AgreementVector`: `{ cx_id: CxId, pairs: Vec<(SlotId, SlotId, f32)> }` — the per-constellation output of `weave`
- [x] Implement `weave(cx_id, vault, forge, cache, clock) -> Result<(AgreementVector, MaterializationPlan), CalyxError>`:
  - load active slot vectors for `cx_id` from Aster
  - call `agreement_batch` for all active pairs → `AgreementVector`
  - call `plan_cross_terms` with stub assay/sextant hooks
  - persist `EagerStore` Agreement scalars to xterm CF (write via WAL group-commit, tagged `source: Derived`)
  - return both (allows caller to further materialize Interaction/Concat if gated)
- [x] Define `AgreementGraph`: sparse adjacency `{ edges: HashMap<(SlotId, SlotId), raw_mean_agreement, agreement_weight>, panel_version: u64, computed_at_seq: u64 }`; edge raw value = mean agreement over all constellations touching both slots; edge handoff weight is clamped to `[0,1]`
- [x] Implement `agreement_graph(vault, since_seq: Option<u64>, forge) -> Result<AgreementGraph, CalyxError>`:
  - stream all constellations (optionally since a sequence number for incremental updates)
  - accumulate mean agreement scalars per pair using online mean (Welford)
  - return sparse adjacency; edges with mean > 0.6 are the redundancy-graph backbone
- [x] Write a Ledger stub entry for each `weave` call (A15): `LedgerEntry::LoomWeave { cx_id, pairs_computed, seq }`

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: `weave` on a planted 2-slot constellation with `v_a=[1,0]`, `v_b=[0,1]` → agreement scalar 0.0; xterm CF row written with correct bytes
- [x] unit: `agreement_graph` over 10 constellations all with the same `(a,b)` pair at cos=0.8 → edge weight ≈ 0.8 (Welford mean converges); over 100 constellations at cos=0.5 → edge weight ≈ 0.5
- [x] proptest: `weave` is idempotent — calling it twice on the same `cx_id` with identical slot vectors produces the same `AgreementVector` (same bytes)
- [x] edge: constellation with only one active slot → zero active pairs → empty `AgreementVector`; constellation with 13 active slots → 78 pairs in `AgreementVector`
- [x] fail-closed: `weave` on a `CxId` with a missing slot vector → `CALYX_ASTER_NOT_FOUND` (not a silent skip)

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** xterm CF row for a planted `(cx_id, slot_a, slot_b, Agreement)` after `weave`
- **Readback:**
  ```
  calyx readback --cf xterm --cx <id> --kind agreement --slot-a <a> --slot-b <b>
  ```
  The returned bytes must decode to the expected f32 scalar within ±1e-4.
- **Prove:** run `agreement_graph` over 50 planted constellations with a known mean cos=0.75 for pair `(a,b)`; read the graph edge; confirm it is 0.75 ± 0.01. Confirm the edge for a pair not in any constellation is absent (sparse, not zero-filled).

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] default `agreement_batch_gpu` fails closed; `calyx-loom/cuda` executes the Forge CUDA golden path when enabled
- [x] FSV evidence (readback output / screenshot) attached to the PH27 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
