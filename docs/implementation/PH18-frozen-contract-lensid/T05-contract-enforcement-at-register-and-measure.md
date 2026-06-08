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
determinism probe) into the frozen registration path and wire validation into
`Registry::measure`. After this card,
no vector from any runtime can enter the vault without passing all four
invariants; every violation returns a structured `CALYX_*` error with
remediation.

## Build (checklist of concrete, code-level steps)

- [x] Registration enforces `FrozenLensContract::verify_registration`; optional
  `register_frozen_with_probe` also runs `verify_determinism_probe`:
  1. Verify weights hash/content-addressed contract against the runtime lens.
  2. Run the determinism probe when a probe input is supplied.
  3. On the probe result, verify the output vector.
  4. Return first error encountered, or `Ok(())`.
- [x] Measurement validation is wired through `Registry::validate_entry`, which
  calls `FrozenLensContract::verify_vector` on every returned vector (dim +
  finite + norm); determinism remains a registration probe.
- [x] Keep `Registry::register` and `Registry::register_with_spec` as
  fail-closed compatibility stubs: both return `CALYX_LENS_FROZEN_VIOLATION`
  and do not insert.
- [x] Use `register_frozen`, `register_frozen_with_spec`, or
  `register_frozen_with_probe` for successful insertion. These paths verify
  contract id/shape/modality, optional determinism probe, and then store the
  `FrozenLensContract` beside the runtime lens.
- [x] `Registry::measure` and `Registry::measure_batch` validate every returned
  vector before returning it.
- [x] If either check fails, the error is propagated; **no partial results**.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] integration: register a valid `AlgorithmicLens` → `Ok(())`; confirm
  subsequent `measure` calls pass the contract.
- [x] integration: plain `register` → `CALYX_LENS_FROZEN_VIOLATION` and
  `Registry::contains(id) == false`.
- [x] integration: register with wrong `weights_sha256` → `CALYX_LENS_FROZEN_VIOLATION`.
- [x] integration: mock runtime returns wrong dim → `CALYX_LENS_DIM_MISMATCH`
  at `measure` time.
- [x] integration: mock runtime returns NaN → `CALYX_LENS_NUMERICAL_INVARIANT`
  at `measure` time.
- [x] integration: mock non-deterministic runtime fails at registration →
  `CALYX_LENS_NUMERICAL_INVARIANT` from determinism probe.
- [x] edge (≥3): (1) `AlgorithmicLens` passes all four checks end-to-end;
  (2) `TeiHttpLens` (mocked) passes all four checks; (3) a lens that passes
  registration but later returns NaN for a real input → fails at measure time.
- [x] fail-closed: no code path in `Registry` returns a vector after a
  contract failure; grep the source for any `unwrap` on `check_*` calls.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** end-to-end integration test output on aiwonder; Aster slot CF
  column never written on a failing measure
- **Readback:** `CALYX_FSV_ROOT=/home/croyse/calyx/data/fsv-issue310-registry-frozen-contract-20260608 cargo test -p calyx-registry --test stage3_atomic_fsv -- --ignored --nocapture`
- **Prove:** read
  `/home/croyse/calyx/data/fsv-issue310-registry-frozen-contract-20260608/stage3-atomic-readback.json`;
  it must contain `plain_register_error=CALYX_LENS_FROZEN_VIOLATION`,
  `plain_register_inserted=false`, and successful `register_frozen*`
  runtime/profile readbacks.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence attached in #310 with readback root
      `/home/croyse/calyx/data/fsv-issue310-registry-frozen-contract-20260608`
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
