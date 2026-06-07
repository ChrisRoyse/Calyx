# PH68 · T04 — Dual-DiskANN for asymmetric slots

| Field | Value |
|---|---|
| **Phase** | PH68 — DiskANN dense + SPANN sparse |
| **Stage** | S17 — Scale: DiskANN + SPANN |
| **Crate** | `calyx-sextant` |
| **Files** | `crates/calyx-sextant/src/index/diskann/dual.rs` (≤500) |
| **Depends on** | T01 (this phase — DiskAnnGraphReader/Writer), T02 (this phase — DiskAnnSearch), PH23 (asymmetric dual HNSW baseline + Slot.asymmetry field) |
| **Axioms** | A16 |
| **PRD** | `dbprdplans/10 §3`, `dbprdplans/04 §3` |

## Goal

Implement dual-DiskANN for asymmetric slots (`Slot.asymmetry = Dual`): two
independent on-disk graphs (`asym_a` and `asym_b`) encoding directional
relationships (e.g., cause→effect vs. effect→cause). A directional query picks the
appropriate graph; merge mode uses both with a directional boost. Physically,
`asym_a` lives in `idx/slot_NN.asym_a/` and `asym_b` in `idx/slot_NN.asym_b/`
(`04 §3`), mirroring the embedded dual-HNSW layout from PH23 at server scale.

> **Scale boundary:** server-only. Embedded vaults use dual in-RAM HNSW (PH23).

## Build (checklist of concrete, code-level steps)

- [ ] Define `DualDiskAnnSearch`: holds two `DiskAnnSearch` instances, `search_a` and `search_b`, opened from `idx/slot_NN.asym_a/` and `idx/slot_NN.asym_b/` respectively
- [ ] `fn open_dual(vault_path: &Path, slot_id: u8, params: DiskAnnSearchParams) -> Result<DualDiskAnnSearch, CalyxError>`: opens both graph directories; returns `CALYX_INDEX_IO` if either is missing or corrupt; does NOT silently fall back to a single graph
- [ ] Implement `fn search_directional(&self, query: &[f32], direction: Direction, k: usize) -> Result<Vec<(u32, f32)>, CalyxError>` where `Direction` is `Forward | Reverse`; dispatches to `search_a` (Forward) or `search_b` (Reverse); returns `CALYX_INDEX_DIRECTION_UNAVAILABLE` if the requested graph file is absent
- [ ] Implement `fn search_merged(&self, query: &[f32], k: usize, boost: DirectionalBoost) -> Result<Vec<(u32, f32)>, CalyxError>`: searches both graphs, applies `boost.forward_weight` and `boost.reverse_weight` to respective scores, merges by score (highest first), deduplicates by cx_id keeping the better score, returns top-k; `DirectionalBoost { forward_weight: f32, reverse_weight: f32 }` with `forward + reverse == 1.0` enforced
- [ ] Implement `SlotIndex` trait for `DualDiskAnnSearch`: `search` delegates to `search_merged` with default boost (0.5/0.5); `insert` appends to both `search_a` and `search_b` graphs atomically (write to `asym_a`, then `asym_b`; on failure of the second write, log `CALYX_INDEX_IO` and mark slot `degraded` — index is rebuildable)
- [ ] `fn build_dual(vault_path: &Path, slot_id: u8, a_vectors: &[(u32, Vec<f32>)], b_vectors: &[(u32, Vec<f32>)], params: DiskAnnBuildParams) -> Result<(), CalyxError>`: calls `build_diskann_graph` for both; staged inside `hotpool` dataset
- [ ] All error paths return structured `CALYX_*` codes; no silent single-graph fallback when dual is expected

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: build dual graph (200 nodes each direction, dim=8, seed=55) → `search_directional(query, Forward, k=5)` returns 5 results from `asym_a`; `search_directional(query, Reverse, k=5)` returns 5 results from `asym_b`; the two result sets differ (planted asymmetry: a-vectors cluster around `[1,0,…]`, b-vectors around `[0,1,…]`)
- [ ] unit: `search_merged` with boost `{forward=0.7, reverse=0.3}` returns top-k where scores reflect the weighting — a query near a-cluster returns an a-result as rank-0
- [ ] unit: `DirectionalBoost` with `forward + reverse ≠ 1.0` → `CALYX_INDEX_INVALID_PARAMS` before any search
- [ ] unit: `open_dual` with one graph directory missing → `CALYX_INDEX_IO`, no partial open, no fallback to single-graph mode
- [ ] proptest: for any `forward_weight ∈ [0.0, 1.0]`, merged results contain only ids present in at least one of the two graphs; no phantom ids (seed `88u64`)
- [ ] edge: both graphs have 0 nodes → empty result, no panic
- [ ] edge: `k > node_count` in merged mode → returns all unique ids across both graphs (up to combined deduped count)
- [ ] fail-closed: `asym_b/graph.cda` corrupt → `CALYX_INDEX_CORRUPT`; `search_directional(Forward)` still succeeds; `search_directional(Reverse)` returns `CALYX_INDEX_CORRUPT`

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `<vault>/idx/slot_00.asym_a/graph.cda` and
  `<vault>/idx/slot_00.asym_b/graph.cda` on `hotpool` NVMe
- **Readback:**
  ```
  ls -lh /zfs/hot/calyx/<vault>/idx/slot_00.asym_a/graph.cda
  ls -lh /zfs/hot/calyx/<vault>/idx/slot_00.asym_b/graph.cda
  # Both must be non-zero size
  xxd /zfs/hot/calyx/<vault>/idx/slot_00.asym_a/graph.cda | head -2
  xxd /zfs/hot/calyx/<vault>/idx/slot_00.asym_b/graph.cda | head -2
  # Both must show magic CLXDA001 at offset 0
  ```
- **Prove:** build a vault with an asymmetric slot; confirm both graph files exist
  on disk at the correct paths with correct magic; run `calyx search --strategy
  DirectionalForward` and `--strategy DirectionalReverse` on the same query;
  confirm the two result sets differ (asymmetry is real); evidence attached to
  PH68 issue

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (ls + xxd for both asym_a and asym_b, plus differing search results) attached to the PH68 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
