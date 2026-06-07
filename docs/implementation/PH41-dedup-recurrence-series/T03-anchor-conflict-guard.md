# PH41 · T03 — Anchor-conflict guard (MUST NOT merge conflicting anchors)

| Field | Value |
|---|---|
| **Phase** | PH41 — DedupPolicy TctCosine + Recurrence Series + Signature |
| **Stage** | S9 — Temporal & Dedup |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/dedup/policy.rs` (≤500) |
| **Depends on** | T02 (this phase) · PH04 (Anchor type) |
| **Axioms** | A28, A3 |
| **PRD** | `dbprdplans/25 §5`, `dbprdplans/19 §4` |

## Goal

Implement the anchor-conflict check that runs **before** any cosine comparison in
the dedup engine. Two constellations have a conflicting anchor when they carry
anchors of the same type (e.g., `SpeakerMatch`, `StyleHold`) with mutually
exclusive values — for example, opposite `SpeakerMatch` anchors, or `StyleHold`
anchors with incompatible style vectors. Such constellations MUST NOT be merged;
they stay separate and mark a **contested region**. The check is a first-pass
guard: if it fires, `check_dedup` returns `AnchorConflict` immediately without
computing any cosines.

## Build (checklist of concrete, code-level steps)

- [ ] Define `AnchorConflictResult` enum: `Compatible` | `Conflicting { anchor_type: AnchorTypeId, reason: ConflictReason }` | `NoAnchor` (one or both have no anchor of that type)
- [ ] Define `ConflictReason` enum: `OppositeValue` | `IncompatibleVector { cos: f32 }` | `ExclusiveTag`
- [ ] Implement `check_anchor_conflict(new_cx: &Constellation, existing_cx: &Constellation) -> AnchorConflictResult`:
  - for each anchor type present in `new_cx.anchors`: find matching anchor type in `existing_cx.anchors`
  - if opposite polarity anchor (e.g., `SpeakerMatch::value` differs by construction) → `Conflicting { reason: OppositeValue }`
  - if anchor has a vector (e.g., `StyleHold`) and `cos(new_anchor_vec, existing_anchor_vec) < τ_anchor` (τ_anchor = 0.70 hardcoded for anchor-type comparison) → `Conflicting { reason: IncompatibleVector { cos } }`
  - if anchor has an exclusive tag set and `new_cx.tag ≠ existing_cx.tag` → `Conflicting { reason: ExclusiveTag }`
  - if no shared anchor types → `NoAnchor` (not a conflict; proceed to cosine check)
  - if all shared anchor types are compatible → `Compatible`
- [ ] Integrate into `check_dedup` (T02): call `check_anchor_conflict` first; `Conflicting` → return `DedupDecision::AnchorConflict { existing }` immediately (no cosine computed)
- [ ] The contested region is stored as a metadata note on the constellation (a CF field): `contested_with: Option<CxId>` — written when `AnchorConflict` is returned, so both constellations are aware of the conflict
- [ ] `contested_with` write goes through the WAL + group-commit (A15 provenance)

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: two constellations with no shared anchor types → `NoAnchor` → dedup proceeds to cosine
- [ ] unit: `SpeakerMatch` anchor: new=speaker_A, existing=speaker_B (opposite) → `Conflicting { reason: OppositeValue }` → dedup returns `AnchorConflict`
- [ ] unit: `StyleHold` anchor: cosine between style vectors = 0.65 < τ_anchor=0.70 → `Conflicting { reason: IncompatibleVector { cos: 0.65 } }`
- [ ] unit: `StyleHold` anchor: cosine = 0.85 ≥ 0.70 → `Compatible` → cosine check proceeds
- [ ] unit: `ExclusiveTag` mismatch → `Conflicting { reason: ExclusiveTag }`
- [ ] unit: after `AnchorConflict` returned, both CxIds get `contested_with` written to CF; `xxd` the CF rows to confirm
- [ ] proptest: for any pair of constellations with identical anchors, `check_anchor_conflict` returns `Compatible`
- [ ] edge: `new_cx.anchors` is empty → `NoAnchor` for all checks
- [ ] fail-closed: `contested_with` write fails (WAL error) → `CALYX_WAL_WRITE_ERROR` propagated; no silent ignore

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** CF row for each constellation showing `contested_with` field; `DedupDecision::AnchorConflict` returned by `check_dedup`
- **Readback:** `calyx readback constellation --cx-id <A>` and `--cx-id <B>` after ingesting two constellations with opposite `SpeakerMatch` anchors and identical content slots; print `contested_with` fields; print `dedup_audit` output
- **Prove:** both CxIds exist separately (not merged); each shows `contested_with: <other_CxId>`; `dedup_audit` shows `anchor_conflict_blocked: true`; zero cosine computation performed (audit shows no `per_slot_cos`)

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH41 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
