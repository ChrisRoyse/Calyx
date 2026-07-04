# #1223 Generated Candidate Falsification Sweep

## Scope

#1223 runs a fail-closed falsification sweep across the generated disease-hunt
candidate outputs from #1185, #1186, #1187/#1222, #1188, and #1189. It
normalizes one candidate row per hypothesis, joins persisted support and
counter-evidence from the source-validation program, writes one falsification
flag per candidate, and materializes the top falsification rows into a native
Calyx bridge-corpus vault.

This is a triage and demotion instrument only. It does not establish efficacy,
safety, clinical actionability, treatment guidance, dosing, recommendation, or
cure evidence.

## Implementation

Script:

```text
scripts/medicalsearch/issue1223_generated_candidate_falsification.py
```

The script:

- consumes generated candidate JSONL rows from #1185, #1186, #1187/#1222,
  #1188, and #1189;
- uses persisted PubTator/PubMed, ClinicalTrials.gov, DGIdb, Open Targets, and
  drug-safety artifacts as the evidence instruments;
- emits support evidence, counter evidence, one falsification flag per deduped
  generated candidate, top demoted rows, and an output manifest;
- fails closed when required drug-safety or drug-disease trial evidence is
  absent for drug-bearing candidates;
- writes a 1,000-row bridge-corpus slice for native Calyx materialization.

Important normalization fix: drug and disease endpoint strings are not promoted
to gene symbols. Gene/target extraction is limited to explicit gene/target
fields, preventing drug names such as `Kanamycin` or disease labels such as
`Tinnitus` from becoming target symbols.

## Real FSV

Final FSV root:

```text
/home/croyse/calyx/fsv/issue1223-generated-candidate-falsification-20260704T121310Z
```

Primary readbacks:

```text
/home/croyse/calyx/fsv/issue1223-generated-candidate-falsification-20260704T121310Z/out/persisted_readback.json
sha256: 4495e768c412d321c32fdae3f8051dec17dcb2ace436eed7d0e18d846bab9704

/home/croyse/calyx/fsv/issue1223-generated-candidate-falsification-20260704T121310Z/out/calyx_bridge_corpus_readback.json
sha256: 184395b83aa7669960bc3bd904755e604860024b4dbf8ea73b651c7e1474e80e
```

Native Calyx materialization:

```text
name: issue1223-generated-falsification-20260704t121310z
vault_id: 01KWPGYV128AQVM7BMKW3AB6KY
vault_dir: /home/croyse/calyx/vaults/01KWPGYV128AQVM7BMKW3AB6KY
stdout_sha256: 5bb5af08f038d931c8a70346451a254dbba73bfd52df7f8a3d2cfa1b36729c97
stderr_sha256: dc5c2f7264f053d13bf68648d1ac9ae78057ef0115e8645fa0460832e669fb4f
```

Materialization readback assertions:

| Assertion | Value |
|---|---:|
| Row count is 1,000 | true |
| CSR persisted | true |
| Graph nodes match readback | true |
| Graph edges match readback | true |
| Active vault index contains name | true |
| Vault manifest/CURRENT files present | true |
| Graph SST files present | true |
| Time-index SST files present | true |

Native vault counts:

| Item | Count / bytes |
|---|---:|
| Bridge-corpus rows | 1,000 |
| Bridge terms | 625 |
| Graph nodes | 1,625 |
| Graph edges | 8,000 |
| `cf/graph` SST files | 9,628 |
| `cf/graph` SST bytes | 8,741,178 |
| `cf/time_index` SST files | 9,628 |
| `cf/time_index` SST bytes | 1,669,406 |

## Input Candidate Hashes

| Input | Rows | SHA-256 |
|---|---:|---|
| #1185 oncology hypotheses | 19 | `455ea1b611e02b6d70d9a6dbbdb6af4faaa680a4521957b57f9166f8ee04c6e1` |
| #1186 metabolic/cardiovascular hypotheses | 35 | `1a53daf6d93b2ce2f0d28235b679a7cd1427c0a44070285840365c6397f7eb94` |
| #1187/#1222 repaired neuro hypotheses | 131 | `9c23eb5e6ed726c5a3003fdbbd6edbb24b315ac63e919bd8a8f5c3c9b00e5bf6` |
| #1188 infectious/immunology hypotheses | 209 | `de2e6eda7c8aabcb39a8644c9948923cf1b93ddf2c0776372cdbcf7f4e65b61d` |
| #1189 rare-disease hypotheses | 1,491 | `c15fa5ac6b5e41f32a9a7a3fe184b8de2d639005642a723bf106a85a8ff36bfa` |

## Evidence Source Hashes

| Source | SHA-256 |
|---|---|
| PubTator supporting literature | `bf473c33e99f596411116b8fb4a165ca1dd893a73399d552efa8979689ad9cb0` |
| PubTator negative/contradicting literature | `2ded353b125e85436a6fad4d431c61f760bb4aa2de2112ac7a68acec4002dd08` |
| ClinicalTrials.gov trial rows | `ee845c959cb4d9144b4ab38d37e76411a7c8e6c7899c688b3d038fb987533663` |
| ClinicalTrials.gov seed summaries | `00d7be7f73876ade7158350c1ff08b0d377a67bd8ef8e98e035095276caca2e3` |
| DGIdb seed GraphQL interactions | `1ab68b172f383c93b3d4308b143e0028322b800551ab39daad4e8984bee36994` |
| DGIdb broad GraphQL interactions | `8b205f69a58d76b906909b3966b721b607503f574b84e48560f31dc31d816b67` |
| Open Targets association rows | `6ceb368538c52b87cbb1fb661c661b7009a3b03f81be5f1b39f42f470e5b0ba6` |
| Open Targets validation edges | `824ee4f86c0a408593f5f5378f501d9422e1e1c426a649ab80017a4cb1a23869` |
| #1181 candidate safety flags | `862b83ad7d03f8288916e0323445269ca232247baafd2669fa9d387ea06cba80` |
| #1181 safety terms | `a3840c545dc1c8efdf5a23fe944385147045640c2ba0561ff919c58ac0fa20f4` |
| #1224 DGIdb target interactions | `c7ca7ae7104fb68738238390d2eb924e28b84b5191f53de7f4fba0e93028d091` |
| #1224 Open Targets context rows | `974ddf9fa4cbdaccdf398d454e7371dcd452c7e6fccd168fa134822f84bb6897` |
| #1189 DGIdb target interactions | `7d47f24803d10feb025d6c240cd53de813036158226686c4f609a4ee15384297` |

## Output Metrics

| Metric | Count |
|---|---:|
| Input candidate rows | 1,885 |
| Deduped candidate rows | 1,877 |
| Falsification flag rows | 1,877 |
| Support evidence rows | 4,926 |
| Counter-evidence rows | 2,151 |
| Blocked or demoted rows | 1,082 |
| Rows missing required evidence | 1,066 |
| Rows with hard counterevidence | 16 |

Status counts:

| Status | Count |
|---|---:|
| `blocked_missing_required_evidence_or_safety` | 1,066 |
| `complete_no_counterevidence_found_in_current_sources` | 795 |
| `demoted_counterevidence_found` | 16 |

Reason-code counts:

| Reason code | Count |
|---|---:|
| `trial_source_missing_for_drug_disease` | 1,028 |
| `safety_source_missing_fail_closed` | 991 |
| `safety_block_or_high_risk_label` | 90 |
| `dgidb_exact_drug_gene_missing_current_sources` | 14 |
| `embedded_safety_or_trial_gap` | 13 |
| `clinicaltrials_stopped_trial` | 1 |
| `existing_counterevidence_status` | 1 |
| `no_counter_evidence_found_in_current_sources` | 795 |

Input contribution after dedupe:

| Source | Rows |
|---|---:|
| `1185_oncology` | 19 |
| `1186_metabolic_cardiovascular` | 35 |
| `1187_neuro_repaired` | 129 |
| `1188_infectious_immunology` | 203 |
| `1189_rare_disease` | 1,491 |

## Artifact Readback

| Artifact | Rows | SHA-256 |
|---|---:|---|
| `normalized_generated_candidates.jsonl` | 1,877 | `aa64d6027445023d3a94838984406f3178e6f1c1afd30a4a82a0f8c1392bf914` |
| `support_evidence.jsonl` | 4,926 | `09fc9e3df16b7f8bc111fe3c23e00d9d20b27bee75959799266535912a7b09e5` |
| `counter_evidence.jsonl` | 2,151 | `bd043b638cb048a5be78f22fbd029e32dad0d8e7d2b90909a707df47ecffda94` |
| `candidate_falsification_flags.jsonl` | 1,877 | `b3ec172c5f83caa86968ed74de9b10682d2af008b309713e16b57f7137d9aff0` |
| `raw_query_manifest.jsonl` | 14 | `ee6aef67ba2d9763358e95f9d1e206c7707005ffc7c64181ec5e1de5709a6752` |
| `generated_candidate_falsification_bridge_rows.jsonl` | 1,000 | `ee00570033691c6292a353403c8f291bf25bf254b5e8a4e73949f2321dc163b5` |
| `validation_metrics.json` | - | `730e4eb9a31741acff090042545140f02906928c4aff57568883d9e3aef6a046` |
| `persisted_readback.json` | - | `4495e768c412d321c32fdae3f8051dec17dcb2ace436eed7d0e18d846bab9704` |
| `calyx_bridge_corpus_readback.json` | - | `184395b83aa7669960bc3bd904755e604860024b4dbf8ea73b651c7e1474e80e` |

Persisted readback assertions:

| Assertion | Value |
|---|---:|
| One flag per candidate | true |
| Support rows present | true |
| Counter rows present | true |
| Raw manifest present | true |
| Top demoted file present | true |
| Clinical boundary present on all flags | true |

## Top Demoted / Blocked Examples

These are not "bad drugs" or clinical guidance. They are generated candidates
that failed the current required-evidence and counterevidence triage gates.

| Candidate | Source | Gene(s) | Drug(s) | Disease(s) | Status | Score | Reasons |
|---|---|---|---|---|---|---:|---|
| `oncology-civic:1471` | #1185 | NF1 | AZ628; VTX-11e | Skin Melanoma | blocked missing evidence/safety | 0.679688 | DGIdb exact pair missing; embedded safety/trial gap; safety block/high-risk label; trial missing |
| `oncology-civic:7815` | #1185 | NT5C2 | Cytarabine; Doxorubicin; Gemcitabine | Childhood Acute Lymphocytic Leukemia | blocked missing evidence/safety | 0.628440 | DGIdb exact pair missing; embedded safety/trial gap; safety block/high-risk label; trial missing |
| `issue1187:normalized_comention:17b6c746f037502ed3cb` | #1187/#1222 | - | Kanamycin | Tinnitus | blocked missing evidence/safety | 0.615385 | safety source missing; trial missing |
| `issue1187:normalized_comention:1952baf7598d8f077aab` | #1187/#1222 | - | Phenytoin | Seizures | blocked missing evidence/safety | 0.615385 | safety source missing; trial missing |
| `issue1187:normalized_comention:586502cb3434d0d4dd78` | #1187/#1222 | - | Pregabalin | Migraine Disorders | blocked missing evidence/safety | 0.615385 | safety source missing; trial missing |

## Findings

- The generated hunt corpus now has a single persisted falsification state per
  deduped candidate: 1,877 candidates in, 1,877 flags out.
- 1,082 candidates are blocked or demoted before any atlas-promotion step. The
  dominant failure mode is missing required trial/safety evidence for
  drug-bearing disease candidates, which is intentionally fail-closed.
- The normalization repair removed the accidental treatment of endpoint labels
  as target symbols; the DGIdb exact-pair-missing reason fell to 14 rows after
  rerun.
- 795 rows have no counterevidence in the current bounded sources, but that is
  not enough for efficacy, safety, actionability, recommendation, or cure. They
  remain research candidates until outcome, safety, sufficiency, and human-review
  gates are satisfied.
- The 1,000 highest-priority falsification rows are now in the native Calyx
  database as bridge-corpus vault `01KWPGYV128AQVM7BMKW3AB6KY`.

## Conclusion

#1223 is complete for generated-candidate falsification triage: all generated
candidate inputs, source evidence files, falsification flags, demotion summaries,
and the native Calyx bridge-corpus materialization were persisted and separately
read back.

No treatment claim, efficacy claim, safety claim, clinical-actionability claim,
recommendation, or cure claim is made.
