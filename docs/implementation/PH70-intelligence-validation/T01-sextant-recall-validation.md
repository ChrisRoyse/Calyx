# PH70 · T01 — Sextant recall validation — Δ≥15% on real qrels

| Field | Value |
|---|---|
| **Phase** | PH70 — Intelligence validation on real corpora |
| **Stage** | S18 — Datasets & Intelligence FSV |
| **Crate** | `calyx-sextant` |
| **Files** | `scripts/validate_sextant_recall.sh` (≤500) |
| **Depends on** | PH69 T02 (BEIR/MS MARCO verified); PH24 (RRF/WeightedRRF/SingleLens fusion + provenance hits) |
| **Axioms** | A2, A15 |
| **PRD** | `28 §2` (Sextant row), `28 §3` row 1 |

## Goal

Prove the Sextant intelligence claim against grounded qrels on aiwonder: multi-lens
RRF fusion recall@10 is Δ≥15% higher than single-lens recall@10, measured on BEIR
or MS MARCO qrels, with every Hit carrying a `LedgerRef`. Read the persisted metric
CF row on aiwonder — not a harness return value.

## Build (checklist of concrete, code-level steps)

- [ ] `scripts/validate_sextant_recall.sh`:
      (1) Ingest BEIR (e.g., `nfcorpus` or `scifact` split) documents into an Aster
          vault on aiwonder using `calyx ingest`; record ingest seq.
      (2) Run `calyx search` with `strategy=SingleLens` for each query in the qrels;
          collect hits; compute recall@10 against the qrels file;
          write result to `/zfs/hot/calyx/metrics/sextant_single_recall.txt`.
      (3) Run `calyx search` with `strategy=WeightedRRF` (multi-lens) for the same
          queries; compute recall@10; write to
          `/zfs/hot/calyx/metrics/sextant_multi_recall.txt`.
      (4) Compute Δ = multi − single; assert Δ ≥ 0.15 (15 percentage points); write
          to `/zfs/hot/calyx/metrics/sextant_recall_delta.txt`.
      (5) Verify each Hit in the multi-lens results carries a `LedgerRef` (read from
          the search response or Ledger CF row).
- [ ] Fail-closed: if Δ < 0.15 script exits 1 + prints
      `CALYX_FSV_SEXTANT_RECALL_BELOW_THRESHOLD: delta=<actual>`.
- [ ] All metric files written to ZFS hot path (not `/tmp`); never cross-mount rename.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: parse a synthetic qrels file (5 queries, 10 docs each, fixed relevance
      labels); run mock single-lens and multi-lens recall functions; assert recall@10
      computed correctly for a known case where Δ > 0.15.
- [ ] proptest: property that if multi-lens recall ≥ single-lens + 0.15 then the
      script exits 0; if Δ < 0.15 then exits 1.
- [ ] edge (≥3):
      (1) qrels file with no relevant docs for a query → that query skipped, not
          counted as recall=0 erroneously;
      (2) single-lens recall = 1.0 (perfect) → Δ cannot be ≥ 0.15; script exits 1
          with structured error;
      (3) LedgerRef missing from a Hit → script exits 1,
          `CALYX_FSV_LEDGER_REF_MISSING`.
- [ ] fail-closed: empty qrels file → exits 1, `CALYX_FSV_EMPTY_QRELS`.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `/zfs/hot/calyx/metrics/sextant_single_recall.txt`,
  `/zfs/hot/calyx/metrics/sextant_multi_recall.txt`,
  `/zfs/hot/calyx/metrics/sextant_recall_delta.txt` on aiwonder.
- **Readback:**
  ```
  cat /zfs/hot/calyx/metrics/sextant_single_recall.txt
  cat /zfs/hot/calyx/metrics/sextant_multi_recall.txt
  cat /zfs/hot/calyx/metrics/sextant_recall_delta.txt
  # Confirm delta >= 0.15:
  python3 -c "delta=float(open('/zfs/hot/calyx/metrics/sextant_recall_delta.txt').read()); print('PASS' if delta>=0.15 else 'FAIL', delta)"
  ```
- **Prove:** before: metric files absent; after: `sextant_recall_delta.txt` contains
  a value ≥ 0.15; single-recall and multi-recall files contain finite floats;
  the Python assertion prints `PASS`.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output + screenshot of terminal showing PASS and Δ value)
      attached to the PH70 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
