# PH27 · T02 — Lazy xterm compute + LRU cache

| Field | Value |
|---|---|
| **Phase** | PH27 — Agreement graph + cross-terms (lazy) |
| **Stage** | S5 — Loom + Assay (DDA & Bits) |
| **Crate** | `calyx-loom` |
| **Files** | `crates/calyx-loom/src/lru_cache.rs` (≤500), `crates/calyx-loom/src/cross_term.rs` (≤500) |
| **Depends on** | T01 (CrossTermKind, agreement_scalar) · PH13 (Forge matmul) |
| **Axioms** | A8, A9, A16 |
| **PRD** | `dbprdplans/06 §3`, `06 §4` |

## Goal

Implement the three lazy cross-term kinds — `Delta` (`v_a − v_b`), `Interaction`
(`v_a ⊙ v_b` blockwise or low-rank `v_aᵀW v_b`), and `Concat` (`[v_a‖v_b]`) —
plus the LRU cache that makes lazy query-time computation cost-free at rest.
Lazy means: not stored in the xterm CF; computed on demand from the two stored
slot vectors; result cached with TTL. This is the mechanism that keeps storage
`O(n·n_eff)` while preserving queryability of all `C(N,2)` pairs.

## Build (checklist of concrete, code-level steps)

- [x] Implement `delta_vec(v_a: &[f32], v_b: &[f32]) -> Vec<f32>`: element-wise subtraction; canonical pair order enforced (lexicographic SlotId → always `v_lower_id − v_higher_id`); result tagged `source: Derived`
- [x] Implement `interaction_vec(v_a: &[f32], v_b: &[f32], mode: InteractionMode, forge: &ForgeHandle) -> Result<Vec<f32>, CalyxError>`:
  - `InteractionMode::Hadamard`: blockwise `v_a ⊙ v_b`
  - `InteractionMode::LowRank { w: &Mat }`: `v_aᵀ W v_b` via Forge matmul; W is a small random matrix (default 64×64 per pair, seeded from `(slot_a_id, slot_b_id)` deterministically)
- [x] Implement `concat_vec(v_a: &[f32], v_b: &[f32]) -> Vec<f32>`: typed, reversible concatenation; stores a `ConcatMeta { dim_a, dim_b }` header in the value so splitting is lossless
- [x] Implement `LruXtermCache`:
  - Key: `(CxId, SlotId, SlotId, CrossTermKind)` (canonical pair order, lower SlotId first)
  - Value: `CrossTerm` + `computed_at: Instant` via the `Clock` trait
  - Capacity: configurable; default `n_eff * N` entries (passed at construction)
  - Eviction: LRU + TTL (configurable; default 10 min); never `Instant::now()` — always `clock.now()`
  - `get_or_compute(key, compute_fn) -> Result<CrossTerm, CalyxError>`
- [x] Implement `cross_term(cx_id, slot_a, slot_b, kind, cache, forge) -> Result<CrossTerm, CalyxError>`:
  - checks xterm CF first (if already materialized, return stored)
  - else calls `cache.get_or_compute(...)` → invokes the correct lazy compute fn
  - never materializes to CF unless the materialization plan marks the pair as eager

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: `delta_vec([1,0],[0,1]) == [1,-1]`; `delta_vec([0,1],[1,0]) == [-1,1]` (canonical pair order reversal changes sign); `concat_vec(a,b)` round-trips to `(a,b)` via `ConcatMeta`
- [x] proptest: `delta_vec(v_a,v_b) == -delta_vec(v_b,v_a)` for all `(v_a,v_b)` (anti-symmetry); `concat_vec` length = `len(a)+len(b)`
- [x] edge: cache eviction fires when capacity is reached (insert capacity+1 entries; first entry is gone); TTL eviction fires after injected clock advance; cache hit on second access returns identical bytes
- [x] fail-closed: `interaction_vec` with mismatched dims → `CALYX_LOOM_DIM_MISMATCH`; `cross_term` on a non-existent `CxId` → `CALYX_ASTER_NOT_FOUND`

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** a lazy Delta cross-term for a planted pair; the xterm CF must NOT contain the row before query; the LRU cache must contain it after query; the value must equal the offline subtraction
- **Readback:**
  ```
  calyx readback --cf xterm --cx <id> --kind delta  # must return NOT_FOUND before query
  cargo test lazy_xterm_delta_on_demand -- --nocapture  # prints cache hit + value
  ```
- **Prove:** run the test twice on the same pair: first call triggers compute (log shows "computed"); second call is a cache hit (log shows "cache hit"); both return identical `Vec<f32>` bytes.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] CPU↔GPU bit-parity ≤ 1e-3 on the interaction_vec golden set (LowRank path uses Forge matmul)
- [x] FSV evidence (readback output / screenshot) attached to the PH27 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
