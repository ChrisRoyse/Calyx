# PH27 · T01 — `CrossTermKind` types + `agreement_scalar` (eager, always)

| Field | Value |
|---|---|
| **Phase** | PH27 — Agreement graph + cross-terms (lazy) |
| **Stage** | S5 — Loom + Assay (DDA & Bits) |
| **Crate** | `calyx-loom` |
| **Files** | `crates/calyx-loom/src/cross_term.rs` (≤500), `crates/calyx-loom/src/lib.rs` (≤500) |
| **Depends on** | PH13 (Forge batched cosine, CPU/GPU), PH24 (active slot vectors) |
| **Axioms** | A8, A9, A13, A31 |
| **PRD** | `dbprdplans/06 §3`, `06 §4`, `06 §7` |

## Goal

Define the four cross-term kinds (`Agreement`, `Delta`, `Interaction`, `Concat`)
as a typed enum and implement the cheapest, always-eager one: the agreement
scalar `cos(v_a, v_b)`. This scalar is the foundation of the redundancy graph,
the blind-spot detector, and n_eff, so it must be correct, normalized, and
bit-parity tested on both CPU and GPU paths before any downstream work begins.

## Build (checklist of concrete, code-level steps)

- [ ] Define `CrossTermKind` enum: `Agreement`, `Delta`, `Interaction { low_rank: bool }`, `Concat`
- [ ] Define `CrossTerm` value type: `{ kind: CrossTermKind, slot_a: SlotId, slot_b: SlotId, value: CrossTermValue, provenance: CrossTermProvenance }` where `CrossTermValue` is `Scalar(f32)` | `Vec(Vec<f32>)` | `LowRank { u: Vec<f32>, v: Vec<f32> }`
- [ ] Define `CrossTermProvenance`: `{ cx_id: CxId, computed_at_seq: u64, source: Measured | Derived, estimator: AgreementCosine | Delta | Interaction | Concat }`
- [ ] Implement `agreement_scalar(v_a: &[f32], v_b: &[f32], forge: &ForgeHandle) -> Result<f32, CalyxError>`:
  - normalize both vectors via Forge (asserts non-zero); dispatch `batched_cosine` on GPU or CPU SIMD; return scalar
  - if either vector is zero-norm → `CALYX_LOOM_ZERO_NORM_VECTOR`
- [ ] Implement `agreement_batch(pairs: &[(SlotId, SlotId)], slot_vecs: &SlotVecStore, forge: &ForgeHandle) -> Result<Vec<(SlotId, SlotId, f32)>, CalyxError>` — batched form used by `weave`
- [ ] Tag every returned `CrossTerm` with `source: Derived` (a cross-term of two measured lenses is itself derived, not a new external measurement)
- [ ] Wire `CalyxError::LoomZeroNormVector` into the error catalog (`calyx-core/src/error.rs`)

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: two orthogonal unit vectors → agreement scalar = 0.0 ± 1e-6; two identical unit vectors → 1.0 ± 1e-6; two antipodal → -1.0 ± 1e-6
- [ ] proptest: `agreement_scalar(v, v) == 1.0` for any non-zero `v`; agreement is commutative: `agreement_scalar(a, b) == agreement_scalar(b, a)` for all non-zero `a`, `b`
- [ ] edge: zero-norm vector `a` → `CALYX_LOOM_ZERO_NORM_VECTOR`; single-element vectors → correct cosine; vectors of length 1536 (TEI output dim) → within 1e-4 of numpy reference
- [ ] fail-closed: `NaN`-containing vector → `CALYX_LOOM_ZERO_NORM_VECTOR` or `CALYX_FORGE_INVALID_INPUT` (not silent NaN propagation)

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** the agreement scalar returned by `agreement_batch` for a planted pair `(v_a, v_b)` where `v_a = [1,0,…]`, `v_b = [cos(θ), sin(θ), 0,…]` with θ=π/3
- **Readback:** run the unit test with `--nocapture` on aiwonder; the printed scalar must be `0.5 ± 1e-4`; confirm GPU path used by checking Forge dispatch log
- **Prove:** CPU path and GPU path both return the same value within ≤1e-3 (bit-parity check). Run `cargo test agreement_scalar_parity -- --nocapture` on aiwonder; confirm both paths printed.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] CPU↔GPU bit-parity ≤ 1e-3 on the agreement scalar golden set
- [ ] FSV evidence (readback output / screenshot) attached to the PH27 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
