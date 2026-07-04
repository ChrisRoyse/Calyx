# #1249 openFDA FAERS Safety-Source Expansion

Status: complete for the openFDA FAERS event co-report expansion pass.

This slice continued #1248 after the openFDA Human Drug Label no-hit result by
querying a distinct source instrument: openFDA FAERS drug event reports. A
FAERS co-report row is adverse-event source triage only. It is not safety
clearance, pair-interaction proof, treatment guidance, recommendation, dosing
guidance, clinical actionability, efficacy, or cure evidence.

Clinical boundary:

```text
openFDA FAERS event expansion is adverse-event source triage only; not safety clearance, contraindication guidance, treatment guidance, dosing guidance, recommendation, clinical actionability, efficacy, or cure evidence.
```

## Inputs

FSV root:

```text
/home/croyse/calyx/fsv/issue1249-openfda-faers-safety-expansion-20260704T203500Z
```

Sealed upstream inputs:

```text
/home/croyse/calyx/fsv/issue1248-openfda-independent-safety-validation-20260704T211500Z/out/candidate_openfda_independent_status.jsonl
sha256: 075ba8265ad5160d8f5ae89c99e6c66cf61c8c4733cfdc3d2046e171ae3773a1

/home/croyse/calyx/fsv/issue1248-openfda-independent-safety-validation-20260704T211500Z/out/openfda_independent_pair_status.jsonl
sha256: 2ff40c8720c24c68a77f028dd641715d2368b27b307e3652496ef0fcec71268c

/home/croyse/calyx/fsv/issue1248-openfda-independent-safety-validation-20260704T211500Z/out/persisted_readback.json
sha256: 5a88dcf76c2092345cd14ba83dd5a7f8547d8a121c3aa38650052362d0fcfcda

/home/croyse/calyx/fsv/issue1248-openfda-independent-safety-validation-20260704T211500Z/out/calyx_bridge_corpus_readback.json
sha256: 92ff7fb1f974e8d01c06511fb54264a2f479e2c592da3154e9905fdc0f27584c
```

Persisted source/API documentation:

| Source doc | Bytes | SHA-256 |
|---|---:|---|
| `openfda_drug_event_api.html` | 128,623 | `8a043cbfa4650d79191f05309b279687ada890e074432e6ea80574dcab0c61f8` |
| `openfda_drug_event_fields.html` | 127,160 | `a318fe3288d61ecc00d5ac891de26622bf3e0966f876c6ebed36bad918920203` |
| `openfda_download_docs.html` | 122,916 | `3d7111b6d8449d47518bb1ec1651e7368bd9fe6c369342099d3368998d467d0a` |

Source contract:

- Input scope was the 363 #1248 rollups with
  `independent_openfda_no_hit_still_blocked`.
- The 363 rollups represented 202 unique pair keys.
- Each pair was queried against the openFDA Drug Event endpoint using both
  component names in `patient.drug.medicinalproduct`.
- A FAERS evidence row required both component terms to be physically present
  in returned event drug fields.
- Returned FAERS event rows remain blocking triage, not safety clearance.

## Output Artifacts

| Artifact | Rows | Bytes | SHA-256 |
|---|---:|---:|---|
| `faers_query_responses.jsonl` | 202 | 18,380,119 | `703ee94ca110e23bd58941387c31d3334615d508e7e7baf288b90c694956cc2b` |
| `faers_event_evidence.jsonl` | 45 | 14,936,518 | `c7ff1704f9491b27e6f8edd4f5b432e300236a41261d4999293145d3252df58b` |
| `faers_pair_status.jsonl` | 202 | 230,919 | `b4f75d65e844c062ea97e54fb4b709ac3b87d2a348f7ad398d98e690e0535c16` |
| `faers_rollup_status.jsonl` | 363 | 559,887 | `78656d19f4451d2a0edb7ee4db071b3b32518b664d4f12fb4fc28aef212276f3` |
| `faers_bridge_rows.jsonl` | 812 | 1,005,020 | `74595085d6767956fc00d054f2da1135230446bec29df3ed8472ca946d47812c` |
| `input_manifest.json` | - | 3,177 | `6301198353d780362bdef9cc1e766d64de676bf51351f2c8407eeb80331d54cd` |
| `validation_metrics.json` | - | 1,052 | `c5a5df0cc00b8bb932f0f933af2e20bd3dbeabaee751a47329a8d79bfb117da7` |
| `output_manifest.json` | - | 2,280 | `2971fa3f8f8cf3acd5ee5acf5b32b46bc2904c127786cc56884f5a2ba6d86128` |
| `persisted_readback.json` | - | 3,455 | `5590cf7e67a14d01264c2fca161d5fc237f6cd1212b064df52643516d910b7e7` |
| `calyx_bridge_corpus_stdout.json` | - | 746 | `f2733919f9bf2362c407b32249a28428774d48197438ff26ab80666b2750ad42` |
| `calyx_bridge_corpus_readback.json` | - | 5,708 | `ebc35251cd48365decc34688ef9bb34f810dfa885386d05f69c129d7e4e19334` |

## Metrics

| Metric | Count |
|---|---:|
| #1248 rollups checked | 363 |
| Unique pair keys queried | 202 |
| FAERS query response rows | 202 |
| FAERS event evidence rows | 45 |
| Pair status rows | 202 |
| Rollup status rows | 363 |
| Rollups with FAERS event co-report | 82 |
| Rollups with serious FAERS event co-report | 79 |

Query HTTP status counts:

| HTTP status | Pair queries |
|---|---:|
| `200` | 45 |
| `404` | 157 |

Pair status counts:

| Status | Pair rows |
|---|---:|
| `faers_pair_coreport_event_hit_still_blocked` | 2 |
| `faers_pair_coreport_serious_event_hit_still_blocked` | 43 |
| `faers_pair_query_no_result_still_blocked` | 157 |

Rollup status counts:

| Status | Rollups |
|---|---:|
| `faers_rollup_coreport_event_hit_still_blocked` | 3 |
| `faers_rollup_coreport_serious_event_hit_still_blocked` | 79 |
| `faers_rollup_query_no_result_still_blocked` | 281 |

Validation assertions:

| Assertion | Result |
|---|---|
| #1248 persisted readback all true | true |
| #1248 Calyx readback all true | true |
| Query response for every queryable pair key | true |
| Pair status for every pair key | true |
| Rollup status for every #1248 rollup | true |
| All FAERS hits have evidence rows | true |
| Evidence rows have source hashes | true |
| Evidence rows have pair terms in event drug fields | true |
| Pair/rollup status values allowed | true |
| Rows carry the clinical boundary | true |
| Rows remain blocked | true |
| Bridge rows <= 1,000 | true |

## Native Calyx Materialization

```text
name: issue1249-openfda-faers-safety-expansion-20260704t203500z
vault_id: 01KWQE3VV0ZZ3ACPVHXM8T5547
vault_dir: /home/croyse/calyx/vaults/01KWQE3VV0ZZ3ACPVHXM8T5547
```

Materialization readback:

| Item | Count / value |
|---|---:|
| Bridge rows | 812 |
| Bridge terms | 547 |
| Graph nodes | 1,359 |
| Graph edges | 6,800 |
| CSR persisted | true |
| Active vault index contains final name exactly once | true |
| Graph nodes match readback | true |
| Graph edges match readback | true |
| Vault manifest/CURRENT files present | true |
| Graph files present | true |

## Result

#1249 expanded independent safety-source coverage beyond openFDA Human Drug
Labels by adding FAERS event co-report triage. It found 45 source-backed FAERS
event evidence rows covering 82 of the 363 #1248 rollups; 79 rollups were tied
to serious-event co-report rows. Those rows are blockers/review inputs only,
not safety clearance or actionability.

No safety clearance, contraindication guidance, treatment guidance, dosing
guidance, clinical recommendation, clinical actionability, efficacy claim, or
cure claim is made.
