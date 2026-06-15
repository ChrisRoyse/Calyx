# PH37 · T01 — `GuardProfile` struct + `GuardPolicy` + `NoveltyAction` + `CalibrationMeta`

| Field | Value |
|---|---|
| **Phase** | PH37 — Gτ Guard Math + GuardProfile |
| **Stage** | S8 — Ward Gτ Guard |
| **Crate** | `calyx-ward` |
| **Files** | `crates/calyx-ward/src/profile.rs` (≤500), `crates/calyx-ward/src/lib.rs` (≤500) |
| **Depends on** | — (first card; PH22 `SlotId` type required) |
| **Axioms** | A3, A12 |
| **PRD** | `dbprdplans/09 §4` |

**STATUS:** DONE / FSV-signed-off in #258. The unchecked checklist rows below
are historical build prompts; the authoritative evidence is the #258 closeout
and `/home/croyse/calyx/data/fsv-issue258-ph37-t01-20260609-tsus`.

## Goal

Define the canonical `GuardProfile` struct exactly as specified in `09 §4`,
along with its enums `GuardPolicy`, `NoveltyAction`, and the nested
`CalibrationMeta` struct. This is the configuration object that every guard
call reads; getting it right first prevents downstream type churn.

## Post-implementation note

Implemented in `crates/calyx-ward/src/profile.rs` and re-exported from
`crates/calyx-ward/src/lib.rs`. aiwonder FSV root:
`/home/croyse/calyx/data/fsv-issue258-ph37-t01-20260609-tsus`. Readback artifacts:
`happy.json`, `edge-kofn0.json`, `edge-calibrated.json`, and
`edge-empty-required.json`; each round-trips through `serde_json` and was read
back separately with `xxd`, `sha256sum`, grep, and parsed JSON output.

Post-sweep hardening #650 keeps those shapes representable for compatibility
but rejects them at runtime: empty `required_slots` and `KofN { k: 0 }` now
return `CALYX_GUARD_INERT_PROFILE` before any trusted guard surface can pass.

## Build (checklist of concrete, code-level steps)

- [ ] Define `GuardPolicy` enum: `AllRequired` | `KofN { k: usize }` — serde,
      `Clone`, `Debug`, `PartialEq`
- [ ] Define `NoveltyAction` enum: `NewRegion` | `Quarantine` | `RejectClosed`
      — serde, `Clone`, `Debug`, `PartialEq`
- [ ] Define `CalibrationMeta` struct:
      `corpus_hash: [u8; 32]`, `estimator: String`, `far: f32`, `frr: f32`,
      `confidence: f32`, `ts: i64` (Unix µs, via `Clock` trait — never
      `SystemTime::now()`) — serde, `Clone`, `Debug`
- [ ] Define `GuardProfile` struct:
      `guard_id: GuardId`, `panel_version: u64`, `domain: String`,
      `tau: BTreeMap<SlotId, f32>`, `required_slots: Vec<SlotId>`,
      `policy: GuardPolicy`, `calibration: Option<CalibrationMeta>`,
      `novelty_action: NoveltyAction` — serde, `Clone`, `Debug`
- [ ] Add `GuardProfile::is_calibrated(&self) -> bool` returning
      `self.calibration.is_some()`
- [ ] Add `GuardProfile::tau_for(&self, slot: &SlotId) -> Option<f32>` (BTreeMap
      lookup; `None` means slot not guarded)
- [ ] Wire `profile.rs` into `src/lib.rs`; `pub use profile::*`
- [ ] `GuardId` newtype wrapping `uuid::Uuid`; implement `Display` and `FromStr`

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: construct a `GuardProfile` with two slots (`s1`, `s2`), tau
      `{s1: 0.80, s2: 0.65}`, policy `AllRequired`, action `NewRegion`;
      assert `tau_for(&s1) == Some(0.80)` and `tau_for(&s3) == None`
- [ ] proptest: `GuardProfile` serde round-trip: `serde_json::from_str(
      serde_json::to_string(&p).unwrap()).unwrap() == p`
- [ ] edge: `KofN { k: 0 }` is representable and round-trips cleanly; runtime
      guard calls reject it as `CALYX_GUARD_INERT_PROFILE` after #650
- [ ] edge: `CalibrationMeta` with `far=0.0, frr=1.0` round-trips and
      `is_calibrated()` returns `true`
- [ ] edge: `GuardProfile` with empty `required_slots` serializes/deserializes
      without panic; runtime guard calls reject it as
      `CALYX_GUARD_INERT_PROFILE` after #650
- [ ] fail-closed: `tau_for` on a slot absent from the map returns `None` (not
      0.0 or a default — caller must handle absence explicitly)

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** durable aiwonder evidence root
  `/home/croyse/calyx/data/fsv-issue258-ph37-t01-<date>/` containing
  `GuardProfile` JSON readback artifacts and a SHA-256 manifest.
- **Readback:** write the fixture JSON through Ward serde, then separately read
  those files with `xxd`, `sha256sum`, grep, and parsed JSON.
- **Prove:** durable JSON contains `"tau"`, `"required_slots"`, `"policy"`,
  `"calibration"`, `"novelty_action"`, and UUID `"guard_id"` bytes; parsed
  readback confirms `tau_for` values and no field is silently dropped.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH37 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
