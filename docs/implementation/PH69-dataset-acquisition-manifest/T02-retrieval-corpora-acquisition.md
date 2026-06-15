# PH69 · T02 — Retrieval corpora acquisition (BEIR / MS MARCO / NQ / TREC-COVID)

| Field | Value |
|---|---|
| **Phase** | PH69 — Dataset acquisition + MANIFEST + checksum FSV |
| **Stage** | S18 — Datasets & Intelligence FSV |
| **Crate** | `—` (scripts/infra) |
| **Files** | `scripts/acquire_retrieval.sh` (≤500) |
| **Depends on** | T01 (MANIFEST schema + verify tooling) |
| **Axioms** | A2, A34 |
| **PRD** | `28 §3` row 1, `28 §3.2` |

## Goal

Acquire the text-retrieval benchmark corpora (BEIR, MS MARCO, Natural Questions,
TREC-COVID) to `/zfs/archive/calyx/datasets/<name>/`, checksum-verify each on
arrival, and write MANIFEST rows. These datasets provide the qrels that PH70 uses
to prove Sextant recall Δ≥15% (PRD `28 §2`, Sextant FSV).

## Build (checklist of concrete, code-level steps)

- [ ] `scripts/acquire_retrieval.sh`:
      uses `HF_HUB_TOKEN` from env; calls `huggingface-cli download` (or Python
      `datasets.load_dataset`) with pinned `revision` hash for each corpus;
      target path: `/zfs/archive/calyx/datasets/beir/`,
      `/zfs/archive/calyx/datasets/msmarco/`,
      `/zfs/archive/calyx/datasets/natural_questions/`,
      `/zfs/archive/calyx/datasets/trec_covid/`;
      writes raw corpus + qrels files; never writes to `/tmp` then renames
      (EXDEV risk on ZFS cross-mount).
- [ ] For each dataset: record expected rows and sha256 from the dataset card
      **before** downloading (pin in script comment); after download call
      `scripts/verify_dataset.sh <name>`; fail-closed if verify exits non-zero.
- [ ] Append MANIFEST row for each:
      `| beir | huggingface:BeIR/beir | <revision> | <sha256> | <rows> | <bytes> | Apache-2.0 | Sextant recall qrels |`
      (and equivalently for msmarco / natural_questions / trec_covid).
- [ ] Subset selection: use the BEIR `nfcorpus` or `scifact` split for initial
      verification (small); use MS MARCO dev-small qrels subset; NQ and TREC-COVID
      full test sets. Record which split in MANIFEST `version` column.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: parse a synthetic qrels file (3 query/doc/rel triples, fixed content)
      and assert the row count and a known-line checksum match expected values.
- [ ] proptest: property that the verify script, given the MANIFEST row it just
      wrote, exits 0 (round-trip acquire → verify).
- [ ] edge (≥3):
      (1) partial download (truncated file) → verify exits 1, `CALYX_DATASET_CHECKSUM_MISMATCH`;
      (2) zero-byte qrels file → verify exits 1, `CALYX_DATASET_ROWCOUNT_MISMATCH`;
      (3) HF download fails (bad token) → script exits 1, structured error, no
          partial MANIFEST row written.
- [ ] fail-closed: missing `HF_HUB_TOKEN` env var → script exits 1 immediately,
      prints `CALYX_SECRET_MISSING: HF_HUB_TOKEN`.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `/zfs/archive/calyx/datasets/beir/` (and msmarco / nq / trec_covid) on
  aiwonder; rows in `datasets/MANIFEST.md`.
- **Readback:**
  ```
  bash scripts/verify_dataset.sh beir
  bash scripts/verify_dataset.sh msmarco
  bash scripts/verify_dataset.sh natural_questions
  bash scripts/verify_dataset.sh trec_covid
  cat $CALYX_HOME/datasets/MANIFEST.md | grep -E 'beir|msmarco|natural_questions|trec_covid'
  ```
- **Prove:** before: directories absent; after: `verify_dataset.sh` exits 0 for
  each; MANIFEST contains 4 rows with populated sha256/rows/bytes fields;
  `sha256sum` computed live matches MANIFEST value.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH69 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
