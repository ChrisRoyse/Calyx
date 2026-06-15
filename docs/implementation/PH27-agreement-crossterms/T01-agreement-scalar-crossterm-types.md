# PH27 · T01 — `CrossTermKind` types + `agreement_scalar` (eager, always)

| Field | Value |
|---|---|
| **Phase** | PH27 — Agreement graph + cross-terms (lazy) |
| **Stage** | S5 — Loom + Assay (DDA & Bits) |
| **Crate** | `calyx-loom` |
| **Files** | `crates/calyx-loom/src/cross_term.rs` (≤500), `crates/calyx-loom/src/lib.rs` (≤500) |
| **Depends on** | PH13 (Forge CUDA for the optional `calyx-loom/cuda` path), PH24 (active slot vectors) |
| **Axioms** | A8, A9, A13, A31 |
| **PRD** | `dbprdplans/06 §3`, `06 §4`, `06 §7` |

## Goal

Define the four cross-term kinds (`Agreement`, `Delta`, `Interaction`, `Concat`)
as a typed enum and implement the cheapest, always-eager one: the agreement
scalar `cos(v_a, v_b)`. This scalar is the foundation of the redundancy graph,
the blind-spot detector, and n_eff, so it must be correct and normalized. The
default build uses the CPU path; `agreement_batch_gpu` must fail closed unless
the explicit `calyx-loom/cuda` feature is enabled and then dispatches through
Forge CUDA.

## Build (checklist of concrete, code-level steps)

- [x] Define `CrossTermKind` enum: `Agreement`, `Delta`, `Interaction { low_rank: bool }`, `Concat`
- [x] Define `CrossTerm` value type: `{ kind: CrossTermKind, slot_a: SlotId, slot_b: SlotId, value: CrossTermValue, provenance: CrossTermProvenance }` where `CrossTermValue` is `Scalar(f32)` | `Vec(Vec<f32>)` | `LowRank { u: Vec<f32>, v: Vec<f32> }`
- [x] Define `CrossTermProvenance`: `{ cx_id: CxId, computed_at_seq: u64, source: Measured | Derived, estimator: AgreementCosine | Delta | Interaction | Concat }`
- [x] Implement `agreement_scalar(v_a: &[f32], v_b: &[f32]) -> Result<f32, CalyxError>`:
  - normalize both vectors logically (asserts non-zero/non-finite); return scalar on the default CPU path
  - if either vector is zero-norm → `CALYX_LOOM_ZERO_NORM_VECTOR`
- [x] Implement `agreement_batch_cpu(pairs: &[(&[f32], &[f32])]) -> Result<Vec<f32>, CalyxError>` — batched form used by `weave`
- [x] Implement `agreement_batch_gpu(pairs: &[(&[f32], &[f32])]) -> Result<Vec<f32>, CalyxError>` — default build returns `CALYX_LOOM_FORGE_UNAVAILABLE`; `calyx-loom/cuda` uses Forge CUDA
- [x] Tag every returned `CrossTerm` with `source: Derived` (a cross-term of two measured lenses is itself derived, not a new external measurement)
- [x] Wire `CalyxError::LoomZeroNormVector` into the error catalog (`calyx-core/src/error.rs`)

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: two orthogonal unit vectors → agreement scalar = 0.0 ± 1e-6; two identical unit vectors → 1.0 ± 1e-6; two antipodal → -1.0 ± 1e-6
- [x] proptest: `agreement_scalar(v, v) == 1.0` for any non-zero `v`; agreement is commutative: `agreement_scalar(a, b) == agreement_scalar(b, a)` for all non-zero `a`, `b`
- [x] edge: zero-norm vector `a` → `CALYX_LOOM_ZERO_NORM_VECTOR`; single-element vectors → correct cosine; vectors of length 1536 (TEI output dim) → within 1e-4 of numpy reference
- [x] fail-closed: `NaN`-containing vector → `CALYX_LOOM_ZERO_NORM_VECTOR` or `CALYX_FORGE_INVALID_INPUT` (not silent NaN propagation)

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** the agreement scalar returned by `agreement_batch_cpu` for a planted pair `(v_a, v_b)` where `v_a = [1,0,…]`, `v_b = [cos(θ), sin(θ), 0,…]` with θ=π/3
- **Readback:** run the unit test with `--nocapture` on aiwonder; the printed scalar must be `0.5 ± 1e-4`; default `agreement_batch_gpu` readback must show `CALYX_LOOM_FORGE_UNAVAILABLE`
- **Prove:** when `calyx-loom/cuda` is enabled on aiwonder, the CUDA path executes through Forge and returns the same golden values within the accepted tolerance.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] default GPU entrypoint fails closed with `CALYX_LOOM_FORGE_UNAVAILABLE`
- [x] `calyx-loom/cuda` executes the Forge CUDA path on the agreement scalar golden set
- [x] FSV evidence (readback output / screenshot) attached to the PH27 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
