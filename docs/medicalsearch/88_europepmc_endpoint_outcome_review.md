# #1247 Europe PMC Endpoint/Outcome Review

Status: complete.

This slice reviewed the #1244 Europe PMC rollups that had bounded source-text
relation context without safety/counter language. It separates endpoint/outcome,
pharmacokinetic/exposure, mechanistic, trial-design, and effect-size language.
Every row remains blocked pending independent endpoint validation,
safety/falsification, and human review.

Clinical boundary:

```text
Europe PMC endpoint/outcome review is literature triage only; not efficacy, safety, treatment guidance, dosing guidance, recommendation, clinical actionability, or cure evidence.
```

## Inputs

FSV root:

```text
/home/croyse/calyx/fsv/issue1247-europepmc-endpoint-outcome-review-20260704T223000Z
```

Sealed upstream inputs:

```text
/home/croyse/calyx/fsv/issue1244-europepmc-relation-validation-20260704T193000Z/out/candidate_europepmc_relation_rollup.jsonl
sha256: 730cd82b585050ad04909c0d29d0e43bd2a891ff6d40cf9f23ac096bb03f37ef

/home/croyse/calyx/fsv/issue1244-europepmc-relation-validation-20260704T193000Z/out/europepmc_source_text_relation_validation.jsonl
sha256: ab2cdc88971890fe6e036fe11f3a46c1e779e9c2a52cae429f5c16a2966c33d2
```

Source contract:

- The input filter was `source_text_validation_status == source_text_relation_extracted_still_blocked`.
- Scoped #1244 rollups: 108.
- The stage did not fetch new source data; it read #1244 bounded source-text windows.
- Endpoint/outcome category labels are review flags only, not efficacy findings or clinical advice.

## Method

The reviewer:

- emitted one rollup review row for each scoped #1244 rollup;
- emitted evidence-review rows for linked #1244 source-text validation rows;
- separated clinical endpoint, preclinical/cell endpoint, endpoint/outcome,
  pharmacokinetic/exposure, mechanistic, and combination/coexposure context;
- preserved source ids, source-window text, source hashes, relation classes,
  model-system labels, effect-size strings, and dose/exposure strings;
- kept every row blocked pending independent endpoint/outcome validation,
  safety/falsification, and human review;
- wrote a 215-row bridge-corpus slice for native Calyx materialization.

## Output Artifacts

| Artifact | Rows | Bytes | SHA-256 |
|---|---:|---:|---|
| `europepmc_endpoint_outcome_evidence_review.jsonl` | 107 | 774,049 | `edfc474cbe8b98a3bb3a17452a7a34cca7d67ae34cd4b810e609fbb9c4c74f64` |
| `candidate_europepmc_endpoint_outcome_rollup.jsonl` | 108 | 298,820 | `d13703c71a7e1dedf595ee4b21fd0fbf5535d33c7487bebfd3717426e9aea90c` |
| `europepmc_endpoint_outcome_bridge_rows.jsonl` | 215 | 323,268 | `95ca04007dca7d23d5427cc66d7dd763b87243ab4e67716e6332a56167a5a9b8` |
| `input_manifest.json` | - | 2,077 | `f5a13a7f79997922a4cce5e3bf18c9844753c6832aca5606615f63619670163a` |
| `validation_metrics.json` | - | 20,834 | `4ee0551bf2b2d7ecbc5a6165eec008b73e099b3557e17f87963b24feefc89ad4` |
| `output_manifest.json` | - | 1,806 | `ad0f8c76b27fe20764c340719f1f09204c7b38180bd4946820866a87805e37f4` |
| `persisted_readback.json` | - | 2,958 | `1ffc6d8ca28310b039c8d2ed7eef7bda81ab166e5ab028a2a92fa38d766f2b3c` |
| `calyx_bridge_corpus_stdout.json` | - | 743 | `e8f9a330bee58df0201f7c0d37bc8484c5e1936199658f56ea148ec78c5a8ffa` |
| `calyx_bridge_corpus_readback.json` | - | 5,206 | `88c08f1188021eeebef24b5e3cf09035b87d6d66862d7d8bab79354ce8507e36` |

## Metrics

| Metric | Count |
|---|---:|
| Scoped #1244 rollups | 108 |
| Evidence-review rows | 107 |
| Rollup-review rows | 108 |
| Rollups with endpoint/outcome language | 82 |
| Rollups with effect-size language | 32 |
| Rollups with pharmacokinetic/exposure language | 34 |
| Rollups with mechanistic language | 91 |
| Rollups with trial-design language | 74 |
| Rollups with preclinical language | 77 |
| Rollups with combination/coexposure language | 33 |

Source #1244 relation-class counts:

| Relation class | Rollups |
|---|---:|
| `combination_or_coexposure` | 5 |
| `mechanistic_or_interaction_context` | 91 |
| `trial_or_outcome_context` | 12 |

Evidence category counts:

| Category | Evidence rows |
|---|---:|
| `clinical_endpoint_language_review` | 48 |
| `endpoint_or_outcome_language_review` | 26 |
| `mechanistic_endpoint_context_review` | 18 |
| `pharmacokinetic_or_exposure_endpoint_review` | 6 |
| `preclinical_or_cell_endpoint_language_review` | 5 |
| `combination_or_coexposure_context_review` | 4 |

Rollup category counts:

| Category | Rollups |
|---|---:|
| `clinical_endpoint_language_review` | 59 |
| `endpoint_or_outcome_language_review` | 21 |
| `mechanistic_endpoint_context_review` | 19 |
| `pharmacokinetic_or_exposure_endpoint_review` | 4 |
| `combination_or_coexposure_context_review` | 3 |
| `preclinical_or_cell_endpoint_language_review` | 2 |

Evidence primary model-system counts:

| Model/system | Evidence rows |
|---|---:|
| `human_clinical_or_patient` | 76 |
| `in_vitro_or_cell_system` | 18 |
| `unclear` | 8 |
| `animal_or_xenograft` | 3 |
| `computational_or_in_silico` | 2 |

Validation assertions:

| Assertion | Result |
|---|---|
| #1244 persisted readback all true | true |
| #1244 Calyx readback all true | true |
| Rollup review for every scoped rollup | true |
| Reviewed pair ids match scope | true |
| All rollup reviews have evidence | true |
| Evidence reviews have source windows | true |
| Evidence reviews carry the clinical boundary | true |
| Rollup reviews carry the clinical boundary | true |
| Evidence reviews have allowed categories | true |
| Rollup reviews have allowed categories | true |
| Evidence reviews remain blocked | true |
| Rollup reviews remain blocked | true |
| Bridge rows <= 1,000 | true |

## Native Calyx Materialization

```text
name: issue1247-europepmc-endpoint-outcome-review-20260704t223000z
vault_id: 01KWQA7Z9A3TJXJK57J5FJCD8N
vault_dir: /home/croyse/calyx/vaults/01KWQA7Z9A3TJXJK57J5FJCD8N
```

Materialization readback:

| Item | Count / value |
|---|---:|
| Bridge rows | 215 |
| Bridge terms | 451 |
| Graph nodes | 666 |
| Graph edges | 2,364 |
| CSR persisted | true |
| Active vault index contains final name exactly once | true |
| Graph nodes match readback | true |
| Graph edges match readback | true |
| Vault manifest/CURRENT files present | true |
| Graph files present | true |

## Result

#1247 accounted for all 108 #1244 relation-context rollups and separated 107
linked evidence windows into endpoint/outcome review categories. The useful
output is a blocked triage layer: 82 rollups have endpoint/outcome language, 32
have effect-size language, 34 have pharmacokinetic/exposure language, and 91
have mechanistic language, but none are validated endpoint evidence or clinical
claims.

No efficacy claim, safety claim, treatment guidance, recommendation, clinical
actionability, dosing guidance, or cure claim is made.
