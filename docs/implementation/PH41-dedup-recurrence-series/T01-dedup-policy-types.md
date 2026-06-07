# PH41 · T01 — `DedupPolicy` types + vault-creation config

| Field | Value |
|---|---|
| **Phase** | PH41 — DedupPolicy TctCosine + Recurrence Series + Signature |
| **Stage** | S9 — Temporal & Dedup |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/dedup/mod.rs` (≤500) |
| **Depends on** | PH09 (vault creation entry point) |
| **Axioms** | A28, A3 |
| **PRD** | `dbprdplans/25 §5`, `dbprdplans/25 §6` |

## Goal

Define all types governing deduplication behavior, set at vault or collection
creation. `DedupPolicy` is a first-class vault-level option stored in the
manifest CF. `TctCosineConfig` encodes which slots must agree, the threshold
strategy, and the action to take on match. `DedupAction::RecurrenceSeries` is
the path that captures recurring events as a series (§4 PRD).

## Build (checklist of concrete, code-level steps)

- [ ] Define `TauStrategy` enum: `PerSlot(Vec<(SlotId, f32)>)` (explicit per-slot threshold) | `Calibrated` (reuse Ward `GuardProfile` conformal calibration from PH38)
- [ ] Define `DedupAction` enum: `Collapse` (replace existing with merged) | `Link` (store both + a link record) | `RecurrenceSeries` (append occurrence to series)
- [ ] Define `TctCosineConfig { required_slots: Vec<SlotId>, tau: TauStrategy, action: DedupAction }` — validate: `required_slots` must not contain any `SlotId` that maps to a temporal lens (E2/E3/E4); violation → `CALYX_DEDUP_TEMPORAL_SLOT_IN_REQUIRED`
- [ ] Define `DedupPolicy` enum: `Off` | `Exact` | `TctCosine(TctCosineConfig)`
- [ ] Implement `DedupPolicy::validate(panel: &Panel) -> Result<(), CalyxError>`: for `TctCosine`, cross-check `required_slots` against panel to ensure none are temporal lenses; `required_slots` empty → `CALYX_DEDUP_NO_REQUIRED_SLOTS`
- [ ] Define `DedupResult` enum: `New(CxId)` | `DedupMerge { into: CxId, occurrence: OccurrenceId }` | `ExactDuplicate(CxId)` — returned by `ingest_at`
- [ ] Define `OccurrenceId(u64)` — monotonic per-series identifier
- [ ] `serde::{Serialize, Deserialize}` + `Clone` + `Debug` + `PartialEq` on all types
- [ ] Store `DedupPolicy` in the vault manifest CF at creation; read it back on every `ingest_at` call

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `DedupPolicy::TctCosine(TctCosineConfig { required_slots: [e2_slot_id], .. })` → `CALYX_DEDUP_TEMPORAL_SLOT_IN_REQUIRED`
- [ ] unit: `DedupPolicy::TctCosine(TctCosineConfig { required_slots: [], .. })` → `CALYX_DEDUP_NO_REQUIRED_SLOTS`
- [ ] unit: `DedupPolicy::Off` → `validate` always returns `Ok(())`
- [ ] unit: `DedupPolicy` round-trips through `serde_json` byte-exact (all three variants)
- [ ] unit: `TauStrategy::PerSlot` with two slots round-trips; `TauStrategy::Calibrated` round-trips
- [ ] edge: `DedupAction::RecurrenceSeries` serialized to `"RecurrenceSeries"` (not variant index)
- [ ] fail-closed: `DedupPolicy` written to manifest CF, vault reloaded → policy reads back identical

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `DedupPolicy` JSON stored in manifest CF row for the vault
- **Readback:** `calyx readback vault-manifest --field dedup_policy` on a vault created with `TctCosine { action: RecurrenceSeries }`; `xxd` the raw CF row
- **Prove:** JSON round-trips; `required_slots` does not contain any E2/E3/E4 slot IDs; `action` field reads `"RecurrenceSeries"`

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH41 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
