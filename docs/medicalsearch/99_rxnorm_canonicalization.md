# #1258 RxNorm/RxNav Canonicalization

Status: complete for the RxNorm canonicalization pass over the #1257 nSIDES
no-map remainder.

This slice used the official RxNav/RxNorm APIs to canonicalize the 152 unique
terms remaining after #1257 name matching, persisted every response body, and
rescanned the #1257 TwoSIDES/OffSIDES source archives by trusted RxCUIs. Exact
and normalized RxCUIs, plus ingredient relations derived from trusted RxCUIs,
were allowed for source matching. Approximate RxNav matches were retained only
as provisional manual-review inputs and were not trusted for pair matching.

Clinical boundary:

```text
RxNorm/RxNav canonicalization is identity/source-mapping support only; mapped RxCUIs, ingredients, approximate matches, TwoSIDES RxCUI rows, and OffSIDES RxCUI rows are blockers/review inputs, not safety clearance, efficacy, treatment guidance, dosing guidance, recommendation, clinical actionability, pair-interaction proof, or cure evidence.
```

## Inputs

FSV root:

```text
/home/croyse/calyx/fsv/issue1258-rxnorm-canonicalization-20260705T000500Z
```

Sealed upstream #1257 inputs:

```text
/home/croyse/calyx/fsv/issue1257-nsides-source-mining-20260704T235500Z/out/candidate_nsides_status.jsonl
sha256: 70631c484ca28d8a20d08550ca0d15fabaa2eebfa3ab766ef2b43d4d8ccc4282

/home/croyse/calyx/fsv/issue1257-nsides-source-mining-20260704T235500Z/out/nsides_pair_status.jsonl
sha256: 16626c6da02115bb1cb6a9a71d1f094bdba9ca799c59b69bad7bc62e8715ca2a

/home/croyse/calyx/fsv/issue1257-nsides-source-mining-20260704T235500Z/out/offsides_single_drug_context.jsonl
sha256: 4d5a6991457b0088b04e3aa4cf22f3eac780fe3c627d7d19cf35f8aa765f12dc

/home/croyse/calyx/fsv/issue1257-nsides-source-mining-20260704T235500Z/out/persisted_readback.json
sha256: b0cbb727d64e4e3bab246d64e16d565bdbffa7e7a99a06dcbad0a8f163964fe4

/home/croyse/calyx/fsv/issue1257-nsides-source-mining-20260704T235500Z/out/calyx_bridge_corpus_readback.json
sha256: f92138819ab44501f9a1a00e9fc22c940551af31716562a04659ec54be231fb6

/home/croyse/calyx/fsv/issue1257-nsides-source-mining-20260704T235500Z/out/output_manifest.json
sha256: 644e1095034650d89686ccf58164ea7e9d6e2d2776be2bff268e063164a7bc5d
```

Persisted source documentation:

| Source doc | URL | Bytes | SHA-256 |
|---|---|---:|---|
| `rxnorm_api_overview.html` | `https://lhncbc.nlm.nih.gov/RxNav/APIs/RxNormAPIs.html` | 21,042 | `d037f07cac2e2f18225cff27f792c2d0133d945d67b175108d0866d4acfd49d8` |
| `find_rxcui_by_string.html` | `https://lhncbc.nlm.nih.gov/RxNav/APIs/api-RxNorm.findRxcuiByString.html` | 26,687 | `169ad799d3153b19e5d56597b7a8d2850b10a69f7fa3a3edd9e4d0562e9bbbae` |
| `approximate_match.html` | `https://lhncbc.nlm.nih.gov/RxNav/APIs/api-RxNorm.getApproximateMatch.html` | 24,462 | `d58f956f8f11a13029151c41f4538362ef25be3047ef7f7b1b6e2f7511cc1888` |
| `related_by_type.html` | `https://lhncbc.nlm.nih.gov/RxNav/APIs/api-RxNorm.getRelatedByType.html` | 24,658 | `9b0f92ac5831eb2549fb96e475254b6382a3532e0c87cf8ae8e1fc81edce37cf` |

Persisted source archives reused from #1257:

| Archive | Rows parsed | Bytes | SHA-256 |
|---|---:|---:|---|
| `twosides.csv.gz` | 42,920,391 | 738,463,578 | `59e5654a2b4cee2ebad1d37ec7840405c11eed3746dab337d836f73e63aea700` |
| `offsides.csv.gz` | 3,206,558 | 68,762,346 | `0b5d2bd93ed44b95c22d8f9f053acbef4f59280027ae54d48dfe40d4fb9d60b3` |

Source contract:

- Input scope was the 532 candidate rows and 353 unique pair keys still blocked
  after #1257.
- Each unique term was queried with RxNav `findRxcuiByString` using exact or
  normalized search, `getApproximateMatch`, and `getRelatedByType` for trusted
  exact/normalized RxCUIs.
- Exact/normalized RxCUIs and their related ingredient RxCUIs were trusted
  identity mappings for source rescans.
- Approximate matches remained provisional and did not create trusted pair
  evidence.
- TwoSIDES evidence required one TwoSIDES row containing trusted RxCUIs for
  both pair sides, in either drug position.
- OffSIDES evidence remained single-drug adverse-effect context only.
- Every output row remains blocked behind external identity, safety, outcome,
  falsification, and human-review gates.

Runtime note:

- The first materialization attempt exposed a real bridge-row text/term contract
  mismatch: row text did not include the `raw_docs` bridge term. The bridge row
  builder was patched to include all source and pair bridge terms in row text,
  then the full FSV root was rerun and materialized successfully.

## Output Artifacts

| Artifact | Rows | Bytes | SHA-256 |
|---|---:|---:|---|
| `rxnorm_source_rows.jsonl` | 7 | 7,287 | `65c11cddb30eec8f5d294358c37aa788428989a8ed58b3adcd46527f334cc2e2` |
| `rxnav_api_response_rows.jsonl` | 376 | 210,097 | `bf546c59ea7ed07d7eeccc55ef0571633745615997327ccb03fff064be92e309` |
| `rxnorm_term_status.jsonl` | 152 | 400,296 | `8907e7a9cb319a53a0c0fe7db78c9366209dd2aeb8a830b6ba02ce32917f7dd7` |
| `rxnorm_twosides_pair_evidence.jsonl` | 757 | 1,372,373 | `7392eee05700b23d3b7ae49a923a4255525fa91b327b4a5967a912194fb49d17` |
| `rxnorm_offsides_single_drug_context.jsonl` | 1,088 | 1,796,596 | `cbb8980f79f05010ab4c515a5b1a9908a302cccc4d867d9f84d9e60c1c7bdc30` |
| `rxnorm_pair_status.jsonl` | 353 | 717,170 | `20a57894bb796b843d49ddda07224a5a0545f2d28821f0ff0df8885eb4f07df0` |
| `candidate_rxnorm_status.jsonl` | 532 | 1,039,327 | `cb9ca1d1a1dd8af195e4dd9830e1072c4c1bfe7e2197ef77f9ec26f2baeddba2` |
| `rxnorm_bridge_rows.jsonl` | 1,000 | 1,392,869 | `087257f34fe3fdf850fa569d31aa592a8c08d376a69827bda935dfcdc2bf10a9` |
| `input_manifest.json` | - | 6,411 | `2110ff5270725451f4518f4e7c25b4fa3e7798f7a72dc1109a78dcc3ad9e3e74` |
| `validation_metrics.json` | - | 1,860 | `b6b896a1f25851d821b4e9617691f49a3e206747d7187bc715d8fc30750d7de0` |
| `output_manifest.json` | - | 3,260 | `1c90ac1e495357da85a74a5c405423d59c914482aaae7036e3c6543216a0c48c` |
| `persisted_readback.json` | - | 4,357 | `0011f528f7af04e18154e83dc193c822a1171e9d753d3c67f5e4d53add04b55d` |
| `calyx_bridge_corpus_stdout.json` | - | 763 | `5c7cd657bff0ff88d3fe80d1c6d9dd9e032ecb404aac6a5adfa37ee4f3b8dc05` |
| `calyx_bridge_corpus_stderr.txt` | - | 336 | `4026c3ec984e71df637a49ae4d5fa9480ef973d30e1933c8397a118bdb3e048c` |
| `calyx_bridge_corpus_readback.json` | - | 3,834 | `521f420f3b8b5f821064c3a341be76dcdd5ef18cc11da8f3235b5ff893913a50` |

## Metrics

| Metric | Count |
|---|---:|
| #1257 blocked candidate rows checked | 532 |
| Unique pair keys checked | 353 |
| Unique terms queried through RxNav | 152 |
| RxNav API response rows persisted | 376 |
| RxNav HTTP 200 responses | 376 |
| Trusted term mappings | 70 |
| Approximate-only provisional term mappings | 32 |
| No term mapping | 50 |
| TwoSIDES source rows parsed | 42,920,391 |
| OffSIDES source rows parsed | 3,206,558 |
| TwoSIDES RxCUI pair adverse-effect rows | 757 |
| Unique pair keys with TwoSIDES RxCUI rows | 7 |
| OffSIDES RxCUI single-drug context sample rows | 1,088 |
| Pair status rows | 353 |
| Candidate status rows | 532 |

Term status counts:

| Status | Term rows |
|---|---:|
| `rxnorm_term_trusted_mapping_still_blocked` | 70 |
| `rxnorm_term_approximate_only_provisional_still_blocked` | 32 |
| `rxnorm_term_no_mapping_still_blocked` | 50 |

Pair status counts:

| Status | Pair rows |
|---|---:|
| `rxnorm_twosides_rxcui_pair_hit_still_blocked` | 7 |
| `rxnorm_both_terms_trusted_mapped_without_twosides_pair_hit_still_blocked` | 29 |
| `rxnorm_approximate_only_provisional_still_blocked` | 163 |
| `rxnorm_unmapped_still_blocked` | 154 |

Candidate status counts:

| Status | Candidate rows |
|---|---:|
| `rxnorm_candidate_twosides_rxcui_pair_hit_still_blocked` | 16 |
| `rxnorm_candidate_both_terms_trusted_mapped_without_twosides_pair_hit_still_blocked` | 45 |
| `rxnorm_candidate_approximate_only_provisional_still_blocked` | 233 |
| `rxnorm_candidate_unmapped_still_blocked` | 238 |

TwoSIDES RxCUI pair evidence summary:

| Pair key | Rows | Max PRR | Example conditions |
|---|---:|---:|---|
| `etanercept szzs||sitagliptin` | 212 | 30.0 | Abdominal discomfort; Abdominal distension; Abdominal pain; Abdominal pain upper; Alopecia |
| `saxagliptin anhydrous||sitagliptin hydrochloride monohydrate` | 151 | 60.0 | Abdominal discomfort; Abdominal distension; Abdominal pain; Abdominal pain upper; Anaemia |
| `sitagliptin hydrochloride monohydrate||valacyclovir` | 124 | 40.0 | Abdominal discomfort; Abdominal pain; Abdominal pain upper; Abnormal dreams; Alanine aminotransferase increased |
| `infliximab dyyb||sitagliptin` | 123 | 40.0 | Abdominal pain; Anaemia; Anxiety; Arthralgia; Arthropathy |
| `etanercept szzs||ribavirin monophosphate` | 68 | 40.0 | Alanine aminotransferase increased; Anaemia; Arthralgia; Asthenia; Back pain |
| `etanercept||ribavirin monophosphate` | 68 | 40.0 | Alanine aminotransferase increased; Anaemia; Arthralgia; Asthenia; Back pain |
| `metformin||trametinib dimethyl sulfoxide` | 11 | 40.0 | Anaemia; Blood creatinine increased; Chills; Death; Dehydration |

Interpretation:

- The 757 rows are TwoSIDES adverse-effect source rows keyed by trusted RxCUIs.
- These rows are useful as safety/falsification blockers and review triage.
- They do not establish beneficial interaction, efficacy, safety clearance,
  causality, dosing, recommendation, clinical actionability, pair-interaction
  proof, or cure evidence.

Validation assertions:

| Assertion | Result |
|---|---|
| #1257 persisted readback all true | true |
| #1257 Calyx readback all true | true |
| 152 unique terms queried | true |
| Every RxNav response persisted | true |
| Every RxNav response had HTTP 200 status | true |
| Trusted pair matching used exact/normalized/related RxCUIs only | true |
| Approximate-only matches stayed provisional | true |
| Pair status for every pair key | true |
| Candidate status for every candidate | true |
| TwoSIDES evidence rows have both pair RxCUI matches | true |
| Evidence rows carry source row hashes | true |
| OffSIDES context rows are single-drug only | true |
| Status values allowed | true |
| Rows carry the clinical boundary | true |
| Rows remain blocked | true |
| Bridge rows <= 1,000 | true |

## Native Calyx Materialization

```text
name: issue1258-rxnorm-canonicalization-20260705t000500z
vault_id: 01KWQR9X3RSFTATP35PGFPY62A
vault_dir: /home/croyse/calyx/vaults/01KWQR9X3RSFTATP35PGFPY62A
```

Materialization readback:

| Item | Count / value |
|---|---:|
| Bridge rows | 1,000 |
| Bridge terms | 906 |
| Graph nodes | 1,906 |
| Graph edges | 10,278 |
| CSR persisted | true |
| Materializer index contains final name | true |
| Graph nodes match readback | true |
| Graph edges match readback | true |
| Vault `CURRENT` and `MANIFEST` files present | true |
| Vault `cf/graph` files present | true |
| Vault `cf/graph` file count | 12,187 |
| Vault `cf/graph` total bytes | 11,443,976 |

## Result

#1258 converted a name-normalization no-map remainder into a RxNorm-grounded
identity map. It rescued 70 trusted term mappings and surfaced 757 TwoSIDES
RxCUI pair adverse-effect rows across 7 pair keys, plus 1,088 OffSIDES
single-drug adverse-effect context rows. The strongest operational value is
that those 7 pair keys now have concrete safety/falsification rows to validate
independently rather than remaining name-matching misses.

Every candidate remains blocked. No efficacy, safety clearance, treatment
guidance, dosing guidance, clinical recommendation, clinical actionability,
pair-interaction proof, or cure claim is made.
