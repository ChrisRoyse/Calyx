# PH41 · T06 — Recurrence signature detector (content-agree + temporal-differ)

| Field | Value |
|---|---|
| **Phase** | PH41 — DedupPolicy TctCosine + Recurrence Series + Signature |
| **Stage** | S9 — Temporal & Dedup |
| **Crate** | `calyx-aster` / `calyx-loom` |
| **Files** | `crates/calyx-aster/src/dedup/signature.rs`; `crates/calyx-loom/src/recurrence/signature.rs`; `crates/calyx-aster/src/dedup/ingest_at.rs`; CLI readback fixture support (all ≤500) |
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

- [x] Define `SignatureResult` enum: `RecurrenceSignature { same_action: CxId, new_time: EpochSecs }` | `NewContent` | `ContentMismatch` | `SameTime` (temporal slots identical — exact dup, not recurrence)
- [x] Implement `detect_recurrence_signature(new_cx: &Constellation, existing_cx: &Constellation, config: &TctCosineConfig, temporal_slot_ids: &[SlotId], guard_profile: Option<&GuardProfile>, new_time: EpochSecs) -> Result<SignatureResult, CalyxError>`:
  - Check content slots: call `cosine_passes_all_required` (T02); if not all pass → `ContentMismatch`
  - Check temporal slots: for each slot_id in `temporal_slot_ids` (E2/E3/E4 slots): compute `cos(new_temporal_k, existing_temporal_k)` — if all are ≈ 1.0 (within 1e-6) → `SameTime` (exact dup, not recurrence)
  - If content passes AND at least one temporal slot cos < 1.0 − 1e-6 → `RecurrenceSignature { same_action: existing_cx.id, new_time }`
  - Otherwise → `NewContent`
- [x] Integrate into `ingest_at` (T04): after `check_dedup` returns `Match`, call `detect_recurrence_signature`; if `RecurrenceSignature` AND `action=RecurrenceSeries` → route to `append_occurrence`
- [x] Export `temporal_slot_ids_for_panel(panel: &Panel) -> Vec<SlotId>` — returns SlotIds for E2/E3/E4 lenses from the panel; used to populate `temporal_slot_ids` parameter
- [x] The temporal slots are EXCLUDED from dedup agreement (their cosine is not checked in T02); they are only checked HERE to confirm time actually differs

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: content cos=0.95 (≥τ=0.9), temporal cos=0.30 (differs) → `RecurrenceSignature`
- [x] unit: content cos=0.95, temporal cos=1.0 (exactly same time) → `SameTime`
- [x] unit: content cos=0.85 (< τ=0.9) → `ContentMismatch` (regardless of temporal)
- [x] unit: `temporal_slot_ids_for_panel` on default panel → returns SlotIds for temporal lenses
- [x] unit: integrate with `ingest_at`: same content, different `at` → `RecurrenceSignature` fires → `DedupMerge` returned; same content, same temporal vector → `ExactDuplicate`/`SameTime` path
- [x] regression: identical content and temporal vectors never return `RecurrenceSignature`
- [x] edge: `temporal_slot_ids` is empty → treat as `SameTime` (cannot confirm time differs) so no false recurrence is appended
- [x] edge: single required content slot with exact match; single temporal slot with near-match cos=0.9999 < 1.0 → `RecurrenceSignature` (threshold is exact equality, 1e-6 tolerance)
- [x] fail-closed: `temporal_slot_ids` contains a SlotId not in `new_cx` → `CALYX_RECURRENCE_SLOT_MISSING`

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** Aster recurrence CF rows and Ledger CF rows under the durable aiwonder
  FSV vaults.
- **Readback:** trigger `CALYX_DEDUP_INGEST_AT_FSV_ROOT=<root> cargo test -p
  calyx-cli --test dedup_ingest_at_readback -- --nocapture`, then separately
  read `dedup-ingest-at-readback.json`, `BLAKE3SUMS.txt`, `calyx readback
  recurrence-series`, `calyx readback --cf recurrence`, and `calyx readback --cf
  ledger`.
- **Prove:** happy recurrence Ledger entries for the second and third ingests
  show `recurrence_signature: true`, `new_time: 200/300`, and
  `same_action: d81ef4fcfac617cc0a980c48dfb095de`; recurrence series has
  occurrences 0/1/2 at t=100/200/300. Same-temporal edge reads back one
  occurrence and an `ExactDuplicate` Ledger row with `recurrence_signature:
  false`. Missing-temporal edge reads back `CALYX_RECURRENCE_SLOT_MISSING`, one
  Ledger row, and one recurrence row.

## Evidence (2026-06-10)

- aiwonder commit: `8b0d0bba9b93a09d5ab25f6f8e15c677e200f098`
- FSV root:
  `/home/croyse/calyx/data/fsv-issue384-recurrence-signature-20260610-8b0d0bb`
- Before-read: root absent.
- Trigger: `CALYX_DEDUP_INGEST_AT_FSV_ROOT=<root> cargo test -p calyx-cli
  --test dedup_ingest_at_readback -- --nocapture`.
- After-read: 124 files; `b3sum -c BLAKE3SUMS.txt` returned OK for every file.
- `dedup-ingest-at-readback.json` BLAKE3:
  `bb5b028ff861983b2a5cd9dd547bfb2c39337eef16318422db2815990f6d51c1`.
- Direct `calyx readback recurrence-series`:
  happy path `d81ef4fcfac617cc0a980c48dfb095de` returned frequency 3,
  occurrence_count 3, cadence 100.0, and t_k values 100/200/300; same-temporal
  `fcc3cec957f44fd7056c0aaac52afdc7` and missing-temporal
  `bf11d3e93366367fe55a30604acc5f81` each returned one occurrence at t=100.
- Direct `calyx readback --cf ledger` showed happy sequence 1/2 payload bytes
  containing `recurrence_signature:true`; same-temporal sequence 1 showed
  `ExactDuplicate` and `recurrence_signature:false`; missing-temporal had only
  sequence 0.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH41 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
