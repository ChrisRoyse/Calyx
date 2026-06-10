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

**STATUS:** DONE / FSV-signed-off in #260. The unchecked checklist rows below
are historical build prompts; the authoritative evidence is the #260 closeout
and `/home/croyse/calyx/data/fsv-issue260-ph37-t03-20260609-20a2a34`.

## Goal

Implement the core `guard()` function: iterate over each required slot in the
`GuardProfile`, compute `cos(produced_k, matched_k)` via Forge, compare to
`τ_k`, and assemble a `GuardVerdict` with full per-slot breakdown. Under
`AllRequired` policy every required slot must pass; any single failure yields
`overall_pass = false` with `CALYX_GUARD_OOD`. This is the exact mechanism from
`09 §1`: `s = cos(produced_slot_vec, matched_cx.slot_k)`.

## Post-implementation note

Implemented in `crates/calyx-ward/src/guard.rs` and re-exported from
`crates/calyx-ward/src/lib.rs` in commit
`20a2a3486a142e328b7ef449737459d7ba1a9b30`. Ward now exposes
`ProducedSlots`, `MatchedSlots`, `DEFAULT_TAU`, and `guard()`. The
implementation evaluates sorted/deduplicated required slots independently, uses
the actual `calyx-forge` backend cosine API, applies `DEFAULT_TAU = 0.7` when a
slot has no calibrated tau, returns `WardError::MissingSlot` for absent required
inputs, and returns a full `GuardVerdict` with `overall_pass=false` for invalid
vectors or below-tau slots.

aiwonder FSV root:
`/home/croyse/calyx/data/fsv-issue260-ph37-t03-20260609-20a2a34`. Readback
artifacts include `allrequired-fail-verdict.json`,
`allrequired-pass-verdict.json`, `edge-empty-required-verdict.json`,
`edge-zero-vector-verdict.json`, `missing-slot-error.json`, and
`guard-fsv.log`. Separate readback used `xxd`, `sha256sum`, parsed JSON, and
source inspection of `crates/calyx-ward/src/guard.rs`.

Post-sweep hardening #650 replaces the empty-required runtime outcome with
`edge-empty-required-error.json`: empty required-slot profiles now fail closed
with `CALYX_GUARD_INERT_PROFILE` instead of producing a vacuous pass.

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
        - Normalize both vectors to unit length per slot; never concatenate
          required-slot vectors
        - `cos_val` is computed with the actual Forge backend API
          (`calyx_forge::Backend::cosine`, e.g. `CpuBackend`, or a local Ward
          wrapper over that API). There is no direct Ward-specific cosine
          helper to call.
        - `tau_val = profile.tau_for(slot).unwrap_or(0.7_f32)` (cold-start
          prior; calibrated governs per `09 §3`)
        - `pass = cos_val >= tau_val`
        - Push `SlotVerdict { slot, cos: cos_val, tau: tau_val, pass }`
      - Under `AllRequired`: `overall_pass = per_slot.iter().all(|v| v.pass)`
      - Set `action` to `Some(profile.novelty_action.clone())` when
        `!overall_pass`; `None` when pass
      - Return `Ok(GuardVerdict { overall_pass, per_slot, action,
        guard_id: profile.guard_id })`
      - For T03, slot failures return `Ok(GuardVerdict { overall_pass: false,
        ... })` so callers and FSV can inspect the complete per-slot
        decomposition. T04 adds the fail-closed `WardError::Ood` ergonomic
        wrapper path using the failing slot details defined in T02.
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
- [ ] edge: `required_slots` is empty →
      `WardError::InertProfile` / `CALYX_GUARD_INERT_PROFILE`
- [ ] edge: produced vector for a required slot is the zero vector -> normalize
      cannot produce a valid unit vector; test that `guard()` returns a
      fail verdict with the configured novelty action (not a panic). T04 wraps
      this fail verdict into `CALYX_GUARD_OOD` for callers that use `?`.
- [ ] fail-closed: missing slot in `produced` → `WardError::MissingSlot` (not
      a default cos=0.0)

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** durable aiwonder evidence root, e.g.
  `/home/croyse/calyx/data/fsv-issue260-ph37-t03-YYYYMMDD`, containing the
  `GuardVerdict` JSON bytes and source readback artifacts.
- **Readback:** run the ignored/manual FSV fixture with
  `CALYX_WARD_GUARD_FSV_DIR=$root`, then separately inspect the written files
  with `xxd`, `sha256sum`, JSON parsing, and a source readback of
  `crates/calyx-ward/src/guard.rs`; #650 additionally reads back
  `CALYX_GUARD_INERT_PROFILE` for empty required-slot profiles.
- **Prove:** durable JSON shows the two-slot verdict with
  `overall_pass=false` when slot-2 cos=0.55 < tau=0.70; the failing-slots
  readback identifies slot 2 only; source readback confirms no concatenated
  slot-vector path in `guard.rs`.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] CPU↔GPU bit-parity ≤ 1e-3 on the golden cosine set (Forge-touching)
- [ ] FSV evidence (readback output / screenshot) attached to the PH37 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
