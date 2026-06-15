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

- [x] Define `TauStrategy` enum: `PerSlot(Vec<(SlotId, f32)>)` (explicit per-slot threshold) | `Calibrated` (reuse Ward `GuardProfile` conformal calibration from PH38)
- [x] Define `DedupAction` enum: `Collapse` (replace existing with merged) | `Link` (store both + a link record) | `RecurrenceSeries` (append occurrence to series)
- [x] Define `TctCosineConfig { required_slots: Vec<SlotId>, tau: TauStrategy, action: DedupAction }` — validate: every `required_slots` member must exist in the active panel and must not map to a temporal or dedup-excluded lens (E2/E3/E4); violations → `CALYX_DEDUP_SLOT_NOT_IN_PANEL` or `CALYX_DEDUP_TEMPORAL_SLOT_IN_REQUIRED`
- [x] Define `DedupPolicy` enum: `Off` | `Exact` | `TctCosine(TctCosineConfig)`
- [x] Implement `DedupPolicy::validate(panel: &Panel) -> Result<(), CalyxError>`: for `TctCosine`, cross-check `required_slots` against panel to ensure each slot is present and no slot is temporal/dedup-excluded; `required_slots` empty → `CALYX_DEDUP_NO_REQUIRED_SLOTS`
- [x] Define `DedupResult` enum: `New(CxId)` | `DedupMerge { into: CxId, occurrence: OccurrenceId }` | `ExactDuplicate(CxId)` — returned by `ingest_at`
- [x] Define `OccurrenceId(u64)` — monotonic per-series identifier
- [x] `serde::{Serialize, Deserialize}` + `Clone` + `Debug` + `PartialEq` on all types
- [x] Store `DedupPolicy` in the vault manifest at creation; durable reopen reads the manifest value back so PH41 T04 `ingest_at` inherits persisted policy state.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: `DedupPolicy::TctCosine(TctCosineConfig { required_slots: [e2_slot_id], .. })` → `CALYX_DEDUP_TEMPORAL_SLOT_IN_REQUIRED`
- [x] unit: `DedupPolicy::TctCosine(TctCosineConfig { required_slots: [missing_slot_id], .. })` → `CALYX_DEDUP_SLOT_NOT_IN_PANEL`
- [x] unit: `DedupPolicy::TctCosine(TctCosineConfig { required_slots: [dedup_excluded_slot_id], .. })` → `CALYX_DEDUP_TEMPORAL_SLOT_IN_REQUIRED`
- [x] unit: `DedupPolicy::TctCosine(TctCosineConfig { required_slots: [], .. })` → `CALYX_DEDUP_NO_REQUIRED_SLOTS`
- [x] unit: `DedupPolicy::Off` → `validate` always returns `Ok(())`
- [x] unit: `DedupPolicy` round-trips through `serde_json` byte-exact (all three variants)
- [x] unit: `TauStrategy::PerSlot` with two slots round-trips; `TauStrategy::Calibrated` round-trips
- [x] edge: `DedupAction::RecurrenceSeries` serialized to `"RecurrenceSeries"` (not variant index)
- [x] fail-closed: `DedupPolicy` written to manifest, vault reloaded → policy reads back identical

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `DedupPolicy` JSON stored in the Aster durable vault manifest:
  `CURRENT` → `manifest-00000000000000000001.json` plus `MANIFEST` mirror.
- **Readback:** `calyx readback vault-manifest --field dedup_policy` on a vault
  created with `TctCosine { action: RecurrenceSeries }`; `xxd` the raw manifest
  bytes.
- **Prove:** JSON round-trips; `required_slots` does not contain any E2/E3/E4
  slot IDs; `action` field reads `"RecurrenceSeries"`.

**Status:** DONE / FSV-signed-off on aiwonder at commit `0083015`
(`Add dedup policy manifest persistence`). Evidence root:
`/home/croyse/calyx/data/fsv-issue379-dedup-policy-20260610-0083015`.
Manual after-read verified `BLAKE3SUMS.txt`, opened
`dedup-policy-input.json`, `dedup-policy-readback.json`, and the immutable
manifest, ran `calyx readback vault-manifest --field dedup_policy`, and read
raw manifest bytes with `xxd`.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH41 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
