# PH18 · T05 — Full frozen contract enforcement at register + measure

| Field | Value |
|---|---|
| **Phase** | PH18 — Frozen contract + content-addressed LensId |
| **Stage** | S3 — Registry / Lenses |
| **Crate** | `calyx-registry` |
| **Files** | `crates/calyx-registry/src/lib.rs` (≤500), `crates/calyx-registry/src/frozen.rs` (≤500) |
| **Depends on** | T02, T03, T04 (this phase) |
| **Axioms** | A4, A16 |
| **PRD** | `dbprdplans/05 §4` |

## Goal

Compose the four individual guards (weights hash, dim, finite+norm,
determinism probe) into a single `check_frozen_contract` function and wire it
into both `Registry::register` and `Registry::measure`. After this card,
no vector from any runtime can enter the vault without passing all four
invariants; every violation returns a structured `CALYX_*` error with
remediation.

## Build (checklist of concrete, code-level steps)

- [ ] `pub fn check_frozen_contract_at_register(spec: &LensSpec, lens: &dyn Lens, probe_input: &Input) -> Result<()>`:
  1. `check_weights_sha256(actual, spec)` — actual is the computed hash from
     the lens runtime's loaded weights (or spec hash for TEI-http trust-anchor).
  2. Run `determinism_probe(lens, probe_input)` (two measure calls → equal).
  3. On the probe result, call `check_output(v, spec)`.
  4. Return first error encountered, or `Ok(())`.
- [ ] `pub fn check_frozen_contract_at_measure(vec: &SlotVector, spec: &LensSpec) -> Result<()>`:
  calls `check_output(vec, spec)` (dim + finite + norm); no determinism probe
  per-call (too expensive; probe runs at registration only).
- [ ] Update `Registry::register(spec, lens)` to call
  `check_frozen_contract_at_register` with a canonical probe input
  (`Input::new(spec.modality, b"calyx-frozen-probe")` for text; each modality
  has a type-appropriate probe).
- [ ] Update `Registry::measure` and `Registry::measure_batch` to call
  `check_frozen_contract_at_measure` on every returned vector.
- [ ] If either check fails, propagate the error; **no partial results**.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] integration: register a valid `AlgorithmicLens` → `Ok(())`; confirm
  subsequent `measure` calls pass the contract.
- [ ] integration: register with wrong `weights_sha256` → `CALYX_LENS_FROZEN_VIOLATION`.
- [ ] integration: mock runtime returns wrong dim → `CALYX_LENS_DIM_MISMATCH`
  at `measure` time.
- [ ] integration: mock runtime returns NaN → `CALYX_LENS_NUMERICAL_INVARIANT`
  at `measure` time.
- [ ] integration: mock non-deterministic runtime fails at registration →
  `CALYX_LENS_NUMERICAL_INVARIANT` from determinism probe.
- [ ] edge (≥3): (1) `AlgorithmicLens` passes all four checks end-to-end;
  (2) `TeiHttpLens` (mocked) passes all four checks; (3) a lens that passes
  registration but later returns NaN for a real input → fails at measure time.
- [ ] fail-closed: no code path in `Registry` returns a vector after a
  contract failure; grep the source for any `unwrap` on `check_*` calls.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** end-to-end integration test output on aiwonder; Aster slot CF
  column never written on a failing measure
- **Readback:** `cargo test -p calyx-registry contract -- --nocapture 2>&1`
- **Prove:** output shows five distinct test cases: violation/mismatch/
  numerical/determinism/pass — each producing the correct code or `Ok`;
  screenshot attached to PH18 GitHub issue

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH18 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
