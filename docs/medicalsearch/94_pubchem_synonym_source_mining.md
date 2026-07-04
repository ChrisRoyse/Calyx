# #1245 PubChem Synonym Source Mining

Status: complete for the PubChem synonym/equivalence source-mining pass.

This slice continued #1243 after the Europe PMC pair-search no-hit remainder by
querying a distinct source instrument: PubChem PUG-REST compound synonym
records. A PubChem hit required returned structured synonym text to physically
contain both candidate terms or accepted normalized equivalents. CID existence,
query success, and synonym count alone were not treated as evidence.

Clinical boundary:

```text
PubChem synonym/equivalence source mining is chemical identity/source triage only; not efficacy, safety, treatment guidance, dosing guidance, recommendation, clinical actionability, pair-interaction evidence, or cure evidence.
```

## Inputs

FSV root:

```text
/home/croyse/calyx/fsv/issue1245-pubchem-synonym-source-mining-20260704T210000Z
```

Sealed upstream inputs:

```text
/home/croyse/calyx/fsv/issue1243-europepmc-pair-search-20260704T181500Z/out/candidate_europepmc_status.jsonl
sha256: 7a3b63dea0ff880374b68627e9465d1d11d1e7758e1bcf6cec59050611746c03

/home/croyse/calyx/fsv/issue1243-europepmc-pair-search-20260704T181500Z/out/europepmc_pair_status.jsonl
sha256: 153b7f222495f25244d6e221b5cc46ca1830335a12759867328b9a26b2384f9d

/home/croyse/calyx/fsv/issue1243-europepmc-pair-search-20260704T181500Z/out/persisted_readback.json
sha256: 37e6ca92f1a3bfc5f0945f64c538bc38e96911a1281951fe92faf3f13828717a
```

Persisted source/API documentation:

| Source doc | Bytes | SHA-256 |
|---|---:|---|
| `pubchem_pug_rest_docs.html` | 5,134 | `b2de3f40fd32fe75d4adb7e9217745c1451c7229f83f55a08324f85598280dda` |
| `pubchem_pug_view_docs.html` | 5,384 | `073b169762c48b4e89866159cee0be3feae713634bfa52ba54b5fdcce3428a8b` |
| `pubchem_programmatic_access.html` | 5,211 | `f02b9d1c0b9d80df4142349d3059cb00111475c5e3f485952f9e6af2d1c0c175` |

Source contract:

- Input scope was the 532 #1243 candidate rows with
  `overall_external_source_status_after_issue1243 == no_external_hit`.
- Those candidates represented 353 unique pair keys and 152 unique query terms.
- Each term was queried against the PubChem synonym endpoint.
- A pair evidence row required PubChem returned synonym text to contain both
  terms, not merely a CID match for one component.
- All rows remained blocked behind the clinical boundary.

## Output Artifacts

| Artifact | Rows | Bytes | SHA-256 |
|---|---:|---:|---|
| `pubchem_synonym_query_responses.jsonl` | 152 | 467,160 | `4eb6bc219d81a9282d856cfd45fcd7e4bfefe8705538b279b1896c1b8ad6a347` |
| `pubchem_pair_evidence.jsonl` | 0 | 0 | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |
| `pubchem_pair_status.jsonl` | 353 | 417,290 | `60f313a4eb57de2845f2f8a6d9f55a203655c1f53eb593dc6d907ec15aa4002e` |
| `candidate_pubchem_status.jsonl` | 532 | 740,827 | `299efd7e94b82ee99bf8112cc4f879d642b6e507f6d3f0be160edc674a17b179` |
| `pubchem_bridge_rows.jsonl` | 885 | 1,235,844 | `7b4dd92cc7ec43fb4436e182386b62242139a5684cb8159403d14302ca8897ba` |
| `input_manifest.json` | - | 3,197 | `69523c62285f1408d8541bb5bc155931115082d74a41f3b9e9836c3581f74823` |
| `validation_metrics.json` | - | 898 | `f833cbfce8f26be743344eb9dc557e74e6df088d62ee4fdda14166595a72638f` |
| `output_manifest.json` | - | 2,308 | `05076f2c461c31e5726122020804a01d1a7e8ae0c5635d392ffac93ee5d168d4` |
| `persisted_readback.json` | - | 3,511 | `037a33ca5b1828e03b948d891763d5abf5726b5c931016b52e6746749587f9dc` |
| `calyx_bridge_corpus_stdout.json` | - | 710 | `a559e09bf0f4d3509d44a793f040b33656bc69143c3d1e491300627bf06dd4cf` |
| `calyx_bridge_corpus_readback.json` | - | 5,678 | `43598ce544f856b2d81385b8376c134d074c1eaa37cbca44d061986577e5102a` |

## Metrics

| Metric | Count |
|---|---:|
| #1243 no-hit candidate rows checked | 532 |
| Unique pair keys checked | 353 |
| Unique PubChem term queries | 152 |
| PubChem terms with CID/synonym records | 103 |
| PubChem pair evidence rows | 0 |
| Pair status rows | 353 |
| Candidate status rows | 532 |

Query HTTP status counts:

| HTTP status | Term queries |
|---|---:|
| `200` | 103 |
| `404` | 49 |

Pair status counts:

| Status | Pair rows |
|---|---:|
| `pubchem_synonym_record_without_pair_match_still_blocked` | 298 |
| `pubchem_synonym_no_result_still_blocked` | 55 |

Candidate status counts:

| Status | Candidate rows |
|---|---:|
| `pubchem_synonym_record_without_pair_match_still_blocked` | 475 |
| `pubchem_synonym_no_result_still_blocked` | 57 |

Validation assertions:

| Assertion | Result |
|---|---|
| #1243 persisted readback all true | true |
| #1243 Calyx readback all true | true |
| Query response for every unique term | true |
| Pair status for every pair key | true |
| Candidate status for every #1243 no-hit candidate | true |
| All PubChem hits have evidence rows | true |
| Evidence rows have source hashes | true |
| Evidence rows have pair terms | true |
| Pair/candidate status values allowed | true |
| Rows carry the clinical boundary | true |
| Rows remain blocked | true |
| Bridge rows <= 1,000 | true |

## Native Calyx Materialization

```text
name: issue1245-pubchem-synonym-source-mining-20260704t210000z
vault_id: 01KWQF43ADQWB6WCY5DMRR25F2
vault_dir: /home/croyse/calyx/vaults/01KWQF43ADQWB6WCY5DMRR25F2
```

Materialization readback:

| Item | Count / value |
|---|---:|
| Bridge rows | 885 |
| Bridge terms | 507 |
| Graph nodes | 1,392 |
| Graph edges | 7,080 |
| CSR persisted | true |
| Active vault index contains final name exactly once | true |
| Graph nodes match readback | true |
| Graph edges match readback | true |
| Vault manifest/CURRENT files present | true |
| Graph files present | true |

## Result

#1245 expanded external source coverage for the #1243 no-hit remainder by
checking PubChem synonym/equivalence records. PubChem returned 103 term records,
but no pair satisfied the physical two-term synonym/equivalence gate, so there
were zero PubChem pair evidence rows. The 532 carried candidate rows remain
blocked and now have explicit PubChem no-hit/no-pair-match status rows.

No efficacy, safety, treatment guidance, dosing guidance, clinical
recommendation, clinical actionability, pair-interaction proof, or cure claim is
made.
