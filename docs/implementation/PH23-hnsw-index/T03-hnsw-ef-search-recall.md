# PH23 · T03 — HNSW `ef` search + brute-force recall harness

| Field | Value |
|---|---|
| **Phase** | PH23 — Per-slot HNSW index |
| **Stage** | S4 — Sextant Search & Navigation |
| **Crate** | `calyx-sextant` |
| **Files** | `crates/calyx-sextant/src/index/hnsw.rs` (≤500), `crates/calyx-sextant/tests/hnsw_recall.rs` (≤500) |
| **Depends on** | T02 (this phase) |
| **Axioms** | A13, A16 |
| **PRD** | `dbprdplans/10 §3`, `dbprdplans/10 §8` |

## Goal

Add `ef`-controlled greedy beam search to `HnswGraph` and a deterministic
brute-force recall harness. The FSV requires recall@10 ≥ target on aiwonder with
SingleLens p99 < 5 ms at 1e6 cx (`10 §8`). This card proves the index is
correct before fusion is added.

## Build (checklist of concrete, code-level steps)

- [ ] `fn search(&self, query: &[f32], k: usize, ef: usize) -> Result<Vec<(CxId, f32)>, CalyxError>`:
      - dim check; `CALYX_SEXTANT_INDEX_EMPTY` if no entry point; `CALYX_SEXTANT_EF_TOO_SMALL` if `ef < k`
      - greedy descend from entry to layer 1 with `ef=1`
      - beam search at layer 0 with candidate heap of size `ef`
      - return top-k by score (cosine or L2 depending on slot's `DistanceMetric`)
      - use Forge CPU distance (`calyx_forge::cpu::cosine_batch`) — no GPU inside search
- [ ] `fn brute_force_search(&self, query: &[f32], k: usize) -> Vec<(CxId, f32)>` —
      linear scan, used only in test/harness, `#[cfg(test)]`-gated
- [ ] `fn recall_at_k(hnsw_results: &[(CxId, f32)], bf_results: &[(CxId, f32)]) -> f64` —
      intersection size / k, utility fn for the harness
- [ ] Harness `tests/hnsw_recall.rs`: build index with N=10_000 random unit
      vectors (seeded), run 100 random queries, assert `recall_at_k(k=10)` ≥ 0.95,
      print measured p99 wall-clock latency using `std::time::Instant` (test-only
      use of wall time is acceptable; `Clock` trait is for logic, not benchmarks)

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: insert 100 nodes (seed=7), search k=5 ef=50 → results subset of
      brute-force top-5 (recall@5 ≥ 0.8 at this tiny scale)
- [ ] unit: `search` returns exactly k results when n ≥ k
- [ ] unit: `search` returns n results when n < k (no panic)
- [ ] proptest: `recall_at_k(bf, bf) == 1.0` for any result list
- [ ] edge: query with dim ≠ index dim → `CALYX_SEXTANT_DIM_MISMATCH`
- [ ] edge: `ef < k` → `CALYX_SEXTANT_EF_TOO_SMALL`
- [ ] edge: empty index → `CALYX_SEXTANT_INDEX_EMPTY`
- [ ] fail-closed: `k=0` → `CALYX_SEXTANT_EF_TOO_SMALL` (or dedicated variant)

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** stdout of `cargo test -p calyx-sextant hnsw_recall -- --nocapture`
  on aiwonder
- **Readback:** `cargo test -p calyx-sextant hnsw_recall -- --nocapture 2>&1 | grep -E 'recall|p99'`
- **Prove:** must print a line like `recall@10=0.97 p99_us=NNN` where recall ≥ 0.95
  and p99 < 5000 µs (5 ms); the exact numbers are read from aiwonder and attached
  to the PH23 GitHub issue as the FSV evidence

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] CPU↔GPU bit-parity ≤ 1e-3 on the Forge golden set (distance kernel reused
      here is already proven in PH13; confirm it is the same call path)
- [ ] FSV evidence (readback output / screenshot) attached to the PH23 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
