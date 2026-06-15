# PH69 · T06 — Dedup corpora acquisition (Quora Question Pairs / PAWS)

| Field | Value |
|---|---|
| **Phase** | PH69 — Dataset acquisition + MANIFEST + checksum FSV |
| **Stage** | S18 — Datasets & Intelligence FSV |
| **Crate** | `—` (scripts/infra) |
| **Files** | `scripts/acquire_dedup.sh` (≤500) |
| **Depends on** | T01 (MANIFEST schema + verify tooling) |
| **Axioms** | A2, A34 |
| **PRD** | `28 §3` row 5, `28 §3.2` |

## Goal

Acquire the near-duplicate corpora (Quora Question Pairs, PAWS) to
`/zfs/archive/calyx/datasets/<name>/`, checksum-verify each, and write MANIFEST
rows. These provide the duplicate/not-duplicate labels used by PH70 to prove
TCT cosine-Gτ dedup correctness: the dedup policy must merge true duplicates and
never merge conflicting anchors (PRD `28 §2`, dedup FSV; A28/A29).

## Build (checklist of concrete, code-level steps)

- [ ] `scripts/acquire_dedup.sh`:
      Quora Question Pairs — HF `quora` dataset to
      `/zfs/archive/calyx/datasets/quora_qp/`; ~400 K pairs with `is_duplicate` label.
      PAWS — HF `paw-x` or `google-research-datasets/paws` to
      `/zfs/archive/calyx/datasets/paws/`; ~49 K labeled paraphrase pairs.
      Pin revisions; write to ZFS target directly; call verify after each.
- [ ] For each: record expected rows/sha256 pre-download; verify post-download;
      fail-closed on mismatch.
- [ ] MANIFEST rows, e.g.:
      `| quora_qp | huggingface:quora | <revision> | <sha256> | 404290 | <bytes> | custom/non-commercial | TCT cosine-Gτ dedup |`
      `| paws | huggingface:google-research-datasets/paws | <revision> | <sha256> | 49401 | <bytes> | Apache-2.0 | TCT cosine-Gτ dedup |`
- [ ] Ensure both the `is_duplicate=1` and `is_duplicate=0` partitions are present
      so PH70 can test that the dedup policy never merges conflicting-anchor pairs.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: parse a synthetic 6-row duplicate/non-duplicate TSV (3 dup, 3 non-dup,
      fixed seed content); assert row count = 6, dup count = 3.
- [ ] proptest: property that MANIFEST round-trip holds — verify exits 0 on a
      freshly acquired file using the sha256 just written.
- [ ] edge (≥3):
      (1) file with only `is_duplicate=1` rows (missing negatives) → script logs
          warning, does not write MANIFEST (fails-closed — incomplete for FSV gate);
      (2) sha256 mismatch after download → `CALYX_DATASET_CHECKSUM_MISMATCH`;
      (3) PAWS row count ≠ expected → `CALYX_DATASET_ROWCOUNT_MISMATCH`.
- [ ] fail-closed: missing `HF_HUB_TOKEN` → exits 1,
      `CALYX_SECRET_MISSING: HF_HUB_TOKEN`.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `/zfs/archive/calyx/datasets/quora_qp/` and `/zfs/archive/calyx/datasets/paws/`
  on aiwonder; MANIFEST rows.
- **Readback:**
  ```
  bash scripts/verify_dataset.sh quora_qp
  bash scripts/verify_dataset.sh paws
  cat $CALYX_HOME/datasets/MANIFEST.md | grep -E 'quora|paws'
  python3 -c "import pathlib,csv; rows=list(csv.DictReader(open('/zfs/archive/calyx/datasets/paws/train.tsv'),delimiter='\t')); print('rows:',len(rows),'dup_count:',sum(1 for r in rows if r['label']=='1'))"
  ```
- **Prove:** before: directories absent; after: verify exits 0 for both; row counts
  match MANIFEST; label distribution confirms both positive and negative pairs
  present; live sha256 matches stored value.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH69 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
