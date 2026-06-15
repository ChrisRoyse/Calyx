# PH51 · T01 — Energy function + β-softmax + descent step

| Field | Value |
|---|---|
| **Phase** | PH51 — `complete()` unified primitive |
| **Stage** | S11 — Oracle & AGI Layer |
| **Crate** | `calyx-oracle` |
| **Files** | `crates/calyx-oracle/src/energy.rs` (≤500) |
| **Depends on** | PH13 (Forge `batched_cosine` + softmax), PH37 (Gτ region members), PH46 (Anneal β autotune) |
| **Axioms** | A2, A16, A20 |
| **PRD** | `dbprdplans/26 §3`, `dbprdplans/26 §11.1` |

## Goal

Implement the energy function `E(x) = −log Σ_i exp(β · sim(x, cx_i))` over candidate region
members and a gradient-free descent step that updates a free slot vector toward the minimum.
This is the mathematical substrate of `complete()`. Reuses Forge `batched_cosine` and softmax;
β is a tunable sharpness parameter retrieved from Anneal's autotuned config. The descent step
is gradient-free (coordinate-wise update: replace the free slot vector with the softmax-weighted
centroid of region members). Convergence criterion: energy change between steps < ε (default 1e-4).

## Build (checklist of concrete, code-level steps)

- [ ] `pub fn energy(x: &[f32], region_members: &[&[f32]], beta: f32) -> f32` — computes `−log Σ_i exp(β · cosine_sim(x, cx_i))`; uses Forge `batched_cosine(x, region_members)` for the similarity vector; applies `log_sum_exp` (numerically stable); negates → energy scalar
- [ ] `pub fn energy_softmax_weights(x: &[f32], region_members: &[&[f32]], beta: f32) -> Vec<f32>` — returns the softmax weights `exp(β·sim_i) / Σ exp(β·sim_j)`; reuses Forge softmax
- [ ] `pub fn descent_step(free_slot: &mut [f32], region_members: &[&[f32]], beta: f32)` — compute softmax weights; `free_slot ← Σ_i w_i · cx_i` (softmax-weighted centroid); normalize result to unit norm (L2 normalize via Forge); one step of gradient-free descent
- [ ] `pub fn descend(free_slot: &mut [f32], region_members: &[&[f32]], beta: f32, max_steps: usize, eps: f32) -> DescentResult` — run up to `max_steps` descent steps; stop early when `|E_{t+1} - E_t| < eps`; return `DescentResult { steps_taken, converged, final_energy }`
- [ ] `pub fn get_beta(domain: DomainId, anneal: &dyn AnnealConfig) -> f32` — retrieve Anneal-tuned β for the domain; default `beta = 1.0` if not yet tuned (Anneal tunes lazily)
- [ ] `MAX_STEPS = 20`, `DEFAULT_EPS = 1e-4`, `DEFAULT_BETA = 1.0` as named constants
- [ ] All floats `f32`; no `f64` creep; CPU path always available (CUDA optional, same bit-parity guarantee as PH13)

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `energy([1,0], [[1,0],[0,1]], beta=2.0)` = `−log(exp(2) + exp(0))` = known scalar ± 1e-4
- [ ] unit: `descent_step` applied 5 times to a vector equidistant from two attractors converges to a known midpoint ± 1e-3
- [ ] unit: `descend` on a 2-attractor synthetic system converges within `MAX_STEPS`; `DescentResult.converged = true`
- [ ] proptest: for all `beta > 0`, `energy` is finite for unit-norm inputs; softmax weights sum to `1.0 ± 1e-5`
- [ ] edge (≥3): single region member → descent converges in 1 step to that member; `beta = 0` → uniform weights → centroid of all members; empty region → `Err` (no members to descend toward)
- [ ] fail-closed: `region_members` empty → structured error; not a panic or NaN

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `crates/calyx-oracle/src/energy.rs`; test output for known energy computation
- **Readback:** `cargo test -p calyx-oracle -- energy --nocapture` prints energy values; `grep "final_energy"` shows convergence
- **Prove:** known 2-attractor energy value correct ± 1e-4; `descent_step` moves toward the closer attractor (cosine similarity to closer attractor increases monotonically over steps)

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] [Forge-touching] CPU↔GPU bit-parity ≤ 1e-3 on the golden set (Forge cosine + softmax)
- [ ] FSV evidence (readback output / screenshot) attached to the PH51 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
