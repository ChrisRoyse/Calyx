# PH70 T12 - Dedup correctness FSV (QQP / PAWS)

| Field | Value |
|---|---|
| Phase | PH70 - Intelligence validation on real corpora |
| Issue | #605 |
| Crate | `calyx-aster` (engine) / `calyx-cli` (FSV harness) |
| Files | `crates/calyx-cli/tests/dedup_qqp_paws_fsv.rs`, `crates/calyx-cli/tests/support/dedup_qqp_paws_io.rs`, `scripts/acquire_dedup.sh` |
| PRD | `docs/dbprdplans/28_FSV_AND_TEST_DATA.md` section 2 + section 3 row 5 |

## Purpose

The DEDUP `BUILD_DONE` clause - merge true duplicates, NEVER merge conflicting
anchors - must be proven on real labeled near-duplicate corpora, not only on
synthetic vectors. QQP provides 404,290 labeled question pairs with clean
embedding separation; PAWS labeled_final provides adversarial high-lexical-
overlap pairs whose cosines do NOT separate (published SBERT finding: ~0.97 dup
vs ~0.96 non-dup mean cosine), making it the corpus that proves the
anchor-conflict guard, the architectural mechanism that blocks merges cosine
alone would wrongly make.

## Acquisition (`scripts/acquire_dedup.sh`, fail-closed)

- QQP raw TSV from the canonical Quora CDN URL; byte-count, row-count (404,290)
  and label-partition verified; sha256 recorded.
- PAWS labeled_final parquet at pinned HF revision
  `161ece9501cf0a11f3e48bd356eaa82de46d6a09`; converted to TSV with a pinned
  pyarrow venv; row counts 49,401/8,000/8,000 verified.
- Emits the deterministic FSV subset `dedup_fsv_pairs.tsv` (file-order first-N:
  256 dup + 256 non-dup per QQP calib/eval split, 200 dup + 200 non-dup PAWS
  test) plus `manifest.json` per dataset and `MANIFEST.md` rows.
- Exact error codes: `CALYX_DATASET_DOWNLOAD_FAILED`, `CALYX_DATASET_BYTES_MISMATCH`,
  `CALYX_DATASET_ROWCOUNT_MISMATCH`, `CALYX_DATASET_LABEL_PARTITION_MISSING`,
  `CALYX_DATASET_LABEL_INVALID`, `CALYX_DATASET_SUBSET_SHORT`, `CALYX_DATASET_VENV_FAILED`.

## Methodology (research-grounded)

- Embeddings: resident TEI `:8088` (`Alibaba-NLP/gte-multilingual-base`, 768-d).
- Precision-first tau calibration on the QQP calib split: smallest observed
  cosine threshold with precision >= 0.95 (destructive merges demand precision
  over recall; the lowest threshold satisfying the floor maximises recall).
- Every QQP eval pair is decided by the REAL engine (`ingest_at` into a durable
  vault, TctCosine/Collapse), with engine-vs-cosine parity asserted per pair.
- PAWS non-duplicate pairs with cosine >= tau ("would-merge") quantify the
  adversarial premise; the top-cosine ones are re-ingested with label-grounded
  conflicting `Label` anchors and must produce `AnchorConflict` + persisted
  `contested_with` rows, with both Base CF rows intact (never merged).
- Compatible-anchor control: PAWS duplicates with identical anchors still merge.

## FSV evidence (ignored test `qqp_paws_dedup_intelligence_fsv`)

Reads `/zfs/archive/calyx/datasets/dedup_fsv_pairs.tsv`; writes under
`CALYX_FSV_OUT`:

- `ph70_qqp_dedup.json` - calibration (tau/precision/recall), eval confusion
  matrix, per-slot ledger cosine readback vs TEI cosine, gates.
- `ph70_paws_anchor_guard.json` - would-merge count at tau, per-pair contested
  row readback (both directions), compatible-anchor control merges.
- `BLAKE3SUMS.txt` plus retained representative vault directories.

Gates: eval precision >= 0.85, recall >= 0.20, dup/non-dup mean-cosine
separation >= 0.10, PAWS would-merge >= 50/200, anchor-guard blocks 16/16,
compatible control merges 8/8.

## Edge audits (>=3, dedup gets more per PRD 28 section 6)

1. identical content (same bytes, same event time) -> `ExactDuplicate`
2. near-threshold: cos 0.89 vs tau 0.90 -> New; cos 0.91 -> merge
3. conflicting-anchor -> blocked, contested rows persisted (real PAWS pairs)
4. temporal-only difference (RecurrenceSeries) -> occurrence appended
5. fail-closed codes: `CALYX_DEDUP_INVALID_TAU`, `CALYX_DEDUP_NO_REQUIRED_SLOTS`,
   `CALYX_DEDUP_SLOT_NOT_IN_CONSTELLATION`, `CALYX_DEDUP_DPI_EXCEEDED`,
   `CALYX_DEDUP_ANCHOR_CONFLICT`
