# PH37 · T03 — `guard()` per-slot cosine gate — `AllRequired` policy

| Field | Value |
|---|---|
| **Phase** | PH37 — Gτ Guard Math + GuardProfile |
| **Stage** | S8 — Ward Gτ Guard |
| **Crate** | `calyx-ward` |
| **Files** | `crates/calyx-ward/src/guard.rs` (≤500) |
| **Depends on** | T02 (this phase) · PH13 (Forge cosine) |
| **Axioms** | A3, A12 |
| **PRD** | `dbprdplans/09 §1`, `09 §2`, `09 §4` |

## Goal

Implement the core `guard()` function: iterate over each required slot in the
`GuardProfile`, compute `cos(produced_k, matched_k)` via Forge, compare to
`τ_k`, and assemble a `GuardVerdict` with full per-slot breakdown. Under
`AllRequired` policy every required slot must pass; any single failure yields
`overall_pass = false` with `CALYX_GUARD_OOD`. This is the exact mechanism from
`09 §1`: `s = cos(produced_slot_vec, matched_cx.slot_k)`.

## Build (checklist of concrete, code-level steps)

- [ ] Define `ProducedSlots` type alias: `BTreeMap<SlotId, Vec<f32>>` (the
      model-produced per-slot vectors; caller provides pre-normalized or raw;
      guard normalizes before cos)
- [ ] Define `MatchedSlots` type alias: `BTreeMap<SlotId, Vec<f32>>` (the
      grounded constellation's per-slot vectors from the vault)
- [ ] Implement `guard(profile: &GuardProfile, produced: &ProducedSlots,
      matched: &MatchedSlots) -> Result<GuardVerdict, WardError>`:
      - For each slot in `profile.required_slots` (in BTreeMap order for
        determinism):
        - Look up `produced.get(slot)` → `WardError::MissingSlot` if absent
        - Look up `matched.get(slot)` → `WardError::MissingSlot` if absent
        - Normalize both vectors to unit length (inline, not via flatten)
        - `cos_val = forge::cosine_f32(produced_vec, matched_vec)`
        - `tau_val = profile.tau_for(slot).unwrap_or(0.7_f32)` (cold-start
          prior; calibrated governs per `09 §3`)
        - `pass = cos_val >= tau_val`
        - Push `SlotVerdict { slot, cos: cos_val, tau: tau_val, pass }`
      - Under `AllRequired`: `overall_pass = per_slot.iter().all(|v| v.pass)`
      - Set `action` to `Some(profile.novelty_action.clone())` when
        `!overall_pass`; `None` when pass
      - Return `Ok(GuardVerdict { overall_pass, per_slot, action,
        guard_id: profile.guard_id })`
      - If `overall_pass == false`, additionally return
        `Err(WardError::Ood { guard_id, failing })` — but the verdict is
        embedded in the `Ood` variant so callers get full decomposition on error
- [ ] **No flatten path:** the function must never concatenate slot vectors into
      a single vector; each slot is evaluated independently
- [ ] Add `/// CALYX_GUARD_OOD` doc comment on the error return path

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: two required slots, both cos ≥ τ; assert `overall_pass == true` and
      `per_slot.len() == 2` and both `SlotVerdict.pass == true`
- [ ] unit: two required slots, slot-1 cos=0.90 τ=0.70 (pass), slot-2 cos=0.55
      τ=0.70 (fail); assert `overall_pass == false`; assert
      `verdict.failing_slots().len() == 1`; assert the failing slot has
      `cos=0.55` and `tau=0.70`
- [ ] unit: single required slot exactly at boundary `cos == τ`; assert pass
      (`≥` not `>`)
- [ ] proptest: for any two unit-norm vectors and τ in `[0.0, 1.0]`, the verdict
      `pass` matches `cosine(a,b) >= τ`
- [ ] edge: `required_slots` is empty → `overall_pass = true`, `per_slot` empty
- [ ] edge: produced vector for a required slot is the zero vector → normalize
      returns error or zero-vec; test that `CALYX_GUARD_OOD` is still returned
      (not a panic)
- [ ] fail-closed: missing slot in `produced` → `WardError::MissingSlot` (not
      a default cos=0.0)

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `GuardVerdict` returned by `guard()` in the unit test
- **Readback:** `cargo test -p calyx-ward guard_allrequired -- --nocapture 2>&1`
  — print the full `GuardVerdict` via `{:?}` in the test; inspect per-slot
  `(cos, tau, pass)` values
- **Prove:** test output shows the two-slot verdict with `overall_pass: false`
  when slot-2 cos=0.55 < τ=0.70; the `failing_slots` vec shows slot-2 only;
  grep output confirms no concatenation of slot vecs in `guard.rs`

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] CPU↔GPU bit-parity ≤ 1e-3 on the golden cosine set (Forge-touching)
- [ ] FSV evidence (readback output / screenshot) attached to the PH37 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
