# PH39 · T01 — `SpeakerMatch` + `StyleHold` anchor kinds + `IdentityProfile`

| Field | Value |
|---|---|
| **Status** | DONE / FSV-signed-off for #269 |
| **Phase** | PH39 — Identity-Locked Generation (Speaker / Style) |
| **Stage** | S8 — Ward Gτ Guard |
| **Crate** | `calyx-ward` (`calyx-core` already exposed the anchor variants) |
| **Files** | `crates/calyx-ward/src/identity.rs` (≤500), `crates/calyx-ward/tests/identity_profile.rs` (≤500), `crates/calyx-ward/src/error.rs` |
| **Depends on** | PH38 T01 (`CalibrationMeta`) · PH04 (`AnchorKind`) |
| **Issue** | #269 |
| **Implementation commit** | `a336b90` |
| **Axioms** | A12 |
| **PRD** | `dbprdplans/09 §5b` |

## Result

`SpeakerMatch` and `StyleHold` were already first-class `AnchorKind` variants in
`calyx-core` and already had serde/slot-map key support. T01 adds
`calyx-ward::identity` with `IdentitySlotConfig` and `IdentityProfile`:

- `IdentitySlotConfig { slot_id, anchor_kind, tau_override }`
- `IdentityProfile { guard_profile, identity_slots, matched_slot_cache }`
- `IdentityProfile::new()` validates identity anchors, required-slot coverage,
  effective tau, matched-vector presence, matched-vector normalization, and
  duplicate identity-slot rejection.
- `IdentityProfile` JSON deserialization re-runs the same constructor
  validation, so persisted JSON cannot bypass construction invariants.
- `CALYX_GUARD_IDENTITY_SLOT_NOT_REQUIRED` is part of `WardError` and exported
  from the crate root.

## Build Checklist

- [x] Confirm `calyx-core::AnchorKind` already includes `SpeakerMatch` and
      `StyleHold` with stable snake-case serde and slot-map keys.
- [x] Define `IdentitySlotConfig` in `identity.rs`.
- [x] Define `IdentityProfile` with a cached `MatchedSlots` map.
- [x] Implement `IdentityProfile::new(...) -> Result<Self, WardError>`.
- [x] Reject identity slots absent from `guard_profile.required_slots` with
      `WardError::IdentitySlotNotRequired { slot }`.
- [x] Reject non-identity anchor kinds, duplicate identity slots, and profiles
      whose required-slot set does not exactly match the identity-slot set.
- [x] Reject missing matched vectors, zero/non-finite matched vectors, missing
      profile tau for identity slots, non-finite tau, and tau outside `[0, 1]`.
- [x] Normalize cached matched-slot vectors at construction time.
- [x] `IdentityProfile::is_calibrated(&self) -> bool` delegates to
      `self.guard_profile.is_calibrated()`.
- [x] Add and export `CALYX_GUARD_IDENTITY_SLOT_NOT_REQUIRED`.

## Tests

- [x] Unit: construct a two-slot speaker/style `IdentityProfile`; assert slot
      count, normalized cache, calibrated state, and serde round-trip.
- [x] Unit: `SpeakerMatch` and `StyleHold` serialize/deserialize as
      `speaker_match` and `style_hold`.
- [x] Unit: slot absent from `guard_profile.required_slots` fails closed with
      `CALYX_GUARD_IDENTITY_SLOT_NOT_REQUIRED`.
- [x] Unit: missing matched vector fails closed with `CALYX_GUARD_MISSING_SLOT`.
- [x] Unit: NaN tau, negative tau, tau above one, missing inherited tau, missing
      identity-slot coverage, non-identity anchor, and zero matched vector all
      fail closed.
- [x] Unit: deserializing invalid `IdentityProfile` JSON revalidates and fails.
- [x] Proptest: `IdentitySlotConfig` serde round-trip preserves the identity
      anchor kind and tau override.
- [x] Edge: empty identity-slot set constructs as an inert schema edge. T04
      `guard_generate()` must reject empty identity profiles at runtime before
      generation; this is tracked on #272.

## FSV

- **Source of truth:** `/home/croyse/calyx/data/fsv-issue269-identity-profile-20260609`
- **Log:** `/home/croyse/calyx/data/fsv-issue269-identity-profile-20260609.fsv.log`
- **Trigger:** `CALYX_WARD_IDENTITY_FSV_DIR=/home/croyse/calyx/data/fsv-issue269-identity-profile-20260609 cargo test -p calyx-ward --test identity_profile -- --ignored --nocapture issue269_identity_profile_fsv_writes_readbacks`
- **Readback artifacts:** `identity-profile.json`, `anchor-kinds.json`,
  `identity-errors.json`, `identity-summary.json`, and `SHA256SUMS.txt`.
- **Readback facts:** `identity-summary.json` reports `calibrated=true`,
  `identity_slot_count=2`, `matched_slot_count=2`,
  `required_identity_slots=[8,9]`, `speaker_tau=0.91`, `style_tau=0.76`, and
  `identity_slot_error_code=CALYX_GUARD_IDENTITY_SLOT_NOT_REQUIRED`.
- **Edge readback:** `identity-errors.json` contains fail-closed rows for slot
  not required, missing matched slot, NaN/out-of-range/inherited bad tau,
  missing profile tau, missing identity-slot coverage, non-identity anchor kind,
  and zero matched vector.

### Final Hashes

| Artifact | SHA-256 |
|---|---|
| `anchor-kinds.json` | `71c7a56cfdd7342403932087b37c1a38520836c7236e89a05839b1354ddfca75` |
| `identity-errors.json` | `012337a74bc7694586021ee224f22e8fc16645f414de152d3978118874e5ff28` |
| `identity-profile.json` | `2a6bdc2f056500668b9d6b11c90c4392e70989e9b046b3c69fe6fc69d62c2f7a` |
| `identity-summary.json` | `d209885020f7f226eab841c41523e08a5683055a7f9c5fe968228dc2f4eba739` |
| `SHA256SUMS.txt` | `be953d0c80fcb50c4e6927266ab293644ec9be2f05c22a80d30846bf116ed6af` |
| FSV log | `7e8d3022f4c87405a4bcf5ab00bc50e2859eb09204283c8aaa7ff6ed996fb112` |
| focused gate log | `67266da50336d1b9aea0d7eede3e1ed2193ee553a80455d3c2048e0aee21106e` |
| workspace gate log | `753e3c780df0e035ede6a70e4a6dc4e72faeff9f704076fe0da009d7bfc3ded0` |

## Done

- [x] `cargo fmt --check -p calyx-ward`
- [x] `cargo check -p calyx-ward`
- [x] `cargo test -p calyx-ward --test identity_profile -- --nocapture`
- [x] `cargo clippy -p calyx-ward --test identity_profile -- -D warnings`
- [x] `cargo fmt --check`
- [x] `cargo check --workspace`
- [x] `cargo test --workspace`
- [x] `cargo clippy --workspace --all-targets -- -D warnings`
- [x] `bash scripts/linecount.sh`
- [x] `git diff --check`
- [x] FSV evidence attached to #269.
- [x] No harness-only FSV: durable aiwonder JSON bytes and manifest were read
      after the trigger.
