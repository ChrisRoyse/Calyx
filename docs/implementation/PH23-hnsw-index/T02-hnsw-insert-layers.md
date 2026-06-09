# PH23 · T02 — HNSW insert + layer management

| Field | Value |
|---|---|
| **Phase** | PH23 — Per-slot HNSW index |
| **Stage** | S4 — Sextant Search & Navigation |
| **Crate** | `calyx-sextant` |
| **Files** | `crates/calyx-sextant/src/index/hnsw.rs` (≤500) |
| **Depends on** | T01 (this phase) |
| **Axioms** | A16, A26 |
| **PRD** | `dbprdplans/10 §3`, `dbprdplans/10 §8` |

## Goal

Implement the HNSW graph data structure with deterministic layer assignment and
insert logic. Nodes live in a `Vec`-backed adjacency list; layer is assigned
from a seeded RNG (`Clock`-injected) so tests are reproducible. This card
delivers the insert path only; search is T03.

## Build (checklist of concrete, code-level steps)

- [x] `HnswGraph` struct:
  ```rust
  pub struct HnswGraph {
      dim: usize,
      m: usize,           // max neighbors per layer (default 16)
      m_max0: usize,      // max neighbors at layer 0 (default 32)
      ef_construction: usize,
      ml: f64,            // 1/ln(m) for layer assignment
      nodes: Vec<HnswNode>,
      entry_point: Option<usize>,
      rng_seed: u64,      // injected; never SystemTime::now()
  }
  ```
- [x] `HnswNode { id: CxId, vec: Vec<f32>, layers: Vec<Vec<usize>> }` — layer
      list stores neighbor node indices (internal), not `CxId`, for O(1) access
- [x] `fn assign_layer(rng_seed: u64, node_idx: usize, ml: f64) -> usize` —
      deterministic from seed+index, no global state
- [x] `fn insert(&mut self, id: CxId, vec: &[f32]) -> Result<(), CalyxError>`:
      dim check → layer assign → greedy search from entry to insertion layer →
      select M neighbors with simple heuristic → bidirectional link → update
      entry if new layer is higher
- [x] Neighbor pruning: when `|neighbors| > m` (or `m_max0` at layer 0) after
      linking, prune to closest M by the configured `DistanceMetric` — call
      `calyx-forge` CPU distance (no GPU alloc inside insert)
- [x] `CalyxError::CALYX_SEXTANT_DIM_MISMATCH` on vec.len() ≠ self.dim

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: insert 1 node → `entry_point == Some(0)`, `len() == 1`
- [x] unit: insert 10 nodes with seed=42 → neighbor count of node 0 is exactly
      the expected value (compute once on aiwonder, lock as golden constant)
- [x] proptest: `insert(id, vec)` then `len()` increases by 1 for any valid vec
      (dim=4, values in [-1,1])
- [x] edge: insert dim-0 vector → `CALYX_SEXTANT_DIM_MISMATCH`
- [x] edge: insert 1 then remove then insert same `CxId` → `len() == 1`
- [x] edge: insert `usize::MAX` neighbors scenario never reached — assert
      neighbor list never exceeds `m_max0` after every insert in a 1000-node run
- [x] fail-closed: `dim=0` at construction → `CALYX_SEXTANT_DIM_MISMATCH`

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** test output from `cargo test -p calyx-sextant hnsw_insert` on aiwonder
- **Readback:** `cargo test -p calyx-sextant -- hnsw_insert --nocapture 2>&1`
- **Prove:** before — no `HnswGraph` type; after — all insert tests pass, neighbor
  list sizes for seed=42 match the locked golden constants printed to stdout

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH23 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
