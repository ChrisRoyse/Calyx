# #1237 PubMed Source-Text Validation

## Scope

#1237 validates the #1234 PubMed query-level co-mention hits against fetched
PubMed source text. The goal is to prevent ESearch hits from being treated as
stronger evidence than the title/abstract text supports.

The stage reads #1234 PubMed evidence rows, fetches PubMed EFetch XML for every
unique PMID, extracts title/abstract source text, verifies candidate drug-name
occurrence in that source text, and assigns a conservative deterministic
relation class:

- `co_mention_only`
- `asserted_combination`
- `asserted_interaction`
- `asserted_outcome`
- `counter_evidence`
- `insufficient_text`

The output is literature triage only. It is not efficacy proof, safety proof,
dosing guidance, treatment guidance, recommendation, clinical actionability, or
cure evidence.

## Implementation

Script:

```text
scripts/medicalsearch/issue1237_pubmed_source_text_validation.py
```

The script:

- reads #1234 `pubmed_pair_literature_evidence.jsonl`;
- reads #1234 `candidate_external_source_hits.jsonl`;
- fetches PubMed EFetch XML for all 523 unique PMIDs;
- parses `PubmedArticle` and `PubmedBookArticle` records;
- emits one validation row for every #1234 PubMed evidence row;
- emits one rollup row for every #1234 candidate PubMed-hit pair;
- keeps every promoted-looking row blocked pending safety, outcome,
  falsification, and human-review gates;
- writes an 869-row bridge-corpus slice for native Calyx materialization.

Accepted source:

- NCBI E-utilities intro/rate policy: <https://www.ncbi.nlm.nih.gov/books/NBK25497/>
- NCBI E-utilities parameters: <https://www.ncbi.nlm.nih.gov/books/NBK25499/>
- NLM E-utilities guide: <https://www.nlm.nih.gov/dataguide/eutilities/utilities.html>
- PubMed EFetch endpoint: <https://eutils.ncbi.nlm.nih.gov/entrez/eutils/efetch.fcgi>

## Real FSV

Final FSV root:

```text
/home/croyse/calyx/fsv/issue1237-pubmed-source-text-validation-20260704T173000Z
```

Primary readbacks:

```text
/home/croyse/calyx/fsv/issue1237-pubmed-source-text-validation-20260704T173000Z/out/persisted_readback.json
sha256: 9936093f6db4b18cc6cd86adc5056a8969f88405a00ca5cbd0dd42055539ad29

/home/croyse/calyx/fsv/issue1237-pubmed-source-text-validation-20260704T173000Z/out/calyx_bridge_corpus_readback.json
sha256: b9efc78f8865b6a24d7fb94a380aca9595e2af90abda46a5d98ed291ab8c8d5c
```

Native Calyx materialization:

```text
name: issue1237-pubmed-source-text-validation-20260704t173000z
vault_id: 01KWPZHVX7T7V9QBSND746X9T3
vault_dir: /home/croyse/calyx/vaults/01KWPZHVX7T7V9QBSND746X9T3
```

Materialization readback:

| Item | Count / value |
|---|---:|
| Bridge rows | 869 |
| Bridge terms | 669 |
| Graph nodes | 1,538 |
| Graph edges | 7,990 |
| CSR persisted | true |
| Active vault index contains final name exactly once | true |
| Graph nodes match readback | true |
| Graph edges match readback | true |
| Vault manifest/CURRENT files present | true |
| Graph files present | true |

## Inputs

| Input | Rows/bytes | SHA-256 |
|---|---:|---|
| #1234 PubMed evidence | 568 rows | `304400ca4bbc7faea42ac9f2dd708865e7e27a225cbde6e806301d4581b7f7ea` |
| #1234 candidate PubMed hits | 301 rows | `b60932183462b012f284a26750b0412c6a179224849938d7bb05ee42d473e950` |
| #1234 PubMed ESearch responses | 790 rows | `feb6390a6f23eabe273c64710ab9ce5b42c8e8481f101c440389cfabdf3a2339` |
| #1234 PubMed ESummary responses | 4 rows | `b7a871fd3b687d49b63d07e9050db23e1eb97e1b068722d2dcba5e173f5629e3` |
| #1234 persisted readback | 4,368 bytes | `a344a962768ad6d9b4759945e5fa76c95c0ac0de8afeec58e6628fa9a57aba16` |
| #1234 Calyx readback | 3,262 bytes | `21562a1baf00309e8c95a8311cd59dec84ed3df5e9a949cd75e6b7e799cca265` |
| NCBI E-utilities intro | 68,713 bytes | `204e634142073f071ade91b62c83e5034fe57c1ca1b1642a4d468aece020c65c` |
| NCBI E-utilities parameters | 114,258 bytes | `46f8c5354fe36d3d5b5386b5910bc5b46a9348a7a020617fa7ab3e9994039a02` |
| NLM E-utilities guide | 72,237 bytes | `08c0e23ecdec38e4fe8c6cdc0d655e87186662573a6d98de93a22a950fa1e081` |

## Output Artifacts

| Artifact | Rows | Bytes | SHA-256 |
|---|---:|---:|---|
| `pubmed_efetch_responses.jsonl` | 4 | 9,166,957 | `54cc54f6879797f3bc253b522a65bddbdfa7276024b4f82b7d9d73f373e65259` |
| `pubmed_source_records.jsonl` | 523 | 1,510,870 | `6e691f96107fff174115e1406036ff23559c4cdd284bce22186ca9eaf13fd8f0` |
| `pubmed_evidence_validation.jsonl` | 568 | 1,032,727 | `c649b829ae9baf5fe8ff6441e7258547762b1ac286634b949c7b802844c57308` |
| `candidate_pair_pubmed_validation_rollup.jsonl` | 301 | 561,841 | `67ab48f7a8cea6e796ca101531f7deee8fed4af511db9642bf56925c1dcd9052` |
| `candidate_pair_pubmed_validation_hits.jsonl` | 298 | 557,947 | `8d3bc89ba8caaa8a0ca9409d2836a6d789ef24d5b1a6f7e7226e3ee83274f9d9` |
| `pubmed_validation_bridge_rows.jsonl` | 869 | 1,086,712 | `c1835bb07ea73a9636b236e49a4f0a6e05a818f7ed64b1e238a079761e7baeb7` |
| `validation_metrics.json` | - | 12,848 | `9c6e14f650be915cf0c2fdb6c986b6267b455683b62717eda386755753d02033` |
| `output_manifest.json` | - | 2,668 | `b6bdd6738f5caca9386049b99416a09823d1a9cce6b707466f84188eda9b023c` |
| `persisted_readback.json` | - | 3,320 | `9936093f6db4b18cc6cd86adc5056a8969f88405a00ca5cbd0dd42055539ad29` |
| `calyx_bridge_corpus_stdout.json` | - | 735 | `c14ffa44cb635f7fa49d2548a36a44ef085e304702b49b74c8955640af919dac` |
| `calyx_bridge_corpus_readback.json` | - | 3,291 | `b9efc78f8865b6a24d7fb94a380aca9595e2af90abda46a5d98ed291ab8c8d5c` |

## Metrics

| Metric | Count |
|---|---:|
| #1234 PubMed evidence rows | 568 |
| #1234 candidate PubMed-hit rows | 301 |
| Unique PMIDs fetched | 523 |
| PubMed EFetch response chunks | 4 |
| Parsed PubMed source records | 523 |
| Evidence validation rows | 568 |
| Candidate-pair rollup rows | 301 |
| Source-text validated evidence rows | 519 |
| Insufficient-text evidence rows | 49 |
| Counter-evidence rows | 224 |

Evidence relation-class counts:

| Relation class | Rows |
|---|---:|
| `asserted_combination` | 17 |
| `asserted_interaction` | 182 |
| `asserted_outcome` | 87 |
| `co_mention_only` | 9 |
| `counter_evidence` | 224 |
| `insufficient_text` | 49 |

Candidate-pair rollup status counts:

| Status | Rows |
|---|---:|
| `counter_evidence_review_required_still_blocked` | 188 |
| `source_text_validated_still_blocked` | 110 |
| `query_hit_not_validated_by_source_text` | 3 |

Best relation-class counts by candidate pair:

| Best class | Rows |
|---|---:|
| `asserted_combination` | 6 |
| `asserted_interaction` | 68 |
| `asserted_outcome` | 36 |
| `counter_evidence` | 188 |
| `insufficient_text` | 3 |

## Readback Assertions

| Assertion | Value |
|---|---:|
| Source record for every unique PMID | true |
| Validation row for every #1234 PubMed evidence row | true |
| Rollup row for every #1234 candidate PubMed hit | true |
| All validation rows carry the clinical boundary | true |
| All validation rows have a relation class | true |
| All non-insufficient rows have both candidate names present | true |
| Insufficient rows do not claim validation | true |
| Bridge rows <= 1,000 | true |
| Bridge row count matches materializer stdout | true |
| Bridge SHA matches materializer stdout | true |
| Active vault index contains exactly one final name | true |
| Active vault id matches materializer stdout | true |
| Vault manifest/CURRENT files present | true |
| Graph files present | true |

## Findings

- #1237 upgraded #1234 from query-level PubMed co-mention to fetched
  title/abstract source-text validation.
- All 523 unique PMIDs returned a source record after parsing both
  `PubmedArticle` and `PubmedBookArticle`.
- 519 of 568 evidence rows physically contain both candidate drug names in
  fetched title/abstract source text.
- 49 evidence rows remain `insufficient_text` and must not be used as validated
  PubMed support.
- 224 evidence rows and 188 candidate-pair rollups are flagged as
  `counter_evidence` review required by the conservative deterministic rules.
- 110 candidate-pair rollups are source-text validated and still blocked pending
  structured extraction, safety, outcome, falsification, and human-review gates.

## Conclusion

#1237 is complete for source-text validation: it proves which #1234 PubMed
query hits have candidate-name support in fetched title/abstract text and keeps
all rows blocked short of clinical claims.

No treatment claim, efficacy claim, safety claim, clinical-actionability claim,
recommendation, dosing guidance, or cure claim is made.
