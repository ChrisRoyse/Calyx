# #1193 Human-Review Biomedical Hypothesis Atlas

## Scope

#1193 publishes a human-review atlas over the generated biomedical discovery
rows from #1185, #1186, #1187/#1222, #1188, and #1189. The atlas overlays
novelty/calibration state from #1227, generated-candidate falsification state
from #1223, support/counter evidence, safety/trial flags, and source hashes.

This is an inspectable research-review surface only. Every row is
hypothesis-only. No row is efficacy, safety, clinical actionability, treatment
guidance, dosing, recommendation, or cure evidence.

## Implementation

Script:

```text
scripts/medicalsearch/issue1193_biomedical_hypothesis_atlas.py
```

The script emits:

- a normalized JSONL atlas for machine review;
- a TSV atlas for human scanning/filtering;
- filter facets by disease area, drug, target, pathway, evidence type, review
  status, source issue, and falsification status;
- top evidence bundles with source snippets, typed evidence path kinds,
  validation summaries, support/counter examples, hashes, and next validation
  experiments;
- a 1,000-row bridge-corpus slice for native Calyx materialization;
- output manifest, metrics, and persisted readback.

## Real FSV

Final FSV root:

```text
/home/croyse/calyx/fsv/issue1193-human-review-atlas-20260704T124751Z
```

Primary readbacks:

```text
/home/croyse/calyx/fsv/issue1193-human-review-atlas-20260704T124751Z/out/persisted_readback.json
sha256: 01cc9b112372d50d0bfd7c71ea88661c86826c071911d63d9f7109f7acbb248d

/home/croyse/calyx/fsv/issue1193-human-review-atlas-20260704T124751Z/out/calyx_bridge_corpus_readback.json
sha256: e164923f418e878a445fba5d4d910bed7f95d29d1f203e720bc10f0f1446a071
```

Native Calyx materialization:

```text
name: issue1193-human-review-atlas-20260704t124751z
vault_id: 01KWPJR0ADVZF580HNRBZ17CBZ
vault_dir: /home/croyse/calyx/vaults/01KWPJR0ADVZF580HNRBZ17CBZ
```

Materialization readback:

| Item | Count / value |
|---|---:|
| Bridge rows | 1,000 |
| Bridge terms | 1,365 |
| Graph nodes | 2,365 |
| Graph edges | 10,280 |
| CSR persisted | true |
| Active vault index contains name | true |
| Graph nodes match readback | true |
| Graph edges match readback | true |
| Vault manifest/CURRENT files present | true |

## Inputs

| Input | Rows | SHA-256 |
|---|---:|---|
| #1185 oncology hypotheses | 19 | `455ea1b611e02b6d70d9a6dbbdb6af4faaa680a4521957b57f9166f8ee04c6e1` |
| #1186 metabolic/cardiovascular hypotheses | 35 | `1a53daf6d93b2ce2f0d28235b679a7cd1427c0a44070285840365c6397f7eb94` |
| #1187/#1222 repaired neuro hypotheses | 131 | `9c23eb5e6ed726c5a3003fdbbd6edbb24b315ac63e919bd8a8f5c3c9b00e5bf6` |
| #1188 infectious/immunology hypotheses | 209 | `de2e6eda7c8aabcb39a8644c9948923cf1b93ddf2c0776372cdbcf7f4e65b61d` |
| #1189 rare-disease hypotheses | 1,491 | `c15fa5ac6b5e41f32a9a7a3fe184b8de2d639005642a723bf106a85a8ff36bfa` |
| #1223 falsification flags | 1,877 | `b3ec172c5f83caa86968ed74de9b10682d2af008b309713e16b57f7137d9aff0` |
| #1223 support evidence | 4,926 | `09fc9e3df16b7f8bc111fe3c23e00d9d20b27bee75959799266535912a7b09e5` |
| #1223 counter evidence | 2,151 | `bd043b638cb048a5be78f22fbd029e32dad0d8e7d2b90909a707df47ecffda94` |
| #1227 novelty combined view | 340 | `b283207096c22bd710b5002d15f49ed00662c47300fa9f7ab3f5949136462497` |
| #1227 calibration view | 169 | `44be636712d297fdd775b750615860255523ee25076e191c1e02802d73da0741` |
| #1227 novelty leads view | 171 | `731dea0569f99fc7afa3760a663c885a94c1d89bd68f384c0fb02ea3ffdb3815` |
| #1181 safety flags | 13 | `862b83ad7d03f8288916e0323445269ca232247baafd2669fa9d387ea06cba80` |
| #1177 ClinicalTrials rows | 269 | `ee845c959cb4d9144b4ab38d37e76411a7c8e6c7899c688b3d038fb987533663` |
| #1174 Open Targets rows | 1,422 | `6ceb368538c52b87cbb1fb661c661b7009a3b03f81be5f1b39f42f470e5b0ba6` |

## Output Artifacts

| Artifact | Rows | SHA-256 |
|---|---:|---|
| `human_review_biomedical_hypothesis_atlas.jsonl` | 1,877 | `766cebc55e4fb6c49672bf09b2a5e02ebbd81da7dfdbb14a8766e4d66aa8098e` |
| `human_review_biomedical_hypothesis_atlas.tsv` | - | `c918bb5c2992a402e2995b6ac43e3d52f01635d67fe037911e297f1d2530c9cf` |
| `atlas_filters.json` | - | `4489d5ac3cd1bdedf5396b252d43bac1205cd87c54b46276bf411fc83bcb653a` |
| `top_evidence_bundles.json` | - | `4c4587d941398bf3135cb63d7c0f25fa138a42cf36b71a3d48754c240730ade3` |
| `atlas_bridge_rows.jsonl` | 1,000 | `f4f1f449738f9862b0a6f2058b401203596b2a79c1bbfcef0853c3f3a6c920aa` |
| `input_manifest.json` | - | `f8e7de53fe5f5953f7b40a65345d5995d7c71eb702d432b96141a05eba0bb428` |
| `output_manifest.json` | - | `b6c9da262316b7c60b8de54bafbbcc709b5b20c8e94f3b7839db8a9468b4bed9` |
| `validation_metrics.json` | - | `6589a2f7dff2730f72b847305946beee8fe0c40c14fa6259535a2393e2b4a066` |
| `persisted_readback.json` | - | `01cc9b112372d50d0bfd7c71ea88661c86826c071911d63d9f7109f7acbb248d` |
| `calyx_bridge_corpus_readback.json` | - | `e164923f418e878a445fba5d4d910bed7f95d29d1f203e720bc10f0f1446a071` |

## Metrics

| Metric | Count |
|---|---:|
| Raw input source rows | 1,885 |
| Deduped atlas rows | 1,877 |
| Missing falsification flags | 0 |
| Hypothesis-only rows | 1,877 |
| Rows with normalized hypothesis | 1,877 |
| Rows with source snippets | 1,877 |
| Rows with support evidence | 1,877 |
| Rows with counter evidence | 1,082 |
| Rows with validation evidence | 1,869 |
| Rows with disease context | 1,877 |
| Rows with target context | 1,681 |
| Rows with drug context | 1,044 |

Review status:

| Status | Count |
|---|---:|
| `blocked_or_demoted_before_human_review` | 1,082 |
| `ready_for_hypothesis_review` | 726 |
| `calibration_known_positive_reference` | 69 |

Disease area:

| Area | Count |
|---|---:|
| rare disease | 1,491 |
| infectious/immunology/inflammation | 203 |
| neurodegeneration/neuropsychiatric | 129 |
| metabolic/cardiovascular/renal | 35 |
| oncology | 19 |

## Readback Assertions

| Assertion | Value |
|---|---:|
| Deduped rows match falsification flags | true |
| Atlas JSONL rows match | true |
| Bridge rows <= 1,000 | true |
| All rows hypothesis-only | true |
| All rows have clinical boundary | true |
| All rows have normalized hypothesis | true |
| All rows have source snippet | true |
| All rows have review status | true |
| Filters present | true |
| Top evidence bundles present | true |
| Ready rows present | true |
| Blocked rows present | true |

## Top Ready-for-Review Rows

These are research-review rows only. They are not treatment suggestions.

| Atlas rank | Candidate | Area | Target(s) | Drug(s) | Disease/context | Confidence score | Novelty score | Next validation |
|---:|---|---|---|---|---|---:|---:|---|
| 1 | `typed-assoc:concept:ncbi_gene:24835::concept:ncbi_mesh:D011507` | metabolic/cardiovascular/renal | Tnf | - | Proteinuria | 0.555000 | 0.745000 | Validate target-disease association in an outcome-backed assay/model before drug inference |
| 2 | `issue1187:disease_neuro_cluster:259cdcce4807d7a52fde` | neurodegeneration/neuropsychiatric | - | - | Diffuse Neurofibrillary Tangles with Calcification; Proteinuria | 0.435773 | 0.730526 | Human reviewer should inspect evidence bundle and define the next grounded outcome instrument |
| 3 | `typed-assoc:concept:ncbi_gene:920::concept:ncbi_mesh:D011507` | metabolic/cardiovascular/renal | CD4 | - | Proteinuria | 0.545286 | 0.723824 | Validate target-disease association in an outcome-backed assay/model before drug inference |
| 4 | `typed-assoc:concept:ncbi_gene:925::concept:ncbi_mesh:D011507` | metabolic/cardiovascular/renal | CD8A | - | Proteinuria | 0.535571 | 0.702647 | Validate target-disease association in an outcome-backed assay/model before drug inference |
| 5 | `issue1187:disease_neuro_cluster:057d66318046c6764d5c` | neurodegeneration/neuropsychiatric | - | - | Subarachnoid Hemorrhage; Meningitis Bacterial | 0.399437 | 0.635789 | Human reviewer should inspect evidence bundle and define the next grounded outcome instrument |

## Findings

- The atlas consolidates all five current disease-hunt families into one
  review surface: 1,885 input rows became 1,877 deduped atlas rows.
- Every atlas row now has an explicit normalized hypothesis, source snippet,
  review status, support evidence overlay, source hash, clinical boundary, and
  next validation experiment.
- #1223 falsification is now enforced as the promotion gate: 1,082 rows are
  blocked or demoted before human review, and none are missing falsification
  flags.
- The atlas separates calibration/proof rows from novel research leads. The
  69 calibration rows are references for gate health, not novelty claims.
- The 1,000-row atlas bridge slice is materialized into native Calyx vault
  `01KWPJR0ADVZF580HNRBZ17CBZ`, so the review surface is also in the Calyx DB.

## Conclusion

#1193 is complete for the current human-review biomedical hypothesis atlas. The
atlas is persisted, filterable, source-hashed, falsification-aware, and
materialized into Calyx.

No treatment claim, efficacy claim, safety claim, clinical-actionability claim,
recommendation, dosing guidance, or cure claim is made.
