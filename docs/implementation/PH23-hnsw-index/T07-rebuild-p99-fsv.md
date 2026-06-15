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
scratch, used by self-heal and after crash recovery). Current PH23 readback
uses `hnsw_recall_aiwonder_fsv` on a 10,000-row synthetic in-RAM HNSW corpus;
#640 adds release-mode 1e6-cx embedded-scale performance FSV with SingleLens
p99=686 us, RRF-6 p99=3570 us, pipeline p99=17507 us, and exact known-I/O
readback on aiwonder.

## Build (checklist of concrete, code-level steps)

- [x] `fn rebuild(&mut self) -> Result<(), CalyxError>`:
      clears layers/entry, iterates `self.nodes`, re-inserts each in order using
      the same RNG seed and `m`/`ef_construction` params → graph is structurally
      equivalent (recall within 1% of pre-rebuild for any query)
- [x] `fn snapshot_vectors(&self) -> Vec<(CxId, Vec<f32>)>` — returns raw (or
      dequantized) vectors for Aster-backed rebuild; `#[cfg(not(test))]` path
      reads from `SlotIndexMap`; test path uses in-memory copy
- [x] Future scale FSV: extend `tests/hnsw_recall.rs` with a `bench_single_lens` test:
      - build `SlotIndexMap` with 1 slot, insert 1_000_000 synthetic unit vecs
        (seeded RNG, 128-dim)
      - run 1000 queries, record wall-clock `Instant` per query
      - compute p99 = sorted[990] latency in microseconds
      - assert p99 < 5000 (i.e. < 5 ms per `10 §8`)
      - print `recall@10=NNN p99_us=NNN` to stdout for FSV capture
- [x] After rebuild, rerun the recall harness → assert recall within 0.01 of
      pre-rebuild value (rebuild must not degrade quality)

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: build 100-node graph, rebuild, compare neighbor sets → recall@5 on
      20 queries ≥ 0.98 (pre vs post rebuild)
- [x] unit: `snapshot_vectors` returns exactly `len()` entries with correct `CxId`
- [x] proptest: rebuild is idempotent — `rebuild(); rebuild()` ≡ `rebuild()`
      (same recall@10 within 0.01 on fixed queries)
- [x] edge: rebuild on empty graph → no panic, `len() == 0`
- [x] edge: rebuild after removing half the nodes → no dangling neighbor pointers
- [x] fail-closed: if Aster vector read returns `CALYX_ASTER_NOT_FOUND` during
      rebuild, the error is propagated (not silently skipped); rebuild halts

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `hnsw-recall-readback.json` written by
  `cargo test -p calyx-sextant hnsw_recall_aiwonder_fsv -- --ignored --nocapture`
  on aiwonder
- **Readback:** the JSON records `n=10000`, `stored_rows=10000`, `recall_at_10`,
  `p99_us`, neighbor counts, layer histogram, and fail-closed edge codes
- **Prove:** PH23 evidence includes the 10,000-row recall/p99 artifact. #640
  adds the 1e6-cx embedded-scale budget artifact at
  `/home/croyse/calyx/data/fsv-issue640-embedded-scale-exactfast-20260611T055130Z/issue640-embedded-scale-latency-series.json`
  (SHA-256 `7c9e2a299329f9d0fb80529bb237d6777717bebe1a68759666ca909a01676ef2`),
  byte-read back on aiwonder.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] Sextant HNSW rebuild/search remains CPU/index-owned; any Sextant CPU/GPU
      parity request fails loud with `CALYX_SEXTANT_GPU_PARITY_UNAVAILABLE`
      until a real Forge GPU search path is wired. Forge PH13 covers kernel
      parity separately.
- [x] FSV evidence (readback output / screenshot) attached to the PH23 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
