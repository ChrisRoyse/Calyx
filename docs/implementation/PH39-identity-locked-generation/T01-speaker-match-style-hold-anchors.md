# PH39 · T01 — `SpeakerMatch` + `StyleHold` anchor kinds + `IdentityProfile`

| Field | Value |
|---|---|
| **Phase** | PH39 — Identity-Locked Generation (Speaker / Style) |
| **Stage** | S8 — Ward Gτ Guard |
| **Crate** | `calyx-ward` (+ schema change in `calyx-core`) |
| **Files** | `crates/calyx-ward/src/identity.rs` (≤500), `crates/calyx-core/src/enums.rs` (≤500) |
| **Depends on** | PH38 T01 (`CalibrationMeta`) · PH04 (`AnchorKind`) |
| **Axioms** | A12 |
| **PRD** | `dbprdplans/09 §5b` |

## Goal

Introduce `SpeakerMatch` and `StyleHold` as first-class `AnchorKind` variants
in `calyx-core`, and define `IdentityProfile` in `calyx-ward` — the struct that
wraps a `GuardProfile` restricted to identity slots, caches the matched-slot
vectors at construction time, and exposes the identity-slot required-set.
This is the schema foundation for `guard_generate()` in T04.

## Build (checklist of concrete, code-level steps)

- [ ] Add to `calyx-core/src/enums.rs` `AnchorKind` enum:
      `SpeakerMatch` — speaker-verification anchor; binds a voice constellation
      `StyleHold` — persona/style anchor; binds a writing style constellation
      (keep all existing variants; add these two)
- [ ] Define `IdentitySlotConfig` struct in `identity.rs`:
      `slot_id: SlotId`, `anchor_kind: AnchorKind`, `tau_override: Option<f32>`
      (None → use calibrated τ from profile)
- [ ] Define `IdentityProfile` struct:
      `guard_profile: GuardProfile`,
      `identity_slots: Vec<IdentitySlotConfig>`,
      `matched_slot_cache: BTreeMap<SlotId, Vec<f32>>` (pre-embedded at construction)
- [ ] Implement `IdentityProfile::new(guard_profile: GuardProfile,
      identity_slots: Vec<IdentitySlotConfig>,
      matched_vecs: BTreeMap<SlotId, Vec<f32>>) -> Result<Self, WardError>`:
      - Verify every `identity_slot.slot_id` is in `guard_profile.required_slots`
        → else `WardError::IdentitySlotNotRequired { slot }`
      - Verify every slot in `guard_profile.required_slots` that is an identity
        anchor kind has a vec in `matched_vecs` → else
        `WardError::MissingSlot { slot }`
      - Store pre-normalized matched vecs in `matched_slot_cache`
- [ ] `IdentityProfile::is_calibrated(&self) -> bool` delegates to
      `self.guard_profile.is_calibrated()`
- [ ] Add `CALYX_GUARD_IDENTITY_SLOT_NOT_REQUIRED` to `WardError` display

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: construct `IdentityProfile` with two identity slots (`speaker`,
      `style`), both in `required_slots`; assert construction succeeds;
      assert `matched_slot_cache.len() == 2`
- [ ] unit: `SpeakerMatch` and `StyleHold` variants serialize/deserialize via
      `serde_json` round-trip; assert equality
- [ ] unit: attempt to create `IdentityProfile` with a slot absent from
      `guard_profile.required_slots` → `WardError::IdentitySlotNotRequired`
- [ ] unit: attempt with missing matched vec for a required identity slot →
      `WardError::MissingSlot`
- [ ] proptest: `IdentitySlotConfig` serde round-trip; `anchor_kind` preserved
- [ ] edge: `identity_slots` empty → construction succeeds; `matched_slot_cache`
      empty
- [ ] fail-closed: `tau_override: Some(f32::NAN)` → `WardError` returned (NaN
      τ must be rejected at construction time, not silently used)

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** durable aiwonder evidence root containing `IdentityProfile` JSON,
  `AnchorKind` serde JSON, identity-slot error JSON/log, and a SHA-256
  manifest.
- **Readback:** run the manual FSV fixture with `CALYX_WARD_IDENTITY_FSV_DIR=$root`,
  then separately inspect the JSON/log artifacts with `xxd`, `sha256sum`, grep,
  and parsed JSON.
- **Prove:** durable readback contains `SpeakerMatch`, `StyleHold`,
  `IdentityProfile`, the expected slot counts, and
  `CALYX_GUARD_IDENTITY_SLOT_NOT_REQUIRED`.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH39 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
