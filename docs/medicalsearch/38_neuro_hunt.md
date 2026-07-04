# #1187 Neurodegeneration / Neuropsychiatric Association Hunt

## Scope

#1187 composes the current Calyx biomedical association substrate into a bounded
neurodegeneration/neuropsychiatric evidence pack. The run uses:

- #1183 typed all-pair hypotheses.
- #1184 falsification flags for the original typed hypotheses.
- #1171 CxId source expansion and #1172/#1173 concept normalization/typed overlay.
- #1174 Open Targets target-disease validation rows.
- #1178 DGIdb drug-gene rows.
- #884/#994/#1175 clinical/molecular bridge rows for metformin/DPP4.
- Live bounded ClinicalTrials.gov and openFDA probes for selected drug-bearing
  rows.

The output is a ranked research-lead atlas. It is not a treatment, actionability,
efficacy, safety, or cure claim.

## Real FSV

Final FSV root:

```text
/home/croyse/calyx/fsv/issue1187-neuro-hunt-20260704T101459Z
```

Live query sources used during this FSV:

- ClinicalTrials.gov Data API: `https://clinicaltrials.gov/api/v2/studies`
- openFDA drug label API: `https://api.fda.gov/drug/label.json`
- openFDA drug adverse event API: `https://api.fda.gov/drug/event.json`

## Output Readback

| Artifact | Bytes | SHA-256 | Rows |
|---|---:|---|---:|
| `out/input_scope.json` | 5,500 | `24b1d68c9cce9b8ee3140aa5a47d45d7563eaef4708508da15f773412a97b526` | - |
| `out/neuro_normalized_annotations.jsonl` | 192,126 | `af99b6c41539374363d5f6fb0664a4c91205caba8cd0f740bfae0fe446816a74` | 222 |
| `out/neuro_unresolved_terms.jsonl` | 30,850 | `0af0314384a4da16b897a5c3a69d5a6835d1001ce2c5f69a19f38a2ebbe68462` | 70 |
| `out/neuro_hypotheses.jsonl` | 362,520 | `3d782f891933797a3ec096e693e38570cd93d386414f6d19f6fdac2f48a601b8` | 77 |
| `out/neuro_hypotheses.json` | 518,981 | `b27f2b2e9fd67f4c2e1e631d2d8e79ef22d5e6ae64912c25bee9a97f55eb6f5b` | - |
| `out/top_evidence_bundles.json` | 87,680 | `58f65ef7ce9c7a163d92c1d6ebde7dbe1d1853f9712948b00c28330844210545` | 20 bundles |
| `out/external_validation_context.jsonl` | 39,919 | `b1944c4391ad440c953a0e6baa99f89f3dff0ad17e96644b0f3837a86d76d83b` | - |
| `out/raw_query_manifest.jsonl` | 20,358 | `6e5cf2ea4cac30fcfffd77fbff4ca60f2fa59f8ed4ab8fdf101491f0466996f9` | 42 |
| `out/safety_trial_flags.jsonl` | 24,113 | `eb68c9ecd77282cc3e6f47c619349d4e003014b19ef0e22fb1a3d45da2ab9610` | 14 |
| `out/validation_metrics.json` | 1,323 | `0dd0d48c71c15e69a373624ef7422e0e978f94b88b56b70ccff5ebdfe1a62d96` | - |
| `out/persisted_readback.json` | - | `9dfa536723b86435a0ff2af21e2af4ad87ecbb9aecf06e15f42ee2609981b43b` | - |

Readback assertions:

| Assertion | Value |
|---|---:|
| Hypothesis rows read back | 77 |
| Metrics hypothesis total | 77 |
| Row count matches metrics | true |
| Top bundle count | 20 |
| Raw live query rows | 42 |
| Safety/trial flag rows | 14 |
| Normalized neuro annotation rows | 222 |
| Unresolved neuro rows | 70 |

## Metrics

| Metric | Count |
|---|---:|
| Typed rows scanned | 301 |
| Falsification flags loaded | 280 |
| Source-expanded rows loaded | 2,612 |
| Normalized annotations loaded | 2,575 |
| Normalized neuro annotations emitted | 222 |
| Unresolved neuro terms emitted | 70 |
| Open Targets rows loaded | 1,422 |
| Open Targets neuro rows considered | 60 |
| DGIdb interactions loaded | 359 |
| DPP4 interactions loaded | 41 |
| Total ranked hypotheses | 77 |
| Drug-bearing hypotheses | 23 |
| Live drug pair triage queries | 14 |
| Live API artifacts | 42 |

Hypothesis class counts:

| Class | Rows |
|---|---:|
| Open Targets target-neuro disease rows | 30 |
| Typed all-pair neuro-filter rows | 19 |
| DGIdb drug-target/Open Targets neuropsychiatric bridges | 12 |
| Same-source disease-to-neuro disease clusters | 11 |
| Same-source drug/gene/variant-to-neuro disease co-mentions | 4 |
| Clinical/molecular metformin-DPP4-schizophrenia bridge | 1 |

## Top Readback Rows

| Rank | Candidate | Type | Source | Bridge | Target | Score | Evidence |
|---:|---|---|---|---|---|---:|---|
| 1 | `issue1187:opentargets_target_neuro:03088f07d62fb884ac77` | target-disease | NF1 | - | neurofibromatosis type 1 | 0.953413184 | Open Targets |
| 2 | `issue1187:disease_neuro_cluster:259cdcce4807d7a52fde` | disease-neuro cluster | Proteinuria | - | Diffuse Neurofibrillary Tangles with Calcification | 0.95 | source-expanded normalized co-mentions |
| 3 | `issue1187:opentargets_target_neuro:1ef096aa80d38fbf2dc8` | target-disease | NF1 | - | neurofibromatosis-Noonan syndrome | 0.919166479 | Open Targets |
| 4 | `issue1187:opentargets_target_neuro:de90f09ee7af73f6267c` | target-disease | KIF11 | - | microcephaly | 0.903947407 | Open Targets |
| 5 | `issue1187:opentargets_target_neuro:a129f458c7674c4a4146` | target-disease | NF1 | - | neurofibromatosis | 0.902600114 | Open Targets |
| 6 | `issue1187:opentargets_target_neuro:857be828412ccb2587ed` | target-disease | PCNT | - | microcephaly | 0.902032109 | Open Targets |
| 7 | `issue1187:opentargets_target_neuro:ce2dd05db8308fc92f7c` | target-disease | WDR62 | - | microcephaly | 0.899856982 | Open Targets |
| 8 | `issue1187:opentargets_target_neuro:c6f8a60228f1e781557e` | target-disease | MCPH1 | - | microcephaly | 0.898600761 | Open Targets |
| 9 | `issue1187:opentargets_target_neuro:b839e5846eea84a6d0a1` | target-disease | CDK5RAP2 | - | microcephaly | 0.89210711 | Open Targets |
| 10 | `issue1187:opentargets_target_neuro:1bb3ac171377a794830a` | target-disease | NDE1 | - | microcephaly | 0.891178722 | Open Targets |
| 12 | `issue1187:disease_neuro_cluster:057d66318046c6764d5c` | disease-neuro cluster | Meningitis Bacterial | - | Subarachnoid Hemorrhage | 0.888351894 | 5 source CxIds |
| 23 | `issue1187:dpp4_inhibitor_schizophrenia:saxagliptin_anhydrous` | drug-target-disease bridge | Saxagliptin Anhydrous | DPP4 | schizophrenia | 0.874142023 | DGIdb + Open Targets + live safety/trial |
| 34 | `issue1187:molecular_bridge:metformin_dpp4_schizophrenia` | drug-target-disease bridge | Metformin | DPP4 | schizophrenia | 0.834142023 | #884/#994/#1175 + Open Targets + live safety/trial |
| 41 | `issue1187:typed:typed-assoc:concept:ncbi_mesh:D013345::concept:ncbi_mesh:D016920` | typed disease association | Subarachnoid Hemorrhage | - | Meningitis Bacterial | 0.782106343 | #1183 typed paths + #1184 status |
| 42 | `issue1187:typed:typed-assoc:concept:ncbi_mesh:D014012::concept:ncbi_mesh:D014717` | typed disease association | Tinnitus | - | Vertigo | 0.782106343 | #1183 typed paths + #1184 status |
| 43 | `issue1187:typed:typed-assoc:concept:ncbi_mesh:D010040::concept:ncbi_mesh:D014012` | typed disease association | Otosclerosis | - | Tinnitus | 0.727961544 | #1183 typed paths + #1184 status |
| 44 | `issue1187:typed:typed-assoc:concept:ncbi_mesh:D011507::concept:ncbi_mesh:D055956` | typed disease association | Proteinuria | - | Diffuse Neurofibrillary Tangles with Calcification | 0.727961544 | #1183 typed paths + #1184 status |

The top global ranks are dominated by target-disease rows from Open Targets and
known neurogenetic/microcephaly signals. These are useful prioritization rows,
not new treatment claims.

## Drug-Bearing Readback

| Rank | Candidate | Drug | Bridge | Disease | Trial/safety status | Notes |
|---:|---|---|---|---|---|---|
| 23 | `issue1187:dpp4_inhibitor_schizophrenia:saxagliptin_anhydrous` | Saxagliptin Anhydrous | DPP4 | schizophrenia | live triage complete | DGIdb DPP4 interaction + Open Targets DPP4-schizophrenia |
| 34 | `issue1187:molecular_bridge:metformin_dpp4_schizophrenia` | Metformin | DPP4 | schizophrenia | live triage complete | BindingDB/DPP4 bridge + Open Targets score 0.1921893635 |
| 36 | `issue1187:dpp4_inhibitor_schizophrenia:anagliptin` | Anagliptin | DPP4 | schizophrenia | live triage complete | DGIdb DPP4 bridge |
| 37 | `issue1187:dpp4_inhibitor_schizophrenia:bisegliptin` | Bisegliptin | DPP4 | schizophrenia | live triage complete | no openFDA label/event hit in bounded query |
| 38 | `issue1187:dpp4_inhibitor_schizophrenia:omarigliptin` | Omarigliptin | DPP4 | schizophrenia | live triage complete | no openFDA label hit; event query had 31 rows |
| 39 | `issue1187:dpp4_inhibitor_schizophrenia:prusogliptin` | Prusogliptin | DPP4 | schizophrenia | live triage complete | no openFDA label/event hit in bounded query |
| 40 | `issue1187:dpp4_inhibitor_schizophrenia:valacyclovir` | Valacyclovir | DPP4 | schizophrenia | live triage complete | DGIdb row exists but bridge is mechanistically ambiguous |
| 54 | `issue1187:normalized_comention:833636615f7503375d50` | Quinidine | - | Tinnitus | live triage complete | source co-mention; not causality |
| 56 | `issue1187:normalized_comention:17b6c746f037502ed3cb` | Kanamycin | - | Tinnitus | pending broader triage | source co-mention; not causality |
| 57 | `issue1187:normalized_comention:bb3f9dfbe7f9d6830d9b` | Streptomycin | - | Tinnitus | pending broader triage | source co-mention; not causality |
| 59 | `issue1187:normalized_comention:b794231dd43a99a3a80e` | Phenytoin | - | Tinnitus | pending broader triage | source co-mention; not causality |

Live triage examples:

- Metformin/schizophrenia ClinicalTrials query returned 10 first-page studies,
  including schizophrenia/metabolic-syndrome contexts. openFDA label query
  returned 5 label rows and an event query total of 425,794. This is safety/trial
  context, not efficacy.
- Quinidine/tinnitus live triage returned 0 ClinicalTrials hits, 5 openFDA label
  rows, and 4,273 event rows in the bounded query.

## Falsification Status

- #1183 typed rows carry #1184 falsification status where the hypothesis id was
  present.
- Generated #1187 rows created after #1184 are marked explicitly as
  `not_run_for_generated_*` or `external_validation_row_not_falsification_sweep`.
- This is not hidden: #1223 now tracks a full counter-evidence sweep for
  generated disease-hunt candidates before atlas promotion.

## Gaps Split Out

The FSV found real follow-up work:

- #1222 - repair unresolved neuro concept normalization after #1187.
- #1223 - falsify generated disease-hunt candidates across domains.
- #1224 - expand neuropsychiatric target druggability evidence beyond the
  bounded DPP4 slice.

## Conclusion

#1187 is complete for a bounded, persisted neurodegeneration/neuropsychiatric
association hunt:

- it produced 77 ranked rows with normalized names where available, evidence
  paths, validation context, safety/trial flags for selected drug-bearing rows,
  and explicit falsification status;
- it read back all persisted output counts and hashes from aiwonder artifacts;
- it surfaced DPP4/metformin/schizophrenia as a weak/moderate molecular bridge
  research lead, not an efficacy or cure claim;
- it surfaced known neurogenetic target-disease rows and source-backed
  disease/drug co-mentions as prioritization worklist items only.

No clinical recommendation, treatment claim, safety claim, actionability claim,
or cure claim is made by this artifact.
