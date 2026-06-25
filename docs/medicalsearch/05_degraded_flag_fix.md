# 05 - Degraded-flag fix for temporal sidecars

- **Issue:** #872 (epic #867)   **Date (UTC):** 2026-06-25   **FSV host:** aiwonder
- **Goal:** make `flags.degraded` mean an applicable primary content measurement failed, not that retrieval-only temporal sidecars were absent for text ingest.

## Root-cause analysis

The `biomed-clinical-fast` and default domain panels append E2/E3/E4 temporal controls as active `Structured` slots with `retrieval_only=true`. Text batch ingest correctly emits `Absent(NotApplicable)` for those sidecars, but the degraded flag was computed as:

```
degraded |= vector.is_absent();
```

That made expected temporal absence indistinguishable from a real content-lens failure. The bug existed in both CLI ingest paths and the MCP ingest path.

The durable readback surface also needed tightening. Base CF stores slot hashes, not full slot vectors; `decode_constellation_base` reconstructs placeholder `Absent(NotApplicable)` vectors until the per-slot CFs are read. `readback cx-list` now resolves each slot through its `slot_NN` CF row and decodes the actual `SlotVector`, marking `payload_source` in the JSON.

## What changed

- `Slot::counts_toward_degraded(input_modality)` returns true only for active, input-modality, non-retrieval-only slots.
- CLI single-row and batch measurement use that predicate before OR-ing absence into `flags.degraded`.
- MCP ingest uses the same predicate.
- `readback cx-list` reports decoded slot payloads from slot CFs, plus compact slot summaries.
- Regression coverage verifies:
  - a populated content slot plus absent retrieval-only temporal sidecar stores `degraded=false`;
  - a missing applicable content lens still stores `degraded=true`.

## FSV evidence

FSV root:

```
/home/croyse/calyx/fsv/issue872-degraded-sidecar-20260625T084937Z
```

Patched binary:

```
/home/croyse/calyx/repo/target/debug/calyx
sha256 4e458120632af044f3d119ef3a0ff591ac75006d1b563377d00bbb7189b25cb9
```

Positive case:

- Created a real CLI vault from `text-default`.
- Parked default content slots 0-4.
- Added a registered algorithmic text scalar lens.
- Batch ingested one text row.
- `readback cx-list --vault <vault>` decoded actual slot CF payloads.
- Before rows `0`, after rows `1`.
- `flags.degraded=false`.
- Slot summary: `dense_slots=1`, `absent_slots=8`, `absent_reasons={lens_inactive:5, not_applicable:3}`.
- Payload sources: `{slot_cf:9}`.
- `verify-chain`: `status=ok`, `checked=1`.

Edge case:

- Empty batch ingest wrote no rows.
- Before rows `0`, after rows `0`.
- Ingest stdout bytes `0`.
- `verify-chain`: `status=ok`, `checked=0`.

Negative case:

- Fresh `medical-default` vault with unavailable registry content lenses.
- Before rows `0`, after rows `1`.
- `flags.degraded=true`.
- Slot summary: `absent_slots=6`, `absent_reasons={lens_unavailable:3, not_applicable:3}`.
- Payload sources: `{slot_cf:6}`.
- `verify-chain`: `status=ok`, `checked=1`.

Summary artifact:

```
/home/croyse/calyx/fsv/issue872-degraded-sidecar-20260625T084937Z/issue872_fsv_summary.json
bytes 6433
sha256 bbd0cdf615e63c11b1ecaf000f12ad438cda8f15d80cad0f2cc91e10f42d29b4
```

## Gate checks

On aiwonder:

```
cargo fmt --all -- --check
git diff --check
bash scripts/linecount.sh
cargo test -p calyx-cli cmd::ingest::tests::retrieval_only_temporal_absence_does_not_degrade_content_ingest
cargo test -p calyx-cli --test dedup_audit_readback dedup_audit_readback_prints_reversible_undo_bytes
```

All passed before the FSV run.
