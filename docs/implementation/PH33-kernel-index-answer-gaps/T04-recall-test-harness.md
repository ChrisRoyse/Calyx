# PH33 · T04 — Recall test harness: kernel-only recall ≥ 0.95·full

| Field | Value |
|---|---|
| **Phase** | PH33 — Kernel index + kernel_answer + grounding_gaps |
| **Stage** | S6 — Lodestar Kernel |
| **Crate** | `calyx-lodestar` |
| **Files** | `crates/calyx-lodestar/src/recall_test.rs` (≤500) |
| **Depends on** | T01 (`kernel_search`), T02 (`kernel_answer`), T03 (`grounding_gaps`) |
| **Axioms** | A10 |
| **PRD** | `dbprdplans/08 §3` (Stage 5: Recall test), `08 §7` |

## Goal

Implement the recall test harness: given a corpus and a held-out query set,
measure kernel-only recall (using only the `idx/kernel/` ANN index) against
full-corpus recall (using the full HNSW index from PH23). The gate is
**kernel-only recall ≥ 0.95·full** (`08 §3`, `16_STAGE6_LODESTAR.md` PH33 FSV gate).
The harness is deterministic (seeded RNG), produces a `RecallReport`, and emits
`CALYX_KERNEL_RECALL_BELOW_GATE` if the gate is not met.

## Build (checklist of concrete, code-level steps)

- [ ] `pub struct RecallTestParams { held_out_fraction: f32, top_k: usize, rng_seed: u64, min_recall_ratio: f32 }` — defaults: `held_out_fraction=0.1`, `top_k=10`, `rng_seed=42`, `min_recall_ratio=0.95`.
- [ ] `pub fn kernel_recall_test(kernel_index: &KernelIndex, full_index: &dyn AnnIndex, corpus: &dyn CorpusReader, params: &RecallTestParams) -> RecallReport`:
  1. Sample `held_out_fraction * corpus.len()` queries with `rng_seed`.
  2. For each query: run `kernel_search(query, top_k)` → kernel hits set;
     run `full_index.search(query, top_k)` → full hits set.
  3. `recall_at_k = |kernel_hits ∩ full_hits| / |full_hits|`.
  4. Aggregate: `kernel_only = mean(recall_at_k)`, `full = 1.0` (by definition).
  5. `ratio = kernel_only / full`; if `ratio < params.min_recall_ratio` →
     emit `CALYX_KERNEL_RECALL_BELOW_GATE` in `RecallReport.warning`.
- [ ] `RecallReport` updated: add `recall_test_params`, `corpus_name`, `n_queries_tested`.
- [ ] RNG must use the `Clock`-injected timestamp seed when `rng_seed = 0`; otherwise
  the provided seed exactly (never `thread_rng()`).
- [ ] `corpus.len() == 0` → `CALYX_RECALL_EMPTY_CORPUS`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: 100-item corpus; kernel = top 10 items by embedding norm; held-out
  queries are the same 10 items (seed=42); `kernel_only == 1.0`; `ratio == 1.0`.
- [ ] unit: kernel = 1 random item; queries are 10 diverse items → `recall_at_k`
  near 0.1 for `top_k=10`; `ratio < 0.95`; `warning = CALYX_KERNEL_RECALL_BELOW_GATE`.
- [ ] unit: same `rng_seed=42` on same corpus → exactly same held-out set selected
  (determinism check).
- [ ] unit: `n_queries_tested` in report equals `ceil(0.1 * corpus.len())`.
- [ ] edge: `held_out_fraction = 1.0` → all corpus items used as queries; no panic.
- [ ] edge: `held_out_fraction = 0.0` → `CALYX_RECALL_EMPTY_CORPUS` (no queries to test).
- [ ] fail-closed: `min_recall_ratio > 1.0` → `CALYX_RECALL_INVALID_PARAMS`.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `cargo test -p calyx-lodestar recall_test -- --nocapture` stdout.
- **Readback:** `cargo test -p calyx-lodestar recall_test 2>&1 | tee /tmp/ph33_t04_fsv.txt && cat /tmp/ph33_t04_fsv.txt`.
- **Prove:** unit test with perfect kernel prints `ratio = 1.0`; degraded-kernel
  test prints `CALYX_KERNEL_RECALL_BELOW_GATE`; determinism test prints identical
  held-out sets on two runs; output attached to PH33 GitHub issue.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH33 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
