# PH37 · T05 — No-flatten enforcement + average-passing / slot-failing rejection

| Field | Value |
|---|---|
| **Phase** | PH37 — Gτ Guard Math + GuardProfile |
| **Stage** | S8 — Ward Gτ Guard |
| **Crate** | `calyx-ward` |
| **Files** | `crates/calyx-ward/src/guard.rs` (≤500), `crates/calyx-ward/tests/guard_unit.rs` (≤500) |
| **Depends on** | T04 (this phase) |
| **Axioms** | A3, A12 |
| **PRD** | `dbprdplans/09 §2`, `09 §4` |

## Goal

Prove — structurally and by test — that no flattened-vector path exists: an
output that passes a cosine average across all slots but fails at least one
required slot is unconditionally rejected. This is the central anti-injection
property from `09 §2`: "an attack that fools the average can't fool every axis
at once."

## Build (checklist of concrete, code-level steps)

- [ ] Add `#[deny(flatten)]` lint comment block in `guard.rs` documenting that
      slot vectors must never be concatenated; add `// INVARIANT: no flatten —
      A3` comment at the top of `guard()`
- [ ] Implement `average_cosine_would_pass(profile: &GuardProfile,
      per_slot: &[SlotVerdict]) -> bool`: computes the mean of all per-slot cos
      values; returns `true` if mean ≥ mean of all τ values. This function is
      used **only in tests** to demonstrate the attack scenario — never as a gate
- [ ] In test file `tests/guard_unit.rs`, construct the canonical
      average-passing/slot-failing scenario:
      - Two required slots, τ = `[0.7, 0.7]` (both)
      - Cos scores = `[0.95, 0.45]` → average = 0.70 (≥ τ_avg = 0.70), but
        slot-2 fails (0.45 < 0.70)
      - Call `guard()` → assert `overall_pass == false`
      - Call `average_cosine_would_pass()` on the same verdict → assert `true`
      - This demonstrates the attack scenario is blocked
- [ ] Add a source-level test `grep_no_flatten` that reads `guard.rs` bytes and
      asserts the string `"flatten"` does not appear in any non-comment line
      (static source check, not a runtime check)

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: the canonical average-passing/slot-failing case above — `guard()`
      returns `overall_pass == false` while `average_cosine_would_pass()` returns
      `true`; the `per_slot` vec shows cos=0.95/pass=true + cos=0.45/pass=false
- [ ] unit: three slots where average cos=0.71 > mean-τ=0.70 but 2 of 3 slots
      fail individually; under `AllRequired` → overall fail; under `KofN{k:1}`
      → overall pass (1 slot passed)
- [ ] proptest: for any slot-vector set where at least one slot cos < its τ,
      `AllRequired` guard always returns `overall_pass == false` regardless of
      the average
- [ ] edge: identical produced and matched vectors on all slots → all cos=1.0 →
      overall pass regardless of τ (upper-bound sanity)
- [ ] edge: exactly one slot at cos=0.0 with τ=0.7 → fail; average of remaining
      high-cos slots irrelevant
- [ ] fail-closed: if somehow `per_slot` is empty under `AllRequired` and a
      code path tried to compute average — assert no panic (empty average handled
      as pass, not division-by-zero)

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** test output for the `guard_no_flatten` test in `tests/guard_unit.rs`
- **Readback:**
  `cargo test -p calyx-ward no_flatten -- --nocapture 2>&1` and
  `grep -n flatten crates/calyx-ward/src/guard.rs`
- **Prove:** test output shows `overall_pass: false` + `average_would_pass: true`
  in the canonical attack scenario; `grep` returns zero lines (no flatten in
  source); the guard.rs line count is printed and ≤ 500

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH37 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
