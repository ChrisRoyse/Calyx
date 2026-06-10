# PH33 · T01 — `idx/kernel/` ANN index write + kernel-first search funnel

| Field | Value |
|---|---|
| **Phase** | PH33 — Kernel index + kernel_answer + grounding_gaps |
| **Stage** | S6 — Lodestar Kernel |
| **Crate** | `calyx-lodestar` |
| **Files** | `crates/calyx-lodestar/src/kernel_index.rs` (≤500) |
| **Depends on** | PH32-T05 (`Kernel` struct + members list), PH23 (HNSW index primitives) |
| **Axioms** | A10, A11 |
| **PRD** | `dbprdplans/08 §4.1`, `08 §4` |

## Goal

Build and persist the `idx/kernel/` ANN index over the kernel constellation
embeddings. Implement `kernel_search(query_vec) -> Vec<(CxId, f32)>` that routes
queries kernel-first — searching the measured kernel members for the nearest
anchored nodes, then expanding by association edges. PH32's ≈1% compact-kernel
figure is the raw target; PH33/PH34 read back the actual final/tuned kernel size.
This is the "table of contents anchored to reality" funnel described in
`08 §4.1`.

## Build (checklist of concrete, code-level steps)

- [x] `pub struct KernelIndex { hnsw: HnswIndex, kernel_id: KernelId }` — wraps the
  HNSW index from PH23 restricted to kernel member `CxId`s.
- [x] `pub fn build_kernel_index(kernel: &Kernel, embeddings: &dyn EmbeddingStore) -> Result<KernelIndex, CalyxError>` — fetches embedding vectors for each `CxId` in `kernel.members`; inserts into a fresh HNSW index; returns the `KernelIndex`.
- [x] `pub fn kernel_search(index: &KernelIndex, query_vec: &[f32], top_k: usize) -> Result<Vec<(CxId, f32)>, CalyxError>` — ANN search over the kernel; returns `(CxId, cosine_score)` pairs sorted descending.
- [x] `pub fn write_kernel_index(index: &KernelIndex, store: &dyn KernelStore) -> Result<(), CalyxError>` — persists to `idx/kernel/<kernel_id>/`; atomic write.
- [x] `pub fn load_kernel_index(kernel_id: KernelId, store: &dyn KernelStore) -> Result<KernelIndex, CalyxError>` — loads from `idx/kernel/<kernel_id>/`; missing → `CALYX_KERNEL_INDEX_NOT_FOUND`.
- [x] Embedding dimension mismatch between query and index → `CALYX_KERNEL_DIM_MISMATCH`.
- [x] Empty kernel (0 members) → `CALYX_KERNEL_EMPTY_RESULT` on `build_kernel_index`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: build kernel index from 5 synthetic kernel members with unit-sphere embeddings;
  `kernel_search(members[0].embedding, k=3)` → members[0] is top result with score ≈ 1.0.
- [x] unit: write + load round-trip: `write_kernel_index` then `load_kernel_index` →
  searches return identical results (same top-k order and scores within ε=1e-4).
- [x] unit: `kernel_search` on a query close to member A but far from members B–E →
  A is rank-1; deterministic across identical calls (seeded HNSW).
- [x] edge: `top_k > kernel.members.len()` → returns all `kernel.members.len()` results
  without panic or error.
- [x] edge: loading from non-existent path → `CALYX_KERNEL_INDEX_NOT_FOUND`.
- [x] fail-closed: dim mismatch (query has 128 dims, index has 256) →
  `CALYX_KERNEL_DIM_MISMATCH` (not an out-of-bounds panic).

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `FsKernelStore` JSON bytes at
  `$CALYX_FSV_ROOT/idx/kernel/<test_kernel_id>/index.json`, plus stdout naming
  the readback path.
- **Readback:** run the PH33 kernel-index test with an explicit `CALYX_FSV_ROOT`,
  then separately `ls` and `cat` the written `index.json`.
- **Prove:** round-trip test prints identical top-k results before and after write/load;
  `ls` confirms the index file exists; dim-mismatch test prints `CALYX_KERNEL_DIM_MISMATCH`;
  output attached to PH33 GitHub issue.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH33 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
