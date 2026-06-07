# PH28 · T02 — Random-projection pre-step (high-d)

| Field | Value |
|---|---|
| **Phase** | PH28 — KSG MI + partitioned NMI |
| **Stage** | S5 — Loom + Assay (DDA & Bits) |
| **Crate** | `calyx-assay` |
| **Files** | `crates/calyx-assay/src/projection.rs` (≤500) |
| **Depends on** | T01 (KSG estimator, to be called after projection) |
| **Axioms** | A16 |
| **PRD** | `dbprdplans/07 §2` |

## Goal

Implement the JL random-projection pre-step that reduces high-dimensional slot
vectors to a stable-rank subspace before KSG, controlling k-NN bias from the
curse of dimensionality. The projection matrix is seeded deterministically from
`(slot_a_id, slot_b_id, n_samples)` so results are reproducible. The target
dimension is `min(d, 2·ceil(log2(n)))` per the Johnson–Lindenstrauss lemma.

## Build (checklist of concrete, code-level steps)

- [ ] Implement `jl_project(x: &[Vec<f32>], target_dim: usize, seed: u64, forge: &ForgeHandle) -> Result<Vec<Vec<f32>>, CalyxError>`:
  - generate a `d × target_dim` Gaussian random matrix `R` with entries `N(0, 1/target_dim)` using `ChaCha8Rng::seed_from_u64(seed)`
  - apply `X_proj = X · R` via Forge batched matmul (GPU path on aiwonder; CPU SIMD fallback)
  - result is `n × target_dim` matrix; bit-parity tested CPU↔GPU ≤ 1e-3
- [ ] Implement `projection_seed(slot_a: SlotId, slot_b: SlotId, n: usize) -> u64`: deterministic seed combining slot IDs and sample count via a simple hash (`slot_a_id XOR (slot_b_id << 32) XOR (n as u64 * 6364136223846793005)`)
- [ ] Implement `auto_target_dim(d: usize, n: usize) -> usize`: `min(d, 2 * (n as f32).log2().ceil() as usize).max(1)`; documents the JL connection
- [ ] Expose `project_pair_for_ksg(x: &[Vec<f32>], y: &[Vec<f32>], slot_a: SlotId, slot_b: SlotId) -> Result<(Vec<Vec<f32>>, Vec<Vec<f32>>), CalyxError>`:
  - computes seed, target_dim, projects both; used by `ksg_estimate_continuous` when `d > auto_target_dim(d, n)`
  - if `d <= target_dim`: skip projection (pass-through), log "projection skipped (d ≤ target)"

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: project 100 vectors of dim=768 with n=100 → target_dim = `2*ceil(log2(100))` = 14; output shape is `100×14`
- [ ] unit: same seed + same input → identical output bytes (deterministic)
- [ ] proptest: `jl_project` preserves pairwise distances up to JL distortion: for n=200 random vectors, `|‖x_proj‖ − ‖x‖| / ‖x‖ ≤ ε` for ε proportional to `1/sqrt(target_dim)` (probabilistic, checked at seed=0)
- [ ] edge: `d = 1` → target_dim = 1, projection is identity (single-dim pass-through); `n = 50` (quorum floor) → target_dim ≥ 1, no panic
- [ ] fail-closed: input with inconsistent vector lengths → `CALYX_ASSAY_MISMATCHED_DIM`

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** projection of a 200×1536 matrix (typical TEI output) to `2·ceil(log2(200))=16` dims; seeded at 42
- **Readback:**
  ```
  cargo test projection_shape_deterministic -- --nocapture
  ```
  Output: shape `200×16`, identical on both runs (seed=42), CPU↔GPU within 1e-3.
- **Prove:** after projection, call KSG on the projected vectors and confirm MI estimate is not degenerate (within CI of the known value for the planted Gaussian from T01). This proves the projection does not destroy the MI signal.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] CPU↔GPU bit-parity ≤ 1e-3 on the projection matrix golden set
- [ ] FSV evidence (readback output / screenshot) attached to the PH28 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
