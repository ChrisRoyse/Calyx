# PH69 · T03 — Classification corpora acquisition (AG News / IMDB / SST-2 / banking77 / DBpedia-14)

| Field | Value |
|---|---|
| **Phase** | PH69 — Dataset acquisition + MANIFEST + checksum FSV |
| **Stage** | S18 — Datasets & Intelligence FSV |
| **Crate** | `—` (scripts/infra) |
| **Files** | `scripts/acquire_classification.sh` (≤500) |
| **Depends on** | T01 (MANIFEST schema + verify tooling) |
| **Axioms** | A2, A34 |
| **PRD** | `28 §3` row 2, `28 §3.2` |

## Goal

Acquire the text-classification benchmark corpora (AG News, IMDB, SST-2/GLUE,
banking77, DBpedia-14) to `/zfs/archive/calyx/datasets/<name>/`, checksum-verify
each on arrival, and write MANIFEST rows. These provide grounded class-label
anchors that PH70 uses to prove Assay bits/MI differentiation contract (PRD
`28 §2`, Loom/Assay FSV).

## Build (checklist of concrete, code-level steps)

- [ ] `scripts/acquire_classification.sh`:
      uses `HF_HUB_TOKEN`; downloads with pinned revision for each:
      `/zfs/archive/calyx/datasets/ag_news/`,
      `/zfs/archive/calyx/datasets/imdb/`,
      `/zfs/archive/calyx/datasets/sst2/`,
      `/zfs/archive/calyx/datasets/banking77/`,
      `/zfs/archive/calyx/datasets/dbpedia_14/`;
      writes Parquet or Arrow files; never cross-mount rename.
- [ ] For each: record expected rows/sha256 from dataset card before download;
      post-download call `verify_dataset.sh <name>`; fail-closed on mismatch.
- [ ] Append MANIFEST rows, e.g.:
      `| ag_news | huggingface:fancyzhx/ag_news | <revision> | <sha256> | 127600 | <bytes> | CC-BY-4.0 | Assay bits/classification anchor |`
      (and equivalently for the other four).
- [ ] Ensure the test-split labels are present alongside training splits so PH70
      can run held-out evaluation; record split names in MANIFEST `version` field.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: parse a synthetic 4-class, 12-example Parquet fixture (fixed seed);
      assert row count = 12, label distribution = [3,3,3,3], sha256 matches
      known value.
- [ ] proptest: property that MANIFEST round-trip holds — sha256 of acquired file
      equals value stored in the MANIFEST row for that dataset.
- [ ] edge (≥3):
      (1) truncated Parquet file → `verify_dataset.sh` exits 1,
          `CALYX_DATASET_CHECKSUM_MISMATCH`;
      (2) all-null labels column → script logs warning, does not write MANIFEST row
          (fails-closed, not silently corrupt);
      (3) revision pin mismatch (HF updated silently) → sha256 mismatch caught by
          verify, exits 1 with diff printed.
- [ ] fail-closed: `acquire_classification.sh` without `HF_HUB_TOKEN` → exits 1,
      `CALYX_SECRET_MISSING: HF_HUB_TOKEN`; no partial directory created.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `/zfs/archive/calyx/datasets/ag_news/` … `/dbpedia_14/` on aiwonder;
  corresponding MANIFEST rows.
- **Readback:**
  ```
  bash scripts/verify_dataset.sh ag_news
  bash scripts/verify_dataset.sh imdb
  bash scripts/verify_dataset.sh sst2
  bash scripts/verify_dataset.sh banking77
  bash scripts/verify_dataset.sh dbpedia_14
  cat $CALYX_HOME/datasets/MANIFEST.md | grep -E 'ag_news|imdb|sst2|banking77|dbpedia'
  ```
- **Prove:** before: directories absent; after: each verify exits 0; MANIFEST has
  5 rows with populated sha256/rows/bytes; live sha256 matches stored value.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH69 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
