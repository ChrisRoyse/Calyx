# PH70 · T03 — Lodestar kernel-only recall validation — ≥0.95 on ≥3 corpora

| Field | Value |
|---|---|
| **Phase** | PH70 — Intelligence validation on real corpora |
| **Stage** | S18 — Datasets & Intelligence FSV |
| **Crate** | `calyx-lodestar` |
| **Files** | `scripts/validate_lodestar_kernel.sh` (≤500) |
| **Depends on** | PH69 T05 (WordNet/ConceptNet/Cora/ogbn verified); PH33 (kernel index + kernel_answer + grounding_gaps) |
| **Axioms** | A2, A10, A11, A15 |
| **PRD** | `28 §2` (Lodestar row), `28 §3` row 4 |

## Goal

Prove Lodestar kernel-only recall ≥ 0.95·full-recall on ≥3 real graph corpora
(WordNet, Cora, and ≥1 more from PH69) on aiwonder. Read the persisted
`kernel_recall` metric file on aiwonder — not a return value. The ≥3-corpora
requirement satisfies the `BUILD_DONE` `KERNEL_ANY` clause.

## Build (checklist of concrete, code-level steps)

- [ ] `scripts/validate_lodestar_kernel.sh`:
      For each of ≥3 corpora (WordNet, Cora/ogbn, ConceptNet or Wiktionary):
      (1) Build the kernel graph via `calyx kernel build --corpus <name>`; record
          `kernel_size` (target ~10% of nodes) and `mfvs_size` (~1% of nodes).
      (2) Run full-recall query baseline: for a sample of N=500 query nodes,
          retrieve top-k results using the full graph; write to
          `/zfs/hot/calyx/metrics/lodestar_<corpus>_full_recall.txt`.
      (3) Run kernel-only recall: same N=500 queries, kernel-only index;
          write to `/zfs/hot/calyx/metrics/lodestar_<corpus>_kernel_recall.txt`.
      (4) Compute ratio = kernel_recall / full_recall; assert ratio ≥ 0.95; write
          to `/zfs/hot/calyx/metrics/lodestar_<corpus>_recall_ratio.txt`.
      (5) Write `grounding_gaps` output to
          `/zfs/hot/calyx/metrics/lodestar_<corpus>_gaps.txt`.
- [ ] Fail-closed: if ratio < 0.95 on any corpus → exits 1,
      `CALYX_FSV_LODESTAR_KERNEL_RECALL_BELOW_0.95: corpus=<name> ratio=<actual>`.
- [ ] Confirm ≥3 corpora all pass; if only 2 pass → exits 1,
      `CALYX_FSV_LODESTAR_INSUFFICIENT_CORPORA: need ≥3, got <n>`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: a synthetic 50-node graph (seed=42) with a planted MFVS of 5 nodes;
      build kernel; assert `kernel_size` ≤ 10 nodes and kernel recall ≥ 0.95
      on 20 queries with known answers.
- [ ] proptest: property that kernel recall monotonically increases with kernel size
      (larger kernel → recall closer to 1.0).
- [ ] edge (≥3):
      (1) graph with no strongly-connected components (DAG) → MFVS = 0; kernel
          recall = 1.0 (trivially passes);
      (2) single-node graph → kernel = that node; recall = 1.0;
      (3) corpus with < 3 nodes → exits 1, `CALYX_KERNEL_CORPUS_TOO_SMALL`.
- [ ] fail-closed: corpus not found in MANIFEST → exits 1,
      `CALYX_DATASET_NOT_FOUND`.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `/zfs/hot/calyx/metrics/lodestar_wordnet_recall_ratio.txt`,
  `lodestar_cora_recall_ratio.txt`, `lodestar_<third>_recall_ratio.txt` on aiwonder.
- **Readback:**
  ```
  cat /zfs/hot/calyx/metrics/lodestar_wordnet_recall_ratio.txt
  cat /zfs/hot/calyx/metrics/lodestar_cora_recall_ratio.txt
  cat /zfs/hot/calyx/metrics/lodestar_wordnet_gaps.txt | head -20
  python3 -c "
  import pathlib, glob
  ratios = [float(p.read_text()) for p in pathlib.Path('/zfs/hot/calyx/metrics').glob('lodestar_*_recall_ratio.txt')]
  print('corpora:',len(ratios),'min_ratio:',min(ratios),'pass:',len(ratios)>=3 and min(ratios)>=0.95)
  "
  ```
- **Prove:** before: metric files absent; after: ≥3 `_recall_ratio.txt` files each
  contain a float ≥ 0.95; Python assertion prints `pass: True`; gaps files present.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output + screenshot showing ≥3 corpora with ratio ≥ 0.95)
      attached to the PH70 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
