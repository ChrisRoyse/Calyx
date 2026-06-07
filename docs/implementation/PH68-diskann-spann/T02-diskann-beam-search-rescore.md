# PH68 · T02 — DiskANN beam search + raw-f32 rescore

| Field | Value |
|---|---|
| **Phase** | PH68 — DiskANN dense + SPANN sparse |
| **Stage** | S17 — Scale: DiskANN + SPANN |
| **Crate** | `calyx-sextant` |
| **Files** | `crates/calyx-sextant/src/index/diskann/search.rs` (≤500) |
| **Depends on** | T01 (this phase — DiskAnnGraphReader), PH23 (SlotIndex trait), PH57 (VRAM budgeter — gates Forge distance dispatches) |
| **Axioms** | A16, A26, A32 |
| **PRD** | `dbprdplans/10 §3`, `dbprdplans/10 §8`, `dbprdplans/04 §3` |

## Goal

Implement beam search over the on-disk DiskANN graph with NVMe-prefetch, and a
raw-f32 rescore stage that reads the cold sidecar (`slot_NN.raw/`) for exact
distance after approximate graph traversal. Implement `DiskAnnSearch` as the
`SlotIndex` for server vaults; beamwidth is a tuneable parameter that Anneal
adjusts in T06. Achieve sub-SLO latency on the `hotpool` NVMe under the
`KernelFirst@1e8 p99 < 25 ms` target (`10 §8`).

> **Scale boundary:** server-only; embedded vaults use in-RAM HNSW (PH23).

## Build (checklist of concrete, code-level steps)

- [ ] Define `DiskAnnSearchParams { beamwidth: usize, ef_search: usize, rescore_k: usize, rescore_from_raw: bool }`; `beamwidth` and `rescore_k` are the Anneal-tunable parameters (T06 registers them)
- [ ] Implement `DiskAnnSearch`: wraps `DiskAnnGraphReader`; holds a visited-node bitset (reused across queries for heap efficiency, cleared per query); `fn search(&self, query: &[f32], k: usize, params: &DiskAnnSearchParams) -> Result<Vec<(u32, f32)>, CalyxError>`
- [ ] Beam search loop: maintain a priority queue (min-heap by approx distance) of size `ef_search`; at each step, read the page-aligned block for the best unvisited node (one `pread` syscall); compute inner-product or L2 against `query` using Forge CPU SIMD; expand neighbors; stop when no unvisited candidate is closer than the worst in the result set
- [ ] I/O prefetch: for the top `beamwidth` unvisited nodes in the candidate set, issue `libc::posix_fadvise(FADV_WILLNEED)` (or `windows::ReadFileEx` on win32) before the synchronous read; ensures NVMe queue depth is saturated on aiwonder
- [ ] Raw-f32 rescore: after beam search returns `rescore_k` candidates, read each candidate's raw f32 vector from `<vault>/cf/slot_NN.raw/<id>` (cold sidecar, `04 §3`) and recompute exact distance; re-rank and return top-k; skip if `rescore_from_raw = false` or sidecar absent
- [ ] Implement the `SlotIndex` trait for `DiskAnnSearch`: `insert` (appends node during incremental ingest; triggers partial graph patch), `search`, `len`, `persist_path`
- [ ] Return `CALYX_INDEX_IO` on any pread/mmap failure; `CALYX_INDEX_DIM_MISMATCH` if query dim ≠ graph dim; never silent fallback to a zero-vector approximation

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: build 1000-node graph (dim=128, seed=42), search with `k=10, beamwidth=32, ef_search=64, rescore_from_raw=false` → assert all 10 results have `id < 1000` and distances are non-negative and non-decreasing in the returned slice
- [ ] unit: exact recall check — for a planted query vector matching node 7 exactly, search must return node 7 as rank-0 with distance ≤ 1e-5 (raw float, no quantization in this test)
- [ ] unit: rescore path — write a `slot_00.raw/` sidecar for 100 nodes; set `rescore_from_raw=true, rescore_k=20, k=5`; verify returned distances equal exact inner-product (≤ 1e-5 tolerance) by comparing against a brute-force pass
- [ ] proptest: for any `k ∈ [1,50]` and `beamwidth ∈ [4,128]`, `search` returns exactly `min(k, node_count)` results; result ids are distinct; distances non-decreasing (seed RNG `17u64`)
- [ ] edge: `k > node_count` → returns all `node_count` nodes without error
- [ ] edge: empty graph (node_count=0) → returns empty Vec, no panic
- [ ] edge: query dim ≠ graph dim → `CALYX_INDEX_DIM_MISMATCH`
- [ ] fail-closed: graph file truncated mid-node → `CALYX_INDEX_IO`, does not return partial results

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** search latency measurement on `hotpool` NVMe + recall@10 on the
  synthetic 1e6-cx warm-up vault (the 1e8-cx full SLO is in T06)
- **Readback:**
  ```
  calyx bench search \
    --vault /zfs/hot/calyx/ph68-warmup-1e6 \
    --strategy DiskAnn \
    --n 500 \
    --report p50,p99
  # Expected: p99 < 10 ms on 1e6 cx (sublinear margin before the 1e8 SLO run)
  ```
- **Prove:** p99 is present in the printed output and is < 10 ms for the 1e6-cx
  vault; a separate `calyx bench recall` run against brute-force shows recall@10
  ≥ 0.90; evidence screenshot attached to PH68 issue

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] CPU↔GPU bit-parity ≤ 1e-3 on the golden distance set (Forge SIMD vs CUDA path for the rescore recompute)
- [ ] FSV evidence (bench p99 output + recall@10 output) attached to the PH68 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
