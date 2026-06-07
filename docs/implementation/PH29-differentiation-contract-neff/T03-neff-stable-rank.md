# PH29 · T03 — `n_eff` stable rank of redundancy graph

| Field | Value |
|---|---|
| **Phase** | PH29 — Differentiation contract + n_eff |
| **Stage** | S5 — Loom + Assay (DDA & Bits) |
| **Crate** | `calyx-assay` |
| **Files** | `crates/calyx-assay/src/n_eff.rs` (≤500) |
| **Depends on** | T01 (AdmitResult, pair corr) · PH27 T04 (agreement_graph, edge weights) |
| **Axioms** | A9 |
| **PRD** | `dbprdplans/07 §1`, `26 §5` |

## Goal

Compute `n_eff` — the effective number of non-redundant lenses — as the stable
rank of the redundancy graph adjacency matrix (ratio of squared sum to sum of
squares of agreement eigenvalues). This replaces the `Provisional(N as f32)`
placeholder set in PH27 T06. `n_eff` drives the materialization budget
(`O(n·n_eff)` not `O(n·N²)`) and the LRU cache capacity. It is the `n_eff`
described in A9 and `06 §2`.

## Build (checklist of concrete, code-level steps)

- [ ] Implement `n_eff_from_agreement_graph(graph: &AgreementGraph, forge: &ForgeHandle) -> Result<NeffEstimate, CalyxError>`:
  - construct the `N×N` agreement matrix `A[i][j] = mean agreement scalar for pair (i,j)` from the sparse adjacency (fill missing pairs with 0.0)
  - compute the eigenvalues of `A` using the power method / Lanczos (Forge sparse eigensolver; N is small ≤ ~30 for shipped panels, so dense fallback is acceptable)
  - `stable_rank = (Σ λ_i)² / Σ λ_i²` — the standard stable rank formula; sum over all eigenvalues
  - return `NeffEstimate::Computed { value: stable_rank, ci_low, ci_high }` where CI is from a bootstrap over the agreement scalars (200 resamples, seed=0)
- [ ] Implement `n_eff_panel(panel: &Panel, vault, forge, clock) -> Result<NeffEstimate, CalyxError>`: convenience wrapper that calls `agreement_graph` then `n_eff_from_agreement_graph`
- [ ] Wire updated `n_eff` into `AbundanceReport` (replace `Provisional(N as f32)` from PH27 T06 with `Computed { … }`)
- [ ] Wire `n_eff` into `LruXtermCache` capacity: `max(n_eff_value.ceil() as usize * N, MIN_CACHE_CAPACITY)` where `MIN_CACHE_CAPACITY = 256`

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: planted panel with N=5, 5 near-identical lenses (corr ≈ 0.9) → `n_eff ≈ 1.0 ± 0.3` (all redundant → rank ≈ 1)
- [ ] unit: planted panel with N=5, 5 orthogonal lenses (corr ≈ 0.0) → `n_eff ≈ 5.0 ± 0.5` (all independent → rank = N)
- [ ] unit: planted panel with 5 near-identical + 3 independent lenses (N=8) → `n_eff ≈ 3.0 ± 0.8` (known rank ≈ 3+1 = ~4 but stable rank ≈ 3 due to partial redundancy overlap)
- [ ] proptest: `1.0 ≤ n_eff ≤ N` always (stable rank is bounded by 1 and N)
- [ ] edge: N=1 → `n_eff = 1.0` exactly (trivially); N=0 → `n_eff = 0.0`, no panic; all pairs with agreement = 0.0 → `n_eff = N` (fully independent)

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `n_eff` for a planted panel with 5 near-identical + 3 independent lenses (N=8); expected n_eff in [2.5, 4.0]
- **Readback:**
  ```
  cargo test n_eff_planted_panel -- --nocapture
  ```
  Printed `NeffEstimate { value: f32, ci_low, ci_high }` must have `value ∈ [2.5, 4.0]`.
  Also:
  ```
  calyx abundance --vault /home/croyse/calyx/test-vault
  ```
  The `n_eff` line must now show `Computed { value: f32 }` not `[provisional]`.
- **Prove:** run on aiwonder; capture output; confirm the planted panel's n_eff is in the expected range. Confirm the `abundance_report` no longer shows `[provisional]` for n_eff after this card is merged.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH29 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
