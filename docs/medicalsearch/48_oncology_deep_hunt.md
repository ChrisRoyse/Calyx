# #1185 Oncology Deep Association Hunt

## Scope

#1185 composes the current typed association-mining, falsification, and
precision-oncology validation surfaces into one oncology hypothesis atlas.

Inputs are persisted artifacts from #1180, #1183, and #1184. The output is a
ranked evidence bundle for downstream safety triage and external validation.
Rows are hypotheses only: not efficacy claims, safety claims, clinical
actionability, treatment recommendations, or cure evidence.

## Real FSV

Final FSV root:

```text
/home/croyse/calyx/fsv/issue1185-oncology-deep-hunt-20260704T024819Z
```

The first local run for this issue was discarded because cancer type
attribution used the non-cancer endpoint for two typed rows. The final FSV root
above reassigns cancer type from the cancer-side endpoint and is the only
#1185 result to use.

Readback summary:

| Field | Count |
|---|---:|
| Total candidates | 19 |
| CIViC precision-oncology candidates | 15 |
| Typed association candidates | 4 |
| Input manifest rows | 7 |

Cancer type counts:

| Cancer type | Candidates |
|---|---:|
| Meningioma | 5 |
| Skin Melanoma | 4 |
| Childhood Acute Lymphocytic Leukemia | 3 |
| Plexiform Neurofibroma | 2 |
| Breast Cancer | 1 |
| Cancer | 1 |
| Childhood Low-grade Glioma | 1 |
| Leukemia Lymphocytic Chronic B-Cell | 1 |
| Malignant Peripheral Nerve Sheath Tumor | 1 |

## Input Scope

| Role | Bytes | SHA-256 |
|---|---:|---|
| `civic_mapped_rows` | 13,589 | `b9891be3230b6dfa18f0fc3fba5ba130c3bbde66448b890517a8097c0779927d` |
| `civic_evidence_rows` | 3,506,213 | `bb3b6955275f71cf51af65d3541249bb06f4a7137689c323877af3c2946d2261` |
| `civic_summary` | 6,747 | `85baec997eda18c3ca81dcfa864e14b5cf5cdb8f650b141199a33a0a38e52e2e` |
| `falsification_flags` | 180,225 | `9d80c503a5173e8a3056101c132b1b299905e801a634d87aabf5bcab862e3e77` |
| `typed_miner_report` broad | 400,469 | `973d939cfd8f2aec8ac1ef218233078f59c5524de865284c64bc3e1e490c1c8c` |
| `typed_miner_report` chemical/disease | 60,014 | `5614f6fc1594e6eb7ad318364637d73d19bc7d8107b7ebe0891864f001bdc03f` |
| `typed_miner_report` gene/disease | 14,875 | `3532e0bc3e03b0d45469e5cf371f6dddc46e97799d2182cfa0024b03ee658bf3` |

Source paths are recorded in:

```text
/home/croyse/calyx/fsv/issue1185-oncology-deep-hunt-20260704T024819Z/out/input_scope.json
```

## Persisted Artifacts

| Artifact | Bytes | SHA-256 |
|---|---:|---|
| `out/input_scope.json` | 2,160 | `993e37c99bde36b58fd98e0c639ab8a9b301f8719bb868eaf3a8f1bef63d68c0` |
| `out/oncology_hypothesis_atlas.jsonl` | 23,618 | `455ea1b611e02b6d70d9a6dbbdb6af4faaa680a4521957b57f9166f8ee04c6e1` |
| `out/counts_by_cancer_type.json` | 283 | `eebc55ee2d18f915162d4e4218c14f21a5a60e693cdde5928ce603e511d6a454` |
| `out/top_candidate_evidence_bundles.json` | 27,661 | `9ac8d4ff34bd0f9c35fd0492bf078b578a7767142ce3fa750810101f56736ed4` |
| `out/run_summary.json` | 828 | `67e75622b975472744116996359fd222edd7f8a54bc550f56698eaf8e7585fd0` |
| `out/output_manifest.json` | 1,226 | `4bac77449e9b8382394dcac1a98b51406ad3fdd0cc2c15b9bc1f000e70b32c93` |
| `out/persisted_readback.json` | 1,833 | `e114f18c059f0594e43a007e08f5357f48b2111130e41f23c074474683d0fec8` |

Separate persisted readback confirmed:

| Field | Value |
|---|---|
| `status` | `ok` |
| `candidate_count` | `19` |
| `output_files` | `5` |
| `atlas_sha256` | `455ea1b611e02b6d70d9a6dbbdb6af4faaa680a4521957b57f9166f8ee04c6e1` |
| `manifest_sha256` | `4bac77449e9b8382394dcac1a98b51406ad3fdd0cc2c15b9bc1f000e70b32c93` |

## Top Candidates

| Candidate | Cancer type | Gene | Variant | Therapies | Rank | Evidence | Falsification status | Safety/trial flags |
|---|---|---|---|---|---:|---|---|---|
| `oncology-civic:11176` | Plexiform Neurofibroma | NF1 | Mutation | Selumetinib | 6.8 | A Predictive Sensitivity/Response | `complete_no_counterevidence_found_in_current_sources` | `safety_triage_pending_issue_1181`; `trial_ids_present_in_civic_row` |
| `oncology-civic:1958` | Plexiform Neurofibroma | NF1 | Mutation | Selumetinib | 6.6 | A Predictive Sensitivity/Response | `complete_no_counterevidence_found_in_current_sources` | `safety_triage_pending_issue_1181`; `trial_ids_present_in_civic_row` |
| `oncology-civic:10138` | Breast Cancer | IGF1R | Overexpression | Metformin; Exemestane | 5.7 | B Predictive | `not_in_1184_typed_pair_surface` | `safety_triage_pending_issue_1181` |
| `oncology-civic:7487` | Childhood Low-grade Glioma | NF1 | Mutation | Selumetinib | 5.6 | B Predictive | `complete_no_counterevidence_found_in_current_sources` | `safety_triage_pending_issue_1181`; `trial_ids_present_in_civic_row` |
| `oncology-civic:1053` | Meningioma | PTTG1 | OVEREXPRESSION | none | 5.1 | B Prognostic | `complete_no_counterevidence_found_in_current_sources` | none |
| `oncology-civic:1054` | Meningioma | LEPR | UNDEREXPRESSION | none | 5.1 | B Prognostic | `complete_no_counterevidence_found_in_current_sources` | none |
| `oncology-civic:1470` | Skin Melanoma | NF1 | Mutation | Vemurafenib | 4.5 | C Predictive | `complete_no_counterevidence_found_in_current_sources` | `safety_triage_pending_issue_1181` |
| `oncology-civic:1230` | Cancer | NRAS | Mutation | Metformin; Trametinib | 3.5 | D Predictive | `not_in_1184_typed_pair_surface` | `safety_triage_pending_issue_1181` |
| `oncology-civic:1469` | Skin Melanoma | NF1 | Mutation | Sirolimus; Mirdametinib | 3.3 | D Predictive | `complete_no_counterevidence_found_in_current_sources` | `safety_triage_pending_issue_1181` |
| `oncology-civic:1743` | Malignant Peripheral Nerve Sheath Tumor | NF1 | Loss | JQ1 Compound | 3.3 | D Predictive | `complete_no_counterevidence_found_in_current_sources` | `safety_triage_pending_issue_1181` |

Top candidate readback:

| Field | Value |
|---|---|
| Candidate | `oncology-civic:11176` |
| Source URL | `https://civicdb.org/links/evidence_items/11176` |
| Citation | Gross et al., 2020 |
| Trial id | `NCT01362803` |
| Mapped concept | `concept:ncbi_gene:4763` NF1 |
| Raw row SHA-256 | `385346078b2774619a46b813955b6e3eaf5907ecc7ff1f42ea0ebe563f5141fe` |
| Clinical boundary | Hypothesis only; not efficacy, safety, actionability, treatment recommendation, or cure evidence |

## Conclusion

#1185 is complete for this evidence-composition slice. It centralizes the
available oncology-specific candidates into one persisted atlas, with source
hashes, falsification status, cancer-type counts, and safety/trial flags.

The atlas is useful as a work queue for #1181 safety adjudication and future
external validation gates. It does not clear the Calyx clinical-actionability
bar because outcome sufficiency, safety, counter-evidence breadth, and clinical
review remain open gates.
