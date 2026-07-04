# #1226 Novelty / Calibration Split for Disease-Hunt Rankings

## Scope

#1226 re-ranks the persisted disease-hunt outputs from #1185, #1186, #1187,
and #1188 into two explicit views:

- a calibration / known-positive proof view, where rows carry explicit
  validation markers such as high-level CIViC evidence, Open Targets clinical
  or genetic validation scores, DGIdb clinical-trial source metadata, or a
  non-disease typed-pair marker;
- a novelty-prioritized research-lead view, where those calibration rows are
  excluded and remaining rows keep transparent score components.

This does not assert clinical novelty. It only removes evidence-marked
calibration rows from the novelty triage view while preserving them in a
separate proof view.

## Real FSV

Final FSV root:

```text
/home/croyse/calyx/fsv/issue1226-novelty-calibration-split-20260704T104049Z
```

Inputs:

| Source issue | Domain | Rows |
|---|---|---:|
| #1185 | oncology | 19 |
| #1186 | metabolic/cardiovascular/renal | 35 |
| #1187 | neurodegeneration/neuropsychiatric | 77 |
| #1188 | infectious/immunology/inflammation | 209 |

## Output Readback

| Artifact | Bytes | SHA-256 | Rows |
|---|---:|---|---:|
| `out/input_scope.json` | 1,896 | `64bee449eac0f952f09a6c7fd8bbcc1144f1d477d120535679bfe8e91769c71f` | - |
| `out/combined_original_ranked.jsonl` | 2,133,068 | `082c248bdc8686c96f0b2174c76b3fa33ebeed7b66a873f12db95ad87716c015` | 340 |
| `out/calibration_known_positive_rows.jsonl` | 1,134,154 | `8c7188342afc263f3dfede11896d002c7a560ee0942624316f96aac2cd8ec1ce` | 169 |
| `out/novelty_prioritized_research_leads.jsonl` | 998,914 | `97349007d843361d3630c673ccecd874483364670178acbdbaef63210495655a` | 171 |
| `out/before_after_topk.json` | 70,084 | `f4ce3fa5a6b9be04ff2b253b5ee0b418ab999f4a8fa48fe2dd870e74efeab1e7` | - |
| `out/manual_spot_checks.json` | 27,090 | `b3e42f7a6a34017e0185a84a9c4adc7be9821e6de2cf0bf0c20cc460a686ef37` | - |
| `out/validation_metrics.json` | 3,655 | `4127f3a89918c09511e537786106d76120bbefbf215f2adec39d937543f34509` | - |
| `out/persisted_readback.json` | - | `e5d55164369ac431f4666bdbaa00cd8b66619ab9acd3d9fe79589958c340fb9d` | - |

Readback assertions:

| Assertion | Value |
|---|---:|
| Combined rows read back | 340 |
| Calibration rows read back | 169 |
| Novelty rows read back | 171 |
| Total rows match metrics | true |
| Split rows sum to total | true |
| Before top-k rows | 25 |
| After top-k rows | 25 |
| Calibration top-k rows | 25 |
| Top after row is not calibration | true |
| Calibration rows available | true |
| Novelty rows available | true |

## Detector

The detector only routes a row to calibration / known-positive when the row
itself carries an explicit marker:

| Flag | Rows |
|---|---:|
| `open_targets_clinical_precedence_ge_0_75` | 89 |
| `open_targets_clinical_datatype_ge_0_75` | 89 |
| `non_disease_typed_pair_not_novelty_lead` | 47 |
| `open_targets_genetic_or_somatic_validation_ge_0_85` | 32 |
| `dgidb_clinical_source_plus_open_targets_context` | 17 |
| `civic_level_a_b_external_evidence` | 6 |
| `civic_level_a_b_trial_id_present` | 3 |

Rows are not demoted merely because they look familiar. If the current row does
not carry an explicit known-positive/calibration marker, it stays eligible for
the novelty-prioritized view and remains provisional.

## Metrics

| Metric | Count |
|---|---:|
| Total input rows | 340 |
| Calibration / known-positive rows | 169 |
| Novelty-prioritized rows | 171 |
| #1185 calibration rows | 6 |
| #1187 calibration rows | 21 |
| #1188 calibration rows | 142 |
| #1185 novelty rows | 13 |
| #1186 novelty rows | 35 |
| #1187 novelty rows | 56 |
| #1188 novelty rows | 67 |

## Before / After

Before: the combined original-rank top rows were dominated by known proof rows:

| Original view | Calibration? | Candidate | Flag |
|---:|---|---|---|
| 1 | yes | NF1 / Selumetinib / Plexiform Neurofibroma | CIViC level A/B + trial id |
| 2 | no | TNF / Proteinuria | none |
| 3 | yes | NF1 / neurofibromatosis type 1 | Open Targets genetic validation |
| 4 | yes | Golimumab / TNF / psoriatic arthritis | Open Targets clinical + DGIdb clinical source |
| 5 | yes | Certolizumab Pegol / TNF / psoriatic arthritis | Open Targets clinical + DGIdb clinical source |
| 6 | yes | Tregalizumab / CD4 / HIV infectious disease | Open Targets clinical + DGIdb clinical source |

After: the novelty-prioritized top rows exclude calibration/proof rows:

| Novelty rank | Candidate | Source issue | Score | Notes |
|---:|---|---|---:|---|
| 1 | TNF / Proteinuria | #1186 | 0.745 | target-disease lead, not externally marked calibration |
| 2 | Proteinuria / Diffuse Neurofibrillary Tangles with Calcification | #1187 | 0.730526316 | disease-neuro cluster |
| 3 | CD4 / Proteinuria | #1186 | 0.723823529 | target-disease lead |
| 4 | TNF / Proteinuria | #1188 | 0.716153846 | target-disease lead from infectious/immunology run |
| 5 | CD8A / Proteinuria | #1186 | 0.702647059 | target-disease lead |
| 6 | CD4 / Proteinuria | #1188 | 0.698846154 | target-disease lead from infectious/immunology run |
| 7 | Alogliptin / DPP4 / Type 2 Diabetes Mellitus | #1186 | 0.672941176 | drug-target-disease row; still provisional |
| 10 | Saxagliptin Anhydrous / DPP4 / schizophrenia | #1187 | 0.641578947 | drug-target-disease bridge; still provisional |
| 11 | Meningitis Bacterial / Subarachnoid Hemorrhage | #1187 | 0.635789474 | disease association |
| 12 | Nifedipine / SLC14A2 / Hypertension | #1186 | 0.635294118 | drug-target-disease row; still provisional |

The known-positive proof rows are not hidden. They remain in
`out/calibration_known_positive_rows.jsonl` and in the calibration top-k view.

## Spot Checks

| Check | Result |
|---|---|
| TNF / psoriatic arthritis | marked calibration when Open Targets clinical scores and DGIdb clinical sources are present |
| CD4 / HIV infectious disease | marked calibration for Open Targets clinical + DGIdb clinical source rows |
| CIViC NF1 / Selumetinib | marked calibration for level A/B CIViC rows with trial ids |
| Streptomycin / Klebsiella or Rhinoscleroma | remains in novelty view because current rows lack explicit known-positive calibration markers |
| DPP4 / schizophrenia | remains in novelty view when it is a weak bridge rather than an evidence-marked known-positive row |

## Follow-Up

#1227 tracks productionizing this detector as a native Calyx discovery/ranking
stage that consumes sealed discovery-run manifests and writes ledger-sealed
split outputs. #1226 is intentionally a bounded FSV artifact over current
disease-hunt outputs.

## Conclusion

#1226 is complete for the bounded disease-hunt atlas split:

- all 340 rows from #1185/#1186/#1187/#1188 were preserved;
- 169 rows were routed to the calibration / known-positive proof view;
- 171 rows were routed to the novelty-prioritized research-lead view;
- before/after top-k comparisons and manual spot checks were persisted;
- separate readback proved row conservation and artifact hashes.

No clinical novelty, recommendation, treatment claim, safety claim,
actionability claim, efficacy claim, or cure claim is made.
