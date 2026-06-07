# PH23 · T07 — Rebuild-from-base + SingleLens p99 FSV

| Field | Value |
|---|---|
| **Phase** | PH23 — Per-slot HNSW index |
| **Stage** | S4 — Sextant Search & Navigation |
| **Crate** | `calyx-sextant` |
| **Files** | `crates/calyx-sextant/src/index/hnsw.rs` (≤500), `crates/calyx-sextant/tests/hnsw_recall.rs` (≤500) |
| **Depends on** | T06 (this phase) · PH09 (Aster CRUD for reading base vectors) |
| **Axioms** | A15, A16 |
| **PRD** | `dbprdplans/10 §3`, `dbprdplans/10 §8`, `dbprdplans/19 §4` |

## Goal

Implement `rebuild()` on `HnswGraph` (re-inserts all stored vectors from
scratch, used by self-heal and after crash recovery) and deliver the final
SingleLens p99 FSV: the definitive byte-proven latency measurement on aiwonder
that satisfies the PH23 exit gate. This is the card that closes PH23.

## Build (checklist of concrete, code-level steps)

- [ ] `fn rebuild(&mut self) -> Result<(), CalyxError>`:
      clears layers/entry, iterates `self.nodes`, re-inserts each in order using
      the same RNG seed and `m`/`ef_construction` params → graph is structurally
      equivalent (recall within 1% of pre-rebuild for any query)
- [ ] `fn snapshot_vectors(&self) -> Vec<(CxId, Vec<f32>)>` — returns raw (or
      dequantized) vectors for Aster-backed rebuild; `#[cfg(not(test))]` path
      reads from `SlotIndexMap`; test path uses in-memory copy
- [ ] Extend `tests/hnsw_recall.rs` with a `bench_single_lens` test:
      - build `SlotIndexMap` with 1 slot, insert 1_000_000 synthetic unit vecs
        (seeded RNG, 128-dim)
      - run 1000 queries, record wall-clock `Instant` per query
      - compute p99 = sorted[990] latency in microseconds
      - assert p99 < 5000 (i.e. < 5 ms per `10 §8`)
      - print `recall@10=NNN p99_us=NNN` to stdout for FSV capture
- [ ] After rebuild, rerun the recall harness → assert recall within 0.01 of
      pre-rebuild value (rebuild must not degrade quality)

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: build 100-node graph, rebuild, compare neighbor sets → recall@5 on
      20 queries ≥ 0.98 (pre vs post rebuild)
- [ ] unit: `snapshot_vectors` returns exactly `len()` entries with correct `CxId`
- [ ] proptest: rebuild is idempotent — `rebuild(); rebuild()` ≡ `rebuild()`
      (same recall@10 within 0.01 on fixed queries)
- [ ] edge: rebuild on empty graph → no panic, `len() == 0`
- [ ] edge: rebuild after removing half the nodes → no dangling neighbor pointers
- [ ] fail-closed: if Aster vector read returns `CALYX_ASTER_NOT_FOUND` during
      rebuild, the error is propagated (not silently skipped); rebuild halts

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** stdout of `cargo test -p calyx-sextant bench_single_lens -- --nocapture --ignored`
  on aiwonder (marked `#[ignore]` to skip in fast CI, run explicitly for FSV)
- **Readback:** `cargo test -p calyx-sextant bench_single_lens -- --nocapture --ignored 2>&1 | grep -E 'recall|p99'`
- **Prove:** must print a line matching `recall@10=0.9[5-9]+ p99_us=[0-9]+` where
  p99_us < 5000; screenshot or copy of this line attached to the PH23 GitHub
  issue as the final FSV evidence; additionally, the rebuild regression line
  prints `post_rebuild_recall=0.9[5-9]+`

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] CPU↔GPU bit-parity ≤ 1e-3 on the golden set (Forge distance path)
- [ ] FSV evidence (readback output / screenshot) attached to the PH23 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
