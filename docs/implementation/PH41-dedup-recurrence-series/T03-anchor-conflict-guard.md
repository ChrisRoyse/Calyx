# PH41 · T03 — Anchor-conflict guard (MUST NOT merge conflicting anchors)

| Field | Value |
|---|---|
| **Phase** | PH41 — DedupPolicy TctCosine + Recurrence Series + Signature |
| **Stage** | S9 — Temporal & Dedup |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/dedup/policy.rs`, `crates/calyx-aster/src/dedup/engine.rs`, `crates/calyx-aster/src/vault.rs`, `crates/calyx-core/src/model/anchor.rs`, `crates/calyx-cli/tests/dedup_anchor_conflict_readback.rs` (all ≤500) |
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

- [x] Define `AnchorConflictResult` enum: `Compatible` | `Conflicting { anchor_type: AnchorTypeId, reason: ConflictReason }` | `NoAnchor` (one or both have no anchor of that type)
- [x] Define `ConflictReason` enum: `OppositeValue` | `IncompatibleVector { cos: f32 }` | `ExclusiveTag`
- [x] Implement `check_anchor_conflict(new_cx: &Constellation, existing_cx: &Constellation) -> AnchorConflictResult`:
  - for each anchor type present in `new_cx.anchors`: find matching anchor type in `existing_cx.anchors`
  - if opposite polarity anchor (e.g., `SpeakerMatch::value` differs by construction) → `Conflicting { reason: OppositeValue }`
  - if anchor has a vector (e.g., `StyleHold`) and `cos(new_anchor_vec, existing_anchor_vec) < τ_anchor` (τ_anchor = 0.70 hardcoded for anchor-type comparison) → `Conflicting { reason: IncompatibleVector { cos } }`
  - if anchor has an exclusive tag set and `new_cx.tag ≠ existing_cx.tag` → `Conflicting { reason: ExclusiveTag }`
  - if no shared anchor types → `NoAnchor` (not a conflict; proceed to cosine check)
  - if all shared anchor types are compatible → `Compatible`
- [x] Integrate into `check_dedup` (T02): call `check_anchor_conflict` first; `Conflicting` → return `DedupDecision::AnchorConflict { existing }` immediately before content-slot cosine checks
- [x] The contested region is stored as durable `online` CF rows keyed by `dedup:contested_with:<CxId>` — written when `AnchorConflict` is returned, so both constellations are aware of the conflict
- [x] `contested_with` write goes through `commit_online_rows` → `commit_rows` → WAL + group-commit (A15 provenance) and propagates errors with `?`

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: two constellations with no shared anchor types → `NoAnchor` → dedup proceeds to cosine
- [x] unit: `SpeakerMatch` anchor: new=speaker_A, existing=speaker_B (opposite) → `Conflicting { reason: OppositeValue }` → dedup returns `AnchorConflict`
- [x] unit: `StyleHold` anchor: cosine between style vectors = 0.65 < τ_anchor=0.70 → `Conflicting { reason: IncompatibleVector { cos: 0.65 } }`
- [x] unit: `StyleHold` anchor: cosine = 0.85 ≥ 0.70 → `Compatible` → cosine check proceeds
- [x] unit: `ExclusiveTag` mismatch → `Conflicting { reason: ExclusiveTag }`
- [x] unit: after `AnchorConflict` returned, both CxIds get `contested_with` written to the `online` CF; `calyx readback --cf online` confirms raw SST rows
- [x] proptest: for any pair of constellations with identical anchors, `check_anchor_conflict` returns `Compatible`
- [x] edge: `new_cx.anchors` is empty → `NoAnchor` for all checks
- [x] fail-closed: non-finite `StyleHold` vectors cannot bypass the conflict guard; corrupt persisted anchor-vector bytes fail decode; `contested_with` writes propagate WAL/group-commit errors with no silent ignore
- [x] fail-closed: exact/same-CxId duplicates with conflicting shared anchors return `CALYX_DEDUP_ANCHOR_CONFLICT` or storage fail-closed instead of bypassing through `Exact`, DPI exact fallback, self-skip, or duplicate `put`

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** durable Aster `online` CF rows keyed by `dedup:contested_with:<CxId>`, base CF rows for the separate CxIds, WAL bytes, manifests, and the hash manifest under `/home/croyse/calyx/data/fsv-issue381-anchor-conflict-20260610-00c0540`.
- **Readback:** manual aiwonder before-read confirmed the root was absent; trigger was `CALYX_DEDUP_ANCHOR_FSV_ROOT=/home/croyse/calyx/data/fsv-issue381-anchor-conflict-20260610-00c0540 cargo test -p calyx-cli --test dedup_anchor_conflict_readback -- --nocapture`; after-read used `find`, `b3sum -c BLAKE3SUMS.txt`, direct `cat` of the evidence JSON, and `calyx readback --cf online|base --vault <scenario>/vault`.
- **Prove:** `speaker_conflict` wrote reciprocal `online` CF rows for `11111111111111111111111111111111` and `22222222222222222222222222222222` with `reason=OppositeValue`, while both base CF rows exist separately. `missing_slot_conflict_before_cosine` returned `AnchorConflict` and wrote reciprocal rows even though the candidate lacked the required dense slot, proving the anchor check runs before cosine/missing-slot failure. `style_conflict` wrote `IncompatibleVector { cos: 0.6499999761581421 }`; `style_compatible` matched without contested rows; `exclusive_tag_conflict` wrote `ExclusiveTag`; `no_shared_anchor` matched and `readback --cf online` printed no rows.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder at `00c0540`
- [x] file(s) ≤ 500 lines (line-count gate passed on aiwonder)
- [x] FSV evidence attached to GitHub issue #381
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
