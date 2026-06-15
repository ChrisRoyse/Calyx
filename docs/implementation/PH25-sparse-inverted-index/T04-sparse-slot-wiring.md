# PH25 · T04 — Sparse `Index` impl + `SlotIndexMap` wiring

| Field | Value |
|---|---|
| **Phase** | PH25 — Sparse lens inverted index |
| **Stage** | S4 — Sextant Search & Navigation |
| **Crate** | `calyx-sextant` |
| **Files** | `crates/calyx-sextant/src/index/inverted.rs` (≤500), `crates/calyx-sextant/src/slot_index_map.rs` (≤500) |
| **Depends on** | T03 (this phase) · PH23 T06 (`SlotIndexMap`) · PH20 (`SlotKind`) |
| **Axioms** | A16, A19 |
| **PRD** | `dbprdplans/10 §3` |

## Goal

Wire `InvertedIndex` as a proper `Index` implementation so it can be registered
in `SlotIndexMap` alongside the dense HNSW slots. RRF and WeightedRRF fusion
must work without change: sparse and dense slots are peers in the fusion map.

Post-sweep #323 adds a required readback invariant for this wiring: sparse
vector inserts must preserve original non-contiguous `SparseEntry` IDs and
weights through `SlotIndexMap::vector`, including after `rebuild`; text inserts
must clear stale sparse-vector readback.

## Build (checklist of concrete, code-level steps)

- [x] Implement `Index` for `InvertedIndex` (completing the stub from T02):
      - `insert(id: CxId, vec: &[f32])` — see T02 note on text encoding; add a
        parallel `insert_text(id: CxId, text: &str)` method that the ingest path
        calls directly when it knows the slot is sparse; the `vec` path decodes
        UTF-8 bytes from the f32 array (a transmission convention, not computation)
      - `search(query: &[f32], k: usize, ef: usize)` — decode query text from
        the vec, tokenize, BM25-score, return top-k `(CxId, f32)`;
        `ef` is ignored for inverted index (document it)
      - `remove(id: CxId)` — tombstone the internal doc_id
      - `rebuild()` — re-ingest all non-tombstoned documents from scratch
      - `dim()` — returns 0 (sparse index has no fixed dimension; callers must
        check `SlotKind` before calling dim-dependent operations)
      - `len()` — returns `total_docs` minus tombstone count
- [x] Update `SlotIndexMap::register` to accept `SlotKind` alongside the index;
      add `fn kind_of(&self, slot: SlotId) -> Option<SlotKind>` for planner use
- [x] `SlotKind::Sparse` variant added to the enum from PH24 T04; assert that
      `WeightedRRF("lexical")` profile's `slot_weights` now matches `SlotKind::Sparse`
- [x] Ingest path: when adding a constellation to a vault, the registry calls
      `insert_text` for sparse slots and `insert` (float vec) for dense slots —
      this routing lives in `search.rs` near the `EmbedQuery` trait

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: register one HNSW slot + one InvertedIndex slot in `SlotIndexMap`;
      insert the same 5 constellations with both text and float vecs; `RRF` search
      returns hits with `per_lens.len() == 2` (both slots contribute)
- [x] unit: `kind_of(sparse_slot) == Some(SlotKind::Sparse)`,
      `kind_of(dense_slot) == Some(SlotKind::Dense)`
- [x] unit: `WeightedRRF("lexical")` search on a two-slot map → only the sparse
      slot contributes (dense weight=0.0)
- [x] unit: `dim()` on `InvertedIndex` returns 0; dense search guarded by
      `SlotKind` check does not call `dim()` on sparse slots
- [x] edge: `search` on InvertedIndex with empty query text → `Ok(vec![])` (no
      query tokens → no candidates, not an error)
- [x] edge: `insert_text` on a dense slot → `CALYX_SEXTANT_WRONG_INDEX_KIND`
- [x] fail-closed: registering two sparse slots with the same `SlotId` →
      `CALYX_SEXTANT_SLOT_ALREADY_REGISTERED`

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** test output of `cargo test -p calyx-sextant sparse_slot_wiring -- --nocapture`
- **Readback:** `cargo test -p calyx-sextant sparse_slot_wiring -- --nocapture 2>&1`
- **Prove:** prints `rrf_hits=5 per_lens_len=2 lexical_profile_dense_excluded=true`
- **Post-sweep #323 SoT:**
  `/home/croyse/calyx/data/fsv-issue323-sparse-vector-readback-20260608/sparse-vector-readback.json`

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH25 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
