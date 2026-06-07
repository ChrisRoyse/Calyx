# PH68 · T03 — SPANN centroids-in-RAM + posting-lists-on-NVMe

| Field | Value |
|---|---|
| **Phase** | PH68 — DiskANN dense + SPANN sparse |
| **Stage** | S17 — Scale: DiskANN + SPANN |
| **Crate** | `calyx-sextant` |
| **Files** | `crates/calyx-sextant/src/index/spann/centroids.rs` (≤500), `crates/calyx-sextant/src/index/spann/posting.rs` (≤500) |
| **Depends on** | PH25 (inverted index — sparse SlotIndex trait + posting-list in-RAM baseline that SPANN replaces at server scale) |
| **Axioms** | A16, A26 |
| **PRD** | `dbprdplans/10 §3`, `dbprdplans/04 §8`, `dbprdplans/04 §3` |

## Goal

Implement SPANN: a memory/disk hybrid for sparse slots (SPLADE/keyword) where
cluster centroids (≈ √N entries) live in RAM as a tiny HNSW and posting lists live
on NVMe as varint+zstd compressed blocks. At search time: ANN over centroids → read
relevant posting list blocks from `idx/slot_NN.sparse/` → score candidates. This
keeps query-time RAM proportional to `√N`, not `N`, enabling sparse search at
billion scale (`10 §3`, `04 §8`).

> **Scale boundary:** server-only. Embedded vaults use the in-RAM inverted index
> from PH25.

## Build (checklist of concrete, code-level steps)

### centroids.rs
- [ ] `SpannCentroidIndex`: stores centroid vectors in a small in-RAM HNSW (reuse PH23 `HnswIndex`); maps `centroid_id → posting_list_offset` in a `Vec<u64>`; `fn nearest_centroids(query: &[f32], n_probe: usize) -> Vec<u32>`
- [ ] `fn build_centroids(vectors: &[(u32, Vec<f32>)], n_clusters: usize, seed: u64) -> SpannCentroidIndex`: k-means++ (seeded) to produce `n_clusters` centroids; assign each vector to its nearest centroid; persist centroid vectors + assignment map; `n_clusters` defaults to `(vector_count as f64).sqrt() as usize`
- [ ] `fn assign(vec: &[f32]) -> u32`: returns nearest centroid id (used on ingest to route new vectors to posting lists)
- [ ] Persist centroid state to `<vault>/idx/slot_NN.sparse/centroids.spn` (magic `b"CLXSP001"`); reload on open; `CALYX_INDEX_CORRUPT` on magic mismatch

### posting.rs
- [ ] `PostingListWriter`: appends `(cx_id: u32, sparse_score: f32)` pairs to a posting list block; varint-encodes cx_id deltas (sorted ascending for delta efficiency); zstd-compresses each block; writes to `<vault>/idx/slot_NN.sparse/pl_NNNN.spb`; `fn append(centroid_id: u32, cx_id: u32, score: f32) -> Result<(), CalyxError>`
- [ ] `PostingListReader`: random-access read by centroid_id; reads the block file, decompresses zstd, decodes varint deltas; returns `Vec<(u32, f32)>` (cx_id, score); `fn read_list(centroid_id: u32) -> Result<Vec<(u32, f32)>, CalyxError>`
- [ ] `SpannSearch` (top-level coordinator): `fn search(query: &[f32], k: usize, n_probe: usize) -> Result<Vec<(u32, f32)>, CalyxError>` — probe `n_probe` centroids, read their posting list blocks, merge-sort by score, return top-k
- [ ] Implement `SlotIndex` trait for `SpannSearch`
- [ ] `n_probe` is the Anneal-tunable posting-cutoff (T06 registers it as `posting_cutoff` in the autotune hook)
- [ ] Return `CALYX_INDEX_IO` on block read failure; `CALYX_INDEX_CORRUPT` on zstd decompression error; never return partial/silent results

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit (`centroids.rs`): build 1000-vector centroid index (dim=32, n_clusters=31, seed=7) → `nearest_centroids(query, n_probe=5)` returns exactly 5 distinct centroid ids; all ids < 31
- [ ] unit (`centroids.rs`): centroid file round-trip — save to temp path, reload, assert centroid count and first centroid vector byte-exact
- [ ] unit (`posting.rs`): write 200 cx_ids (sorted, seed=3) to a posting list block, read back → all 200 cx_ids present, scores match input ≤ 1e-5, ids in ascending order
- [ ] unit (`posting.rs`): zstd block decompression: written block size < raw size for a 1000-entry list (compression actually happened)
- [ ] unit: `SpannSearch` end-to-end — 2000 vectors, 44 centroids, search `k=10, n_probe=4, seed=99` → returns exactly 10 results; all result cx_ids < 2000; scores non-decreasing
- [ ] proptest: for any `n_probe ∈ [1, n_clusters]`, search returns `min(k, total_vecs)` results with distinct cx_ids (seed `31u64`)
- [ ] edge: posting list for a centroid with 0 members → returns empty Vec, no error
- [ ] edge: `n_probe > n_clusters` → clamped to `n_clusters`, no error
- [ ] edge: zstd decompress of a corrupted block → `CALYX_INDEX_CORRUPT`
- [ ] fail-closed: centroids.spn magic byte flipped → `CALYX_INDEX_CORRUPT` on open

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `<vault>/idx/slot_00.sparse/` directory on `hotpool` NVMe
  (`/zfs/hot/calyx/<vault>/idx/slot_00.sparse/`)
- **Readback:**
  ```
  ls -lh /zfs/hot/calyx/<vault>/idx/slot_00.sparse/
  # Must show: centroids.spn (non-zero), one or more pl_NNNN.spb files (non-zero)
  xxd /zfs/hot/calyx/<vault>/idx/slot_00.sparse/centroids.spn | head -2
  # Must show magic: 43 4C 58 53 50 30 30 31 ("CLXSP001") at offset 0
  ```
- **Prove:** write a 1e5-cx sparse vault; verify centroid and posting files exist
  on disk with correct magic and non-trivial sizes; run a search query and confirm
  the result cx_ids are present in the base CF (`calyx readback --vault <vault>
  --cx <id>` returns data for each hit); evidence attached to PH68 issue

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines each (line-count gate ✅)
- [ ] FSV evidence (ls output showing both file types + xxd magic + search readback) attached to the PH68 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
