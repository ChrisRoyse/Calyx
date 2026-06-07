# PH41 · T06 — Recurrence signature detector (content-agree + temporal-differ)

| Field | Value |
|---|---|
| **Phase** | PH41 — DedupPolicy TctCosine + Recurrence Series + Signature |
| **Stage** | S9 — Temporal & Dedup |
| **Crate** | `calyx-loom` |
| **Files** | `crates/calyx-loom/src/recurrence/signature.rs` (≤500) |
| **Depends on** | T05 (this phase) · T02 (this phase — cosine engine) |
| **Axioms** | A28, A29 |
| **PRD** | `dbprdplans/25 §4c` |

## Goal

Implement the recurrence signature detector: the function that reads the specific
pattern — all CONTENT lenses agree (`cos(new_k, existing_k) ≥ τ_k` for every
required content slot) AND the TEMPORAL lenses (E2/E3/E4) differ (at least one
temporal slot cosine < 1.0 because event times differ) — and classifies this as
a `RecurrenceSignature`. This is the automatic recognition of "the exact same
action, again, at a new time." The detector fires within `ingest_at`; when it
fires and `DedupPolicy::TctCosine { action: RecurrenceSeries }` is set, the
ingest routes to `append_occurrence`.

## Build (checklist of concrete, code-level steps)

- [ ] Define `SignatureResult` enum: `RecurrenceSignature { same_action: CxId, new_time: EpochSecs }` | `NewContent` | `ContentMismatch` | `SameTime` (temporal slots identical — exact dup, not recurrence)
- [ ] Implement `detect_recurrence_signature(new_cx: &Constellation, existing_cx: &Constellation, config: &TctCosineConfig, temporal_slot_ids: &[SlotId], guard_profile: Option<&GuardProfile>, new_time: EpochSecs) -> Result<SignatureResult, CalyxError>`:
  - Check content slots: call `cosine_passes_all_required` (T02); if not all pass → `ContentMismatch`
  - Check temporal slots: for each slot_id in `temporal_slot_ids` (E2/E3/E4 slots): compute `cos(new_temporal_k, existing_temporal_k)` — if all are ≈ 1.0 (within 1e-6) → `SameTime` (exact dup, not recurrence)
  - If content passes AND at least one temporal slot cos < 1.0 − 1e-6 → `RecurrenceSignature { same_action: existing_cx.id, new_time }`
  - Otherwise → `NewContent`
- [ ] Integrate into `ingest_at` (T04): after `check_dedup` returns `Match`, call `detect_recurrence_signature`; if `RecurrenceSignature` AND `action=RecurrenceSeries` → route to `append_occurrence`
- [ ] Export `temporal_slot_ids_for_panel(panel: &Panel) -> Vec<SlotId>` — returns SlotIds for E2/E3/E4 lenses from the panel; used to populate `temporal_slot_ids` parameter
- [ ] The temporal slots are EXCLUDED from dedup agreement (their cosine is not checked in T02); they are only checked HERE to confirm time actually differs

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: content cos=0.95 (≥τ=0.9), temporal cos=0.30 (differs) → `RecurrenceSignature`
- [ ] unit: content cos=0.95, temporal cos=1.0 (exactly same time) → `SameTime`
- [ ] unit: content cos=0.85 (< τ=0.9) → `ContentMismatch` (regardless of temporal)
- [ ] unit: `temporal_slot_ids_for_panel` on default panel → returns 3 SlotIds (one per E2/E3/E4)
- [ ] unit: integrate with `ingest_at`: same content, different `at` → `RecurrenceSignature` fires → `DedupMerge` returned; same content, same `at` → `ExactDuplicate` or `SameTime` path
- [ ] proptest: for any pair of constellations with identical content and identical time, `detect_recurrence_signature` returns `SameTime` (never `RecurrenceSignature`)
- [ ] edge: `temporal_slot_ids` is empty → treat as `SameTime` (cannot confirm time differs) → `ContentMismatch` fallback (safe: no false recurrence)
- [ ] edge: single required content slot with exact match; single temporal slot with near-match cos=0.9999 < 1.0 → `RecurrenceSignature` (threshold is exact equality, 1e-6 tolerance)
- [ ] fail-closed: `temporal_slot_ids` contains a SlotId not in `new_cx` → `CALYX_RECURRENCE_SLOT_MISSING`

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `SignatureResult` returned and logged in the `LedgerEntry` for the ingest call
- **Readback:** `calyx readback ledger --cx-id <CxId>` after ingesting the same content at t=100 and t=200; print `dedup_decision` field; confirm it shows `RecurrenceSignature` was the trigger
- **Prove:** Ledger entry for second ingest shows `recurrence_signature: true`, `new_time: 200`, `same_action: <CxId>`; recurrence series has 2 occurrences; first ingest shows `New`

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH41 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
