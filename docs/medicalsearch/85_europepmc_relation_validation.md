# #1244 Europe PMC Source-Text Relation Validation

Status: complete.

This slice validated the #1243 Europe PMC co-mention hits by reopening the
persisted source text, extracting bounded source-text windows around both
candidate terms, and classifying relation/safety/outcome context with
deterministic lexical gates.

Clinical boundary:

```text
Europe PMC source-text relation validation is literature triage only; not efficacy, safety, treatment guidance, dosing guidance, recommendation, clinical actionability, or cure evidence.
```

## Inputs

FSV root:

```text
/home/croyse/calyx/fsv/issue1244-europepmc-relation-validation-20260704T193000Z
```

Sealed upstream inputs:

```text
/home/croyse/calyx/fsv/issue1243-europepmc-pair-search-20260704T181500Z/out/candidate_europepmc_status.jsonl
sha256: 7a3b63dea0ff880374b68627e9465d1d11d1e7758e1bcf6cec59050611746c03

/home/croyse/calyx/fsv/issue1243-europepmc-pair-search-20260704T181500Z/out/europepmc_pair_evidence.jsonl
sha256: a393992fb1e5b0d8a83f6f99e27e6b1c6aeeb5563615bb2a729b0602075aa609
```

Source contract:

- The input filter was `europepmc_status in {exact_hit, normalized_hit}`.
- Candidate hit rows from #1243: 487.
- Europe PMC evidence rows from #1243: 598.
- No new source data was fetched. The validator read only persisted #1243
  query responses and cached PMCID full-text XML files.
- Relation, safety, outcome, counter, and dose/exposure patterns were evaluated
  on the bounded nearest-term source-text window, not on whole articles.
- Every output row remains blocked pending independent safety, outcome,
  falsification, and human-review gates.

## Method

The validator:

- re-read each #1243 evidence row and source id;
- reconstructed metadata source text from the persisted Europe PMC response or
  read the cached PMCID `fullTextXML` payload;
- verified both candidate names in source text;
- emitted one validation row per #1243 evidence row;
- emitted one rollup row per #1243 hit candidate row;
- classified bounded source windows into `co_mention_only`,
  `combination_or_coexposure`, `comparative_context`,
  `mechanistic_or_interaction_context`, `trial_or_outcome_context`,
  `safety_or_adverse_context`, or `counter_or_negative_context`;
- preserved safety/counter/outcome/dose flags as review fields, not claims;
- wrote a 1,000-row bridge-corpus slice for native Calyx materialization.

## Output Artifacts

| Artifact | Rows | Bytes | SHA-256 |
|---|---:|---:|---|
| `europepmc_source_text_relation_validation.jsonl` | 598 | 15,685,763 | `ab2cdc88971890fe6e036fe11f3a46c1e779e9c2a52cae429f5c16a2966c33d2` |
| `candidate_europepmc_relation_rollup.jsonl` | 487 | 1,173,411 | `730cd82b585050ad04909c0d29d0e43bd2a891ff6d40cf9f23ac096bb03f37ef` |
| `candidate_europepmc_relation_review.jsonl` | 471 | 1,141,991 | `1d097af6e336a0ef2dc4f23b599fdf318ede6921218e79779a35371f51fc615b` |
| `europepmc_relation_bridge_rows.jsonl` | 1,000 | 1,671,617 | `50ce1ac85107522ebf2d37d8ed973f988b8a374c247926386541ad6788ba5a62` |
| `input_manifest.json` | - | 2,654 | `cf1556de9dc566fd72bc1e26491d18411346df020dc980683be07091d01047d0` |
| `validation_metrics.json` | - | 16,700 | `6f0faf3174c9015d30847a4a8b67a8a6f36d9281b84aba0dc2eb22326fb4d379` |
| `output_manifest.json` | - | 2,126 | `edfbc1ceac05a064ae60dc78d51e3d08fbee1abc6db3f50cdd1b139dc1f8683c` |
| `persisted_readback.json` | - | 3,396 | `acd861001ffa19c6e3ece25d1e70071b1096b0a7eeb6855be12797e14eab7a0e` |
| `calyx_bridge_corpus_stdout.json` | - | 740 | `ce4f159f1ea6998feed77b0fa42a865c3d36d81ee90a2414fe07a6490b771e4a` |
| `calyx_bridge_corpus_readback.json` | - | 5,173 | `2341f995f0d0b81fe9093bbaa4483be0992f4e17ab44d069bad39a74bb1f687d` |

## Metrics

| Metric | Count |
|---|---:|
| #1243 candidate rows read | 1,019 |
| #1243 hit candidate rows | 487 |
| #1243 evidence rows validated | 598 |
| Candidate relation rollup rows | 487 |
| Candidate rollups with source-text terms verified | 487 |
| Candidate relation review rows | 471 |
| Rows with safety language | 270 |
| Rows with counter/negative language | 265 |
| Rows with outcome language | 459 |
| Rows with dose/exposure language | 246 |

Evidence relation-class counts:

| Relation class | Rows |
|---|---:|
| `co_mention_only` | 55 |
| `combination_or_coexposure` | 11 |
| `comparative_context` | 1 |
| `counter_or_negative_context` | 265 |
| `mechanistic_or_interaction_context` | 196 |
| `safety_or_adverse_context` | 45 |
| `trial_or_outcome_context` | 25 |

Evidence status counts:

| Status | Rows |
|---|---:|
| `source_text_comention_only_still_blocked` | 55 |
| `source_text_relation_extracted_still_blocked` | 233 |
| `source_text_safety_or_counter_review_required_still_blocked` | 310 |

Candidate rollup status counts:

| Status | Rows |
|---|---:|
| `source_text_comention_only_still_blocked` | 16 |
| `source_text_relation_extracted_still_blocked` | 108 |
| `source_text_safety_or_counter_review_required_still_blocked` | 363 |

Primary model/system counts:

| Model/system | Rows |
|---|---:|
| `human_clinical_or_patient` | 461 |
| `in_vitro_or_cell_system` | 51 |
| `unclear` | 46 |
| `computational_or_in_silico` | 21 |
| `animal_or_xenograft` | 18 |
| `review_or_guideline` | 1 |

Validation assertions:

| Assertion | Result |
|---|---|
| #1243 persisted readback all true | true |
| #1243 Calyx readback all true | true |
| Validation row for every #1243 evidence row | true |
| Rollup row for every #1243 hit candidate | true |
| Validation rows reference known evidence | true |
| Validation rows have source text | true |
| Validation rows verify both terms | true |
| Validation rows carry the clinical boundary | true |
| Rollups carry the clinical boundary | true |
| Validation rows remain blocked | true |
| Rollups remain blocked | true |
| Bridge rows <= 1,000 | true |

## Native Calyx Materialization

```text
name: issue1244-europepmc-relation-validation-20260704t193000z
vault_id: 01KWQ7E16HBVTNPKH848WZQFJM
vault_dir: /home/croyse/calyx/vaults/01KWQ7E16HBVTNPKH848WZQFJM
```

Materialization readback:

| Item | Count / value |
|---|---:|
| Bridge rows | 1,000 |
| Bridge terms | 1,830 |
| Graph nodes | 2,830 |
| Graph edges | 14,052 |
| CSR persisted | true |
| Active vault index contains final name exactly once | true |
| Graph nodes match readback | true |
| Graph edges match readback | true |
| Vault manifest/CURRENT files present | true |
| Graph files present | true |

## Result

#1244 converted the #1243 Europe PMC co-mention hits into bounded source-text
relation/safety/outcome/counter/dose triage rows. All 487 hit-candidate rollups
verified source text for both terms. The useful result is not a treatment claim:
363 candidate rollups require safety/counter review, 108 have relation context
but remain blocked, and 16 remain co-mention-only.

No efficacy claim, safety claim, treatment guidance, recommendation, clinical
actionability, dosing guidance, or cure claim is made.
