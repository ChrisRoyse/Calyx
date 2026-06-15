# PH68 · T01 — DiskANN on-disk graph format + builder

| Field | Value |
|---|---|
| **Phase** | PH68 — DiskANN dense + SPANN sparse |
| **Stage** | S17 — Scale: DiskANN + SPANN |
| **Crate** | `calyx-sextant` |
| **Files** | `crates/calyx-sextant/src/index/diskann/graph.rs` (≤500) |
| **Depends on** | PH23 (HNSW — SlotIndex trait + slot lifecycle that DiskANN must implement) |
| **Axioms** | A15, A16 |
| **PRD** | `dbprdplans/10 §3`, `dbprdplans/04 §8`, `dbprdplans/04 §3` |

## Goal

Define the on-disk graph file format for DiskANN and implement the graph builder
(Vamana-style greedy insert + pruning). Vectors and neighbor lists are co-located
in page-aligned blocks for I/O locality so a single read fetches a node's full
search state. The builder writes a complete graph to `idx/slot_NN.ann/` on the
`hotpool` NVMe dataset.

> **Scale boundary:** this code is server-only. Embedded vaults continue using
> in-RAM HNSW from PH23. Any test that constructs a DiskANN graph must be
> annotated `#[ignore = "server-only"]` and run explicitly on aiwonder.

## Build (checklist of concrete, code-level steps)

- [ ] Define `DiskAnnNode` layout: `[raw_f32_vector (dim × 4 B)] [neighbor_count: u32] [neighbors: [u32; M_MAX]]` packed into a page-aligned block (4 KiB minimum, padded); `const fn node_block_size(dim: usize, m_max: usize) -> usize`
- [ ] Define `DiskAnnHeader` (magic `b"CLXDA001"`, dim, m_max, max_degree, entry_point_id, node_count, format_version) written as first block of the graph file
- [ ] Implement `DiskAnnGraphWriter`: opens `<vault>/idx/slot_NN.ann/graph.cda` (staged inside `hotpool` dataset to avoid `EXDEV`); writes header then node blocks sequentially; `fn write_node(id: u32, vector: &[f32], neighbors: &[u32]) -> Result<(), CalyxError>`
- [ ] Implement `DiskAnnGraphReader`: mmap-opens `graph.cda`; `fn read_node(id: u32) -> DiskAnnNodeRef<'_>` returns a zero-copy view into the mapped region; validate magic on open, return `CALYX_INDEX_CORRUPT` on mismatch
- [ ] Implement `DiskAnnBuilder`: Vamana-style construction — greedy NN search from entry point + candidate pruning (robust prune, `alpha` parameter); `fn build(vectors: &[(u32, Vec<f32>)], params: DiskAnnBuildParams) -> Result<DiskAnnGraphWriter, CalyxError>`; `DiskAnnBuildParams { dim, m_max, ef_construction, alpha: f32 }`
- [ ] Expose `fn open_diskann_graph(path: &Path) -> Result<DiskAnnGraphReader, CalyxError>` and `fn build_diskann_graph(path: &Path, vectors: …, params: …) -> Result<(), CalyxError>` as the public surface
- [ ] All error paths return structured `CALYX_*` codes: `CALYX_INDEX_CORRUPT` (magic/version mismatch), `CALYX_INDEX_IO` (I/O failure), `CALYX_INDEX_INVALID_PARAMS` (dim/m_max/alpha out of range); never panic or silent-fill

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: build a 100-node graph (dim=4, m_max=8, seeded RNG) → read back every node → assert `node.neighbors.len() ≤ m_max` and `node.vector == original_vector` (byte-exact round-trip)
- [ ] unit: `DiskAnnHeader` round-trip: write header, re-open reader, assert all fields equal (magic, dim, m_max, node_count, entry_point_id)
- [ ] unit: `node_block_size` is always a multiple of 4096 for any (dim, m_max) pair in `{(4,8),(128,32),(768,64),(1536,48)}`
- [ ] proptest: for any `dim ∈ [1,2048]` and `m_max ∈ [1,96]`, `node_block_size(dim, m_max) ≥ dim*4 + 4 + m_max*4` (no overflow or truncation)
- [ ] proptest: `build(vecs, params); for each id: read_node(id).vector == vecs[id]` — vectors preserved byte-exact through build+write+read cycle (seed RNG with `42u64`)
- [ ] edge: empty input (`vectors = []`) → `build` returns `CALYX_INDEX_INVALID_PARAMS`, no file written
- [ ] edge: single-node graph → entry_point_id == 0, neighbor list empty, file parseable
- [ ] edge: `m_max = 0` → `CALYX_INDEX_INVALID_PARAMS`
- [ ] fail-closed: flip magic byte in written file → `open_diskann_graph` returns `CALYX_INDEX_CORRUPT`, does not panic

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `<vault>/idx/slot_00.ann/graph.cda` on `hotpool` NVMe
  (`/zfs/hot/calyx/<vault>/idx/slot_00.ann/graph.cda`)
- **Readback:**
  ```
  xxd /zfs/hot/calyx/<vault>/idx/slot_00.ann/graph.cda | head -4
  # Must show magic bytes 43 4C 58 44 41 30 30 31 ("CLXDA001") at offset 0
  ls -lh /zfs/hot/calyx/<vault>/idx/slot_00.ann/graph.cda
  # Must be non-zero size consistent with node_count * node_block_size + header
  ```
- **Prove:** build a 1000-node synthetic graph on aiwonder; `xxd` shows correct
  magic at offset 0; file size = `sizeof(DiskAnnHeader-block) + 1000 * node_block_size(dim, m_max)` (compute expected, compare `ls -l` actual); read back node 0 and node 999 with a test binary and assert vector bytes match input

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (xxd header output + ls size + node readback assertion) attached to the PH68 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
