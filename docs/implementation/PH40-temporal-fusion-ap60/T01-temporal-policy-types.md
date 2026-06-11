# PH40 · T01 — TemporalPolicy + FusionWeights types

| Field | Value |
|---|---|
| **Phase** | PH40 — Temporal Fusion + AP-60 Post-Retrieval Boost |
| **Stage** | S9 — Temporal & Dedup |
| **Crate** | `calyx-core`, `calyx-sextant`, `calyx-aster` |
| **Files** | `crates/calyx-core/src/temporal.rs`; `crates/calyx-sextant/src/temporal/mod.rs`; `crates/calyx-aster/src/manifest/mod.rs`; `crates/calyx-aster/src/vault/durable.rs`; `crates/calyx-aster/tests/temporal_manifest_fsv.rs` (all ≤500) |
| **Depends on** | PH22 (E2/E3/E4 lens types) · PH24 (Hit type) |
| **Axioms** | A27 |
| **PRD** | `dbprdplans/25 §3`, `dbprdplans/25 §6` |

## Goal

Define all types that govern temporal search behavior: `TemporalPolicy`,
`FusionWeights`, `DecayFunction`, `PeriodicOptions`, `SequenceOptions`, and
`BoostConfig`. These types are set at vault creation and govern every downstream
boost operation. The AP-60 invariant — `never_dominant: true` — is encoded as a
field whose violation returns `CALYX_TEMPORAL_AP60_VIOLATION`.

## Build (checklist of concrete, code-level steps)

- [x] Define `DecayFunction` enum: `Linear { max_age_secs: u64 }`, `Exponential { half_life_secs: u64 }`, `Step` (buckets: <1h → 0.8, <1d → 0.5, ≥1d → 0.1)
- [x] Define `PeriodicOptions { target_hour: Option<u8>, target_day_of_week: Option<u8>, use_now: bool }`; validate `target_hour ∈ 0..=23`, `target_day_of_week ∈ 0..=6` → `CALYX_TEMPORAL_INVALID_PERIOD` on violation
- [x] Define `SequenceOptions { direction: SequenceDirection, multi_anchor_mode: MultiAnchorMode }`; `SequenceDirection` = `Forward | Backward`; `MultiAnchorMode` = `First | Last | All`
- [x] Define `FusionWeights { recency: f32, sequence: f32, periodic: f32 }` with a constructor that asserts `(recency + sequence + periodic - 1.0).abs() < 1e-6` → `CALYX_TEMPORAL_WEIGHT_SUM` on violation; default = `{ 0.50, 0.35, 0.15 }`
- [x] Define `BoostConfig { post_retrieval_alpha: f32, causal_high_mult: f32, causal_low_mult: f32 }` with defaults `0.10` / `1.10` / `0.85`; `post_retrieval_alpha` is capped at `0.10` for AP-60
- [x] Define `TemporalPolicy { enabled: bool, decay: DecayFunction, periodic: PeriodicOptions, sequence: SequenceOptions, fusion_weights: FusionWeights, boost: BoostConfig, never_dominant: bool }` — `never_dominant` defaults `true`; setting it `false` returns `CALYX_TEMPORAL_AP60_VIOLATION` at construction
- [x] Implement `Default` for `TemporalPolicy` (enabled, Linear/Exponential half_life=3600, match_hour+dow, forward/first, 0.50/0.35/0.15, 1.10/0.85, never_dominant=true)
- [x] `serde::{Serialize, Deserialize}` + `Clone` + `Debug` on all types
- [x] Re-export from `crates/calyx-sextant/src/temporal/mod.rs`
- [x] Store default policy in Aster durable vault manifests through `VaultOptions`

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: `FusionWeights::default()` → sums to exactly 1.0 within 1e-6
- [x] unit: `FusionWeights::new(0.4, 0.4, 0.2)` → valid; `new(0.4, 0.4, 0.3)` → `CALYX_TEMPORAL_WEIGHT_SUM`
- [x] unit: `TemporalPolicy::default()` round-trips through `serde_json` byte-exact
- [x] unit: `PeriodicOptions { target_hour: Some(24), .. }` → `CALYX_TEMPORAL_INVALID_PERIOD`
- [x] edge: `never_dominant = false` attempted → `CALYX_TEMPORAL_AP60_VIOLATION`
- [x] edge: `BoostConfig { causal_high_mult: 0.0, .. }` now fails closed;
  #656 enforces the T04 causal semantics that high-confidence is a strict boost
  (`>1.0`) and low-confidence is a dampener (`0.0..1.0`).
- [x] fail-closed: zero `FusionWeights { 0.0, 0.0, 0.0 }` → `CALYX_TEMPORAL_WEIGHT_SUM`
- [x] fail-closed: `BoostConfig::new(0.11, 1.10, 0.85)` → `CALYX_TEMPORAL_AP60_VIOLATION`

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** serialized `TemporalPolicy` stored in Aster durable vault manifest bytes
  (`CURRENT` pointer → immutable `manifest-*.json`, mirrored in `MANIFEST`)
- **Readback:** `cat`/`xxd` the pointed manifest and mirror on aiwonder to confirm
  `temporal_policy.never_dominant: true` is present
- **Prove:** default policy round-trips: bytes written at creation match bytes read back exactly; a policy with `never_dominant: false` never reaches the manifest (construction fails with `CALYX_TEMPORAL_AP60_VIOLATION`)
- **Evidence:** `/home/croyse/calyx/data/fsv-issue373-temporal-policy-manifest-20260609-9ca0a93`
- **Post-sweep hardening evidence:** `/home/croyse/calyx/data/fsv-issue373-temporal-policy-reopen-20260609-a54dcc1` proves a custom policy survives cold open plus second flush and invalid alpha fails closed before manifest persistence

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to GitHub issue #373
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
