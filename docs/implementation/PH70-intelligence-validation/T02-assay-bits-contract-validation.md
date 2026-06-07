# PH70 · T02 — Assay bits/contract validation — labeled classification corpora

| Field | Value |
|---|---|
| **Phase** | PH70 — Intelligence validation on real corpora |
| **Stage** | S18 — Datasets & Intelligence FSV |
| **Crate** | `calyx-assay` |
| **Files** | `scripts/validate_assay_bits.sh` (≤500) |
| **Depends on** | PH69 T03 (AG News/banking77 verified); PH30 (panel sufficiency + attribution + reports) |
| **Axioms** | A2, A7, A8, A15 |
| **PRD** | `28 §2` (Loom/Assay row), `28 §3` row 2 |

## Goal

Prove the Assay intelligence claims on grounded labeled classification data
(AG News, banking77) on aiwonder: per-lens `bits_about` ≥0.05; planted-redundant
lens (corr > 0.6) rejected; `I(panel;anchor)` reported with CI; per-stratum bits
present. Read `bits_about` and `assay` CF rows on aiwonder — not return values.

## Build (checklist of concrete, code-level steps)

- [ ] `scripts/validate_assay_bits.sh`:
      (1) Ingest AG News (4 classes, ~127 K rows) or banking77 (77 classes, ~10 K
          rows) into an Aster vault; use class label as a grounded anchor.
      (2) Add ≥2 lenses including one that is intentionally redundant (corr > 0.6
          with another): record which lenses and their expected MI values.
      (3) Run `calyx assay abundance_report`; write output to
          `/zfs/hot/calyx/metrics/assay_abundance.json`.
      (4) Read `bits_about` CF rows: for each lens, assert `bits_about` ≥ 0.05;
          write per-lens values to `/zfs/hot/calyx/metrics/assay_bits_per_lens.txt`.
      (5) Assert the planted-redundant lens is REJECTED (absent from the admitted
          panel); write rejection record to
          `/zfs/hot/calyx/metrics/assay_rejection_log.txt`.
      (6) Assert `I(panel;anchor)` is present with CI bounds in
          `assay_abundance.json`; assert per-stratum bits present.
- [ ] Fail-closed: if any lens has `bits_about` < 0.05 unexpectedly (not the planted
      redundant one) → exits 1, `CALYX_FSV_ASSAY_BITS_BELOW_THRESHOLD`.
- [ ] Planted-redundant lens not rejected → exits 1,
      `CALYX_FSV_ASSAY_REDUNDANT_LENS_NOT_REJECTED`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: a synthetic 3-lens, 200-example fixture (seed=42) with one planted
      redundant lens (corr=0.75); assert `bits_about` computed, redundant lens
      flagged with `corr > 0.6`, and the rejection JSON field is set.
- [ ] proptest: property that any lens with `bits_about < 0.05` triggers rejection.
- [ ] edge (≥3):
      (1) all lenses below 0.05 bits → all rejected, panel empty → exits 1,
          `CALYX_FSV_PANEL_EMPTY`;
      (2) `n_eff` below quorum (n < 50) → Assay fails closed,
          `CALYX_ASSAY_BELOW_QUORUM`;
      (3) CI bounds span 0 (degenerate case, single class in a stratum) → CI
          reported as `[0,0]`, not a crash.
- [ ] fail-closed: ingest zero rows → exits 1, `CALYX_FSV_EMPTY_CORPUS`.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `/zfs/hot/calyx/metrics/assay_abundance.json`,
  `assay_bits_per_lens.txt`, `assay_rejection_log.txt` on aiwonder;
  `assay` CF rows in the Aster vault.
- **Readback:**
  ```
  cat /zfs/hot/calyx/metrics/assay_bits_per_lens.txt
  cat /zfs/hot/calyx/metrics/assay_rejection_log.txt
  python3 -c "import json; d=json.load(open('/zfs/hot/calyx/metrics/assay_abundance.json')); print('I(panel;anchor):',d['panel_mi'],'CI:',d['panel_mi_ci'])"
  calyx readback --cf assay --vault calyx_ph70_validation | head -20
  ```
- **Prove:** before: metric files absent; after: each `bits_about` value in
  `assay_bits_per_lens.txt` is ≥ 0.05 (for non-planted lenses); rejection log
  shows the planted-redundant lens rejected; `I(panel;anchor)` present with CI
  in the JSON; stratum bits present.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output + screenshot of bits_per_lens.txt and
      rejection_log.txt) attached to the PH70 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
