# #1253 Independent Effect-Result Validation

Status: complete for the independent source-validation pass over #1252
effect-result candidates.

This slice loaded the #1252 direction/magnitude and direction-only candidate
rollups, queried independent source surfaces, and required source-local
pair-term evidence outside the #1251 source windows. No independent support row
survived the strict pair-term and source-exclusion gate, so all candidate
rollups remain blocked.

Clinical boundary:

```text
Independent effect-result validation is source triage only; not efficacy, safety, treatment guidance, dosing guidance, recommendation, clinical actionability, or cure evidence.
```

## Inputs

FSV root:

```text
/home/croyse/calyx/fsv/issue1253-independent-effect-result-validation-20260704T202500Z
```

Sealed upstream inputs:

```text
/home/croyse/calyx/fsv/issue1252-effect-result-falsification-gate-20260704T235000Z/out/effect_result_rollup_status.jsonl
sha256: 43c11b2c653ff0772e6ef382ceafa6b1a84d5a10debba491586fc8107309a8b1

/home/croyse/calyx/fsv/issue1252-effect-result-falsification-gate-20260704T235000Z/out/effect_result_evidence_review.jsonl
sha256: faa3c1527ed9e79ecb53841e0a10cada8d0c2b1462c449cc2f2b0dbba453095c

/home/croyse/calyx/fsv/issue1252-effect-result-falsification-gate-20260704T235000Z/out/persisted_readback.json
sha256: 4a9e67714abb8fae8f26b021f5d4829cfd2a3aba4b76cd5777c184215116cf3e

/home/croyse/calyx/fsv/issue1252-effect-result-falsification-gate-20260704T235000Z/out/calyx_bridge_corpus_readback.json
sha256: cd80e2ebcfc7819b6a1322a96a0560464bf14b614d04ef28a6ce23ca8a898756
```

Persisted source/API documentation:

| Source doc | Bytes | SHA-256 |
|---|---:|---|
| `europepmc_rest_docs.html` | 64,486 | `e3b45ced2caf9a35496a33ffbe79e745f79a48ba61d2fbc6996cf94d6bce4c97` |
| `clinicaltrials_oas_v2.yaml` | 80,983 | `ba31adaea67e6bb09ff77af5c0c11daf36d5aff7f5d6cbed89f0aab04b297aea` |
| `ncbi_eutilities_docs.html` | 68,713 | `714a344a0e24d2c98344cee9509bd6770787315fc4c029d4071352157a8784af` |

Source contract:

- Input scope was the 24 #1252 rollups with
  `effect_result_candidate_with_magnitude_and_direction_still_blocked` or
  `effect_result_candidate_direction_only_still_blocked`.
- The 24 rollups represented 17 unique candidate pair keys.
- Each unique pair was queried against Europe PMC Articles REST,
  ClinicalTrials.gov v2, and PubMed E-utilities.
- An independent evidence row required both pair terms in the returned source
  text and a source id not overlapping the #1251 source ids.
- Result, magnitude, comparator, safety, and counter language remained triage
  fields only.

## Output Artifacts

| Artifact | Rows | Bytes | SHA-256 |
|---|---:|---:|---|
| `independent_source_query_responses.jsonl` | 51 | 1,607,383 | `34a4269b07b22922dfa6a155fe78e282e862165d9365fe522b426c55b4745e7a` |
| `independent_effect_evidence_review.jsonl` | 0 | 0 | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |
| `independent_effect_rollup_status.jsonl` | 24 | 34,142 | `c0b79bed2ec4c0ff765697d094f6156903a87586b138cba252b74eb7ae784b8f` |
| `independent_effect_bridge_rows.jsonl` | 75 | 91,238 | `f68026ad9b940064ec5bcba59cca111aaf248b6b0f372eb0889ff173ff7f484f` |
| `input_manifest.json` | - | 3,244 | `fc6cb185fd45355f0d05df40a9c85c2dd1959d8cf6817f936da34fe380e9a0be` |
| `validation_metrics.json` | - | 8,741 | `89b1731f6437dfdbc725a650a76177f3dbdc9fd7c15273efb188649ee37a65b3` |
| `output_manifest.json` | - | 2,078 | `dac470ce302eaab4721bbc40b15f0281a2836bb675a5b5122416fb25f20ee733` |
| `persisted_readback.json` | - | 3,195 | `3d41d08e409c93344a5d2bc7671e364487c03427ce8a01418bb03bca27a09608` |
| `calyx_bridge_corpus_stdout.json` | - | 729 | `cb32e995e150bdc6428851ec4da40f2f04a9c1a4a059f2571280cb3075052160` |
| `calyx_bridge_corpus_readback.json` | - | 5,483 | `85ee7d95707f4b0868d6ad1d014f2a03510f0f1531d712290f505bc170e85227` |

## Metrics

| Metric | Count |
|---|---:|
| Candidate rollups | 24 |
| Unique candidate pairs | 17 |
| Query response rows | 51 |
| Europe PMC queries | 17 |
| ClinicalTrials.gov queries | 17 |
| PubMed queries | 17 |
| Independent evidence rows | 0 |
| Rollup status rows | 24 |
| Rollups with independent result language | 0 |
| Rollups with independent safety/counter language | 0 |

Rollup status counts:

| Status | Rollups |
|---|---:|
| `no_independent_endpoint_result_source_hit_still_blocked` | 24 |

Validation assertions:

| Assertion | Result |
|---|---|
| #1252 persisted readback all true | true |
| #1252 Calyx readback all true | true |
| Rollup status for every candidate rollup | true |
| Query response for every pair and source | true |
| Evidence status values allowed | true |
| Rollup status values allowed | true |
| Independent evidence source provenance present | true |
| Independent evidence pair terms present | true |
| Independent evidence excludes #1251 sources | true |
| Rows carry the clinical boundary | true |
| Rows remain blocked | true |
| Bridge rows <= 1,000 | true |

## Native Calyx Materialization

```text
name: issue1253-independent-effect-result-validation-20260704t202500z
vault_id: 01KWQD30JCVKGAZ8JDGFTF0A2N
vault_dir: /home/croyse/calyx/vaults/01KWQD30JCVKGAZ8JDGFTF0A2N
```

Materialization readback:

| Item | Count / value |
|---|---:|
| Bridge rows | 75 |
| Bridge terms | 53 |
| Graph nodes | 128 |
| Graph edges | 702 |
| CSR persisted | true |
| Active vault index contains final name exactly once | true |
| Graph nodes match readback | true |
| Graph edges match readback | true |
| Vault manifest/CURRENT files present | true |
| Graph files present | true |

## Result

#1253 did not validate any #1252 candidate as an independent effect-result
source. The strict independent gate found zero source-local pair evidence rows
outside the #1251 source windows across Europe PMC, ClinicalTrials.gov, and
PubMed for the 17 unique candidate pairs. All 24 candidate rollups remain
blocked with `no_independent_endpoint_result_source_hit_still_blocked`.

No efficacy claim, safety claim, treatment guidance, recommendation, clinical
actionability, dosing guidance, or cure claim is made.
