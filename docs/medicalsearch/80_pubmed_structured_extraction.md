# #1238 PubMed Structured Relation/Safety/Outcome Extraction

## Scope

#1238 extracts deterministic structured fields from the #1237 PubMed
source-text validation rows. The stage only reads #1237 sealed artifacts and
does not fetch new source data or call a model.

Eligible evidence rows are #1237 rows with relation class:

- `asserted_combination`
- `asserted_interaction`
- `asserted_outcome`
- `counter_evidence`

The output is literature triage only. It is not efficacy proof, safety proof,
dosing guidance, treatment guidance, recommendation, clinical actionability, or
cure evidence.

## Implementation

Script:

```text
scripts/medicalsearch/issue1238_pubmed_structured_extraction.py
```

The script:

- reads #1237 `pubmed_evidence_validation.jsonl`;
- reads #1237 `candidate_pair_pubmed_validation_rollup.jsonl`;
- joins each eligible evidence row to #1237 `pubmed_source_records.jsonl`;
- extracts deterministic source-text fields for relation direction, context,
  model/system, dose/exposure language, outcome/endpoints, safety/adverse-event
  language, and negation/counter-evidence spans;
- emits one structured extraction row per eligible #1237 evidence row;
- emits one structured rollup row per #1237 candidate-pair rollup;
- preserves all `counter_evidence` rows as blocked review inputs;
- writes an 811-row bridge-corpus slice for native Calyx materialization.

## Real FSV

Final FSV root:

```text
/home/croyse/calyx/fsv/issue1238-pubmed-structured-extraction-20260704T164501Z
```

Primary readbacks:

```text
/home/croyse/calyx/fsv/issue1238-pubmed-structured-extraction-20260704T164501Z/out/persisted_readback.json
sha256: 14e92d9f8908474385337d0b71d10531d991897e9edf2dcdfea015806eaba413

/home/croyse/calyx/fsv/issue1238-pubmed-structured-extraction-20260704T164501Z/out/calyx_bridge_corpus_readback.json
sha256: 5e88bb218b3e06e31ff2eb3313a911b6c58daec4d85e79e428e87be6c2bafdf8
```

Native Calyx materialization:

```text
name: issue1238-pubmed-structured-extraction-20260704t164501z
vault_id: 01KWQ0M40D0PDJ52FPMCB1Z1Z7
vault_dir: /home/croyse/calyx/vaults/01KWQ0M40D0PDJ52FPMCB1Z1Z7
```

Materialization readback:

| Item | Count / value |
|---|---:|
| Bridge rows | 811 |
| Bridge terms | 625 |
| Graph nodes | 1,436 |
| Graph edges | 9,548 |
| CSR persisted | true |
| Active vault index contains final name exactly once | true |
| Graph nodes match readback | true |
| Graph edges match readback | true |
| Vault manifest/CURRENT files present | true |
| Graph files present | true |

## Inputs

| Input | Rows/bytes | SHA-256 |
|---|---:|---|
| #1237 PubMed evidence validation | 568 rows | `c649b829ae9baf5fe8ff6441e7258547762b1ac286634b949c7b802844c57308` |
| #1237 candidate-pair validation rollup | 301 rows | `67ab48f7a8cea6e796ca101531f7deee8fed4af511db9642bf56925c1dcd9052` |
| #1237 PubMed source records | 523 rows | `6e691f96107fff174115e1406036ff23559c4cdd284bce22186ca9eaf13fd8f0` |
| #1237 persisted readback | 3,320 bytes | `9936093f6db4b18cc6cd86adc5056a8969f88405a00ca5cbd0dd42055539ad29` |
| #1237 Calyx bridge-corpus readback | 3,291 bytes | `b9efc78f8865b6a24d7fb94a380aca9595e2af90abda46a5d98ed291ab8c8d5c` |
| #1237 output manifest | 2,668 bytes | `b6bdd6738f5caca9386049b99416a09823d1a9cce6b707466f84188eda9b023c` |

## Output Artifacts

| Artifact | Rows | Bytes | SHA-256 |
|---|---:|---:|---|
| `input_manifest.json` | - | 2,774 | `d6b971c770c621a30aeb2b96fddd08a95dacde27da10f5802fa14c1560d4e3c1` |
| `pubmed_structured_extraction.jsonl` | 510 | 3,965,392 | `10bc9ca90b97a089962bd85ee2f7881819668414859fdedb8040c86d78fa992e` |
| `candidate_pair_pubmed_structured_rollup.jsonl` | 301 | 701,773 | `e0c8db57ee492727fa525c9044dfcce85bf03acbfd7a03587030ab2d8393c3e6` |
| `candidate_pair_pubmed_structured_hits.jsonl` | 298 | 696,699 | `cb6ce01e363c6c320231cfaf01e59c9fafb891c6d437e1a9bc2f5ca924874c26` |
| `pubmed_structured_extraction_bridge_rows.jsonl` | 811 | 1,231,788 | `1abc99b16dc2a437cd287ae4d132609fda47b7a1ff7aa4321fd31ccb123cade3` |
| `validation_metrics.json` | - | 15,572 | `63ad0c804585a3ec634f38f7e519b4cf986e540e75353d4b10cce8a19a8eeceb` |
| `output_manifest.json` | - | 2,099 | `8fc6728117d993bb74ea3e1090ee94136f449ded4c5c89f733ce19fe0ba9ce74` |
| `persisted_readback.json` | - | 3,382 | `14e92d9f8908474385337d0b71d10531d991897e9edf2dcdfea015806eaba413` |
| `calyx_bridge_corpus_stdout.json` | - | 742 | `5894bb48d53152880ff0df054b80b4a4625970985abf23b41bed28ea98e390d7` |
| `calyx_bridge_corpus_readback.json` | - | 3,603 | `5e88bb218b3e06e31ff2eb3313a911b6c58daec4d85e79e428e87be6c2bafdf8` |

## Metrics

| Metric | Count |
|---|---:|
| #1237 evidence validation rows | 568 |
| Eligible evidence rows | 510 |
| Structured extraction rows | 510 |
| Candidate-pair structured rollups | 301 |
| Candidate-pair structured hits | 298 |
| Counter-evidence extraction rows | 224 |
| Rows with safety language | 193 |
| Rows with outcome language | 389 |
| Rows with dose/exposure language | 197 |
| Rows with unclear direction | 219 |

Structured relation-class counts:

| Relation class | Rows |
|---|---:|
| `asserted_combination` | 17 |
| `asserted_interaction` | 182 |
| `asserted_outcome` | 87 |
| `counter_evidence` | 224 |

Structured extraction status counts:

| Status | Rows |
|---|---:|
| `counter_evidence_structured_review_required_still_blocked` | 224 |
| `structured_combination_relation_still_blocked` | 17 |
| `structured_interaction_relation_still_blocked` | 182 |
| `structured_outcome_language_still_blocked` | 87 |

Candidate-pair rollup status counts:

| Status | Rows |
|---|---:|
| `counter_evidence_structured_review_required_still_blocked` | 188 |
| `structured_relation_extracted_still_blocked` | 110 |
| `no_structured_extraction_fail_closed` | 3 |

Primary model/system counts:

| Model/system | Rows |
|---|---:|
| `human_clinical_or_patient` | 406 |
| `animal_or_xenograft` | 83 |
| `in_vitro_or_cell_system` | 9 |
| `review_or_guideline` | 2 |
| `computational_or_in_silico` | 1 |
| `unclear` | 9 |

Relation direction counts:

| Direction | Rows |
|---|---:|
| `text_order_drug_a_then_drug_b_not_causal` | 123 |
| `text_order_drug_b_then_drug_a_not_causal` | 168 |
| `undirected_or_unclear` | 219 |

## Readback Assertions

| Assertion | Value |
|---|---:|
| #1237 persisted readback assertions all true | true |
| #1237 Calyx readback assertions all true | true |
| Extraction row for every eligible validation row | true |
| Rollup row for every #1237 candidate rollup | true |
| All extractions have source records | true |
| All extractions carry the clinical boundary | true |
| All rollups carry the clinical boundary | true |
| All extractions remain blocked | true |
| All rollups remain blocked | true |
| Counter-evidence row count preserved | true |
| No co-mention-only or insufficient-text rows extracted | true |
| Bridge rows <= 1,000 | true |
| Bridge rows cover rollups and extractions | true |
| Source records cover extraction PMIDs | true |
| Bridge row count matches materializer stdout | true |
| Bridge SHA matches materializer stdout | true |
| Active vault index contains exactly one final name | true |
| Active vault id matches materializer stdout | true |
| Vault manifest/CURRENT files present | true |
| Graph files present | true |

## Findings

- #1238 converts #1237 source-text validated PubMed evidence into structured
  association metadata for downstream gates.
- 510 of 568 #1237 validation rows were eligible and received structured
  extraction rows.
- 224 counter-evidence rows were preserved exactly as blocked review inputs.
- 298 candidate-pair rollups now have at least one structured extraction row.
- 188 candidate-pair rollups remain blocked by counter-evidence review status.
- 110 candidate-pair rollups have structured relation fields but are still
  blocked pending independent safety, outcome, falsification, and human-review
  gates.
- 3 candidate-pair rollups had no eligible structured extraction and remain
  fail-closed.

## Conclusion

#1238 is complete for deterministic PubMed structured extraction. It provides
source-attributed relation/context/model/safety/outcome/exposure fields for
downstream validation, preserves counter-evidence, and materializes the result
into Calyx.

No treatment claim, efficacy claim, safety claim, clinical-actionability claim,
recommendation, dosing guidance, or cure claim is made.
