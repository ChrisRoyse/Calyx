# 01 — Anchors-at-ingest

- **Issue:** #868 (epic #867)   **Phase:** 1 (prepare substrate)   **Date (UTC):** 2026-06-25   **Vault/panel:** synthetic `anchorfsv2` / `biomed-clinical-fast` (14 active text lenses, 17 slots)
- **Goal:** thread typed anchors through the streaming JSONL ingest so each constellation is grounded at ingest (the QA correct-answer as `label:answer`, the source as `label:dataset`, `test-pass` for verified rows) — no separate per-row `calyx anchor` pass — and FSV that the anchors physically land in base-CF + the Anchors CF the kernel reads.

## Root-cause analysis (first principles)

The discovery program needs **grounding** = typed anchors on constellations (kernel groundedness, Oracle gate). The corpus was ingested with **none**. Tracing the path:

- The storage layer **already** supports anchors-at-ingest: `AsterVault::put` (`store.rs:88-94`) and `put_batch` → `stage_constellation_rows` (`vault.rs:391-394`) both write every `constellation.anchors` entry to `ColumnFamily::Anchors` keyed `anchor_key(cx, kind)` — the **same** CF the post-hoc `calyx anchor` command writes, and the one the kernel's `domain_anchors(kind)` reads (`aster_bridge.rs:193`). Base-CF also embeds the anchors in the constellation blob (`encode.rs`).
- The gap was **entirely in the CLI ingest layer**: `measure_constellation*` hard-code `anchors: Vec::new()` + `ungrounded: true` (`constellation.rs:49,168`), and the batch JSONL parser (`batch.rs`) deserialized only `text` + `metadata`, **silently dropping** any `anchors` (and the existing `label`) field — serde ignores unknown fields by default.

Two further root-cause findings (recorded, not worked around):
1. **`ungrounded` flag.** Measure-time default is `true`. The canonical rule elsewhere is `ungrounded = anchors.is_empty()` (`dedup/ingest_input.rs:128`). Fixed to mirror it when threading anchors.
2. **Re-ingest of existing text with *added* anchors is a silent no-op.** `put`/`put_batch` short-circuit on the dedup path (`base_exists` → `Ok(id)`) **without** writing the new anchors (the new anchor is "compatible", not "conflicting"). ⇒ **backfilling anchors onto the already-ingested `corpus` vault by re-running the feeder will NOT add them.** #869 must use a **fresh** vault. (Fine in practice: the `corpus` medmcqa ingest is incomplete anyway.) Filed as a hazard for #869.

## What was changed (engine, domain-agnostic)

`crates/calyx-cli/src/cmd/ingest/`:
- `batch.rs` — `BatchLine` gains `anchors: Vec<AnchorSpec>` (`{kind, value, source?, confidence?}`); each spec is parsed into a real `Anchor` via the **same** `parse_anchor_kind`/`parse_anchor_value` the `calyx anchor` CLI uses, `source` default `calyx-ingest`, `confidence` default `1.0`. A malformed anchor (unknown kind, bad value, out-of-range confidence) is a **loud, line-numbered usage error** — never a silent skip (doctrine: no fallback). `BatchRow` is now `(text, metadata, Vec<Anchor>)`.
- `command.rs` — `flush_measure_batch` threads anchors onto `cx.anchors` and sets `cx.flags.ungrounded = anchors.is_empty()`, mirroring the metadata threading.
- `parse.rs` — `validate_confidence` made `pub(super)` for reuse.

The feeder/corpus-builder (not the engine) decides *what* to attach, so the engine stays domain-agnostic — consistent with best practice (anchors as a closed, provenance-carrying vocabulary; grounding status verified independently, not blindly trusted — MDPI *Grounded KG Extraction* 15(3):178, 2026; GNBR/literature-KG repurposing, PMID 31797619).

## What was run (exact commands, aiwonder, patched `repo/target/release/calyx`)

```
# fresh vault + real production panel
calyx create-vault anchorfsv2 ; calyx panel template swap --template biomed-clinical-fast --vault anchorfsv2   # 17 active slots
# synthetic batch: 6 anchored QA rows (3 anchors each) + 1 unanchored CONTROL row
calyx ingest anchorfsv2 --batch anchorfsv.jsonl --idempotent     # exit 0, 7 rows
calyx verify-chain anchorfsv2                                     # {"status":"ok"}
calyx readback --cf anchors --vault <VDIR>                        # physical Anchors-CF dump
calyx kernel anchorfsv2 --anchor test-pass --rebuild
```

## Raw evidence / FSV (against stored artifacts, not return values)

**Before/after with the unknown-field bug (same JSONL, two binaries):**
- OLD binary (`/home/croyse/calyx/target/release/calyx`, pre-patch) → `readback --cf anchors` = **EMPTY** (anchors silently dropped). This is the bug, reproduced.
- PATCHED binary → Anchors CF populated.

**Anchors CF physical content (patched), decoded:**
```
physical SST lines           : 36   (18 logical anchors × 2 LSM levels: 0001.sst + 0002.sst)
distinct (KEY,VALUE) anchors : 18   (expect 6 rows × 3)
distinct cx with anchors     : 6    (the 7th = control, correctly 0)
anchor-kind histogram        : {label:answer:6, label:dataset:6, test-pass:6}
answer-anchor decoded values : {A, A, yes, C, B, A}  ==  synthetic truth sorted ['A','A','A','B','C','yes']   ✅ byte-exact
anchor source field          : "calyx-ingest" (default applied)
```

**Kernel grounding reads the ingest-time anchors:**
```
calyx kernel anchorfsv2 --anchor test-pass   -> {"recall":0.857,"total_cx":7,"kernel_cx_ids":["15346002…"],"grounding_gaps":["test_pass:missing_grounding:1"]}
calyx kernel anchorfsv2 --anchor label:answer -> recall 0.857, gap label:answer:1
```
`recall = 6/7 = 0.857` ⇒ exactly **6 of 7** constellations are grounded on the ingest-time anchors; `missing_grounding:1` correctly fingers the **single unanchored control row**. The kernel surface (`intelligence/kernel.rs`) grounds via `has_any_anchor(cx, kind)` over the decoded constellations — i.e. it reads precisely the anchors we threaded.

**Gate:** `cargo fmt --check` clean; `cargo clippy --workspace --all-targets -D warnings` clean; `cargo nextest -p calyx-cli` = **508/508 pass** incl. 3 new FSV tests (`batch_ingest_threads_anchors_into_base_cf_and_anchors_cf`, `batch_ingest_without_anchors_stays_ungrounded`, `batch_jsonl_malformed_anchor_is_loud_usage_error`).

## Findings (honest)

- **Grounded ✅** — anchors-at-ingest works end-to-end through the real production binary + 14-lens panel; physical presence and byte-exact values verified in the Anchors CF; the kernel grounds 6/7 synthetic rows on them. Acceptance for #868 met.
- The `calyx kernel` CLI is the **anchor-presence** kernel (recall = grounded/total). The full lodestar `AssocGraph` + `groundedness_distance` betweenness kernel needs **woven Loom cross-terms** (edges) — **#870** — before multi-hop groundedness propagation is exercisable. Anchors are now ready for it.

## Conclusion & next step

#868 **done** (code + FSV). Unblocks:
- **#869** — re-ingest the corpus with anchors. **Must be a fresh vault** (silent-no-op finding above). Feeder change: map each prov.jsonl row's existing `label` → `{"kind":"label:answer","value":<label>}`, `metadata.source_dataset` → `{"kind":"label:dataset",...}`, and add `{"kind":"test-pass","value":"true"}`. The JSONL `anchors` array is the only new field.
- **#870** — weave Loom cross-terms (the association-graph edges) so `groundedness_distance` can propagate beyond self-anchored nodes.
