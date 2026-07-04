# #1254 ChEMBL Source Mining

Status: complete for the ChEMBL molecule-search source-mining pass.

This slice continued #1245 after the PubChem synonym/equivalence no-hit result
by querying a distinct source instrument: ChEMBL REST molecule search records.
A ChEMBL hit required a returned structured molecule record to physically
contain both candidate terms or accepted normalized equivalents. Search count,
single-term matches, and returned molecule records were not treated as evidence
unless both terms appeared in the same returned record.

Clinical boundary:

```text
ChEMBL source mining is molecule/source triage only; not efficacy, safety, treatment guidance, dosing guidance, recommendation, clinical actionability, pair-interaction evidence, or cure evidence.
```

## Inputs

FSV root:

```text
/home/croyse/calyx/fsv/issue1254-chembl-source-mining-20260704T213500Z
```

Sealed upstream inputs:

```text
/home/croyse/calyx/fsv/issue1245-pubchem-synonym-source-mining-20260704T210000Z/out/candidate_pubchem_status.jsonl
sha256: 299efd7e94b82ee99bf8112cc4f879d642b6e507f6d3f0be160edc674a17b179

/home/croyse/calyx/fsv/issue1245-pubchem-synonym-source-mining-20260704T210000Z/out/pubchem_pair_status.jsonl
sha256: 60f313a4eb57de2845f2f8a6d9f55a203655c1f53eb593dc6d907ec15aa4002e

/home/croyse/calyx/fsv/issue1245-pubchem-synonym-source-mining-20260704T210000Z/out/persisted_readback.json
sha256: 037a33ca5b1828e03b948d891763d5abf5726b5c931016b52e6746749587f9dc

/home/croyse/calyx/fsv/issue1245-pubchem-synonym-source-mining-20260704T210000Z/out/calyx_bridge_corpus_readback.json
sha256: 43598ce544f856b2d81385b8376c134d074c1eaa37cbca44d061986577e5102a
```

Persisted source/API documentation:

| Source doc | Bytes | SHA-256 |
|---|---:|---|
| `chembl_rest_docs.html` | 2,520 | `17427b9bb56f08d217a76e1a8cd3ea07b89f5ec1ae096194222bc625c63e718b` |
| `chembl_molecule_schema_sample.json` | 3,815 | `f9eed1e4504b917dbc6f54de8ce5ae2094e16ea5773a88c36a38f1df2562a163` |

Source contract:

- Input scope was the 532 #1245 candidate rows still blocked after PubChem.
- Those candidates represented 353 unique pair keys.
- Each pair key was queried against ChEMBL molecule search using both component
  names in the query.
- A pair evidence row required a returned ChEMBL molecule record to contain
  both pair terms or accepted normalized equivalents in the same record.
- All rows remained blocked behind the clinical boundary.

## Output Artifacts

| Artifact | Rows | Bytes | SHA-256 |
|---|---:|---:|---|
| `chembl_molecule_query_responses.jsonl` | 353 | 402,552 | `14cd4c625b6a0a5a8859b9ad06aa31fa626ebc84825a83e525b8f169b8d46260` |
| `chembl_pair_evidence.jsonl` | 0 | 0 | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |
| `chembl_pair_status.jsonl` | 353 | 628,835 | `4ea75759362b16db5a3470605d6321d93f2301fa4954b60bd02dc81c5bd2861f` |
| `candidate_chembl_status.jsonl` | 532 | 760,286 | `761ef7b2c8f644fe20b2cb6c2eee1f88f06d8116c0ccfe86a37cbb5b13156689` |
| `chembl_bridge_rows.jsonl` | 885 | 1,183,419 | `893d8c2c9c6691dd46a440d3a6c55c76c25da30c73ddbd30c2cc064f7a79ffea` |
| `input_manifest.json` | - | 2,838 | `13db9f1aa88813a937ed5aa63a305f5f9b041672582e7fe358c3ce31009af3a0` |
| `validation_metrics.json` | - | 962 | `632db838f80603468fdb7284f7470974920298bf5476cfcf0b9cdc2a0cb4bcbd` |
| `output_manifest.json` | - | 2,207 | `6edd4d83adfbebf756de9bf3097944e70f257b1a1de24f0565823d465b8e45a1` |
| `persisted_readback.json` | - | 3,384 | `bffda29de180a185d13f04fc23aafe9ce2837af12db7b26e91d41f01327edfa9` |
| `calyx_bridge_corpus_stdout.json` | - | 673 | `39e4f1fbcb55b8018a31e1b3bfebbb95f4b27bed3000ba90f004991393bb5dfd` |
| `calyx_bridge_corpus_readback.json` | - | 5,505 | `d50c44cab39c975e71d85abe66ee63eeb666b0fb963df0aa91745541b22b747f` |

## Metrics

| Metric | Count |
|---|---:|
| #1245 blocked candidate rows checked | 532 |
| Unique pair keys checked | 353 |
| ChEMBL molecule-search pair queries | 353 |
| ChEMBL queries with returned records | 339 |
| ChEMBL total records returned | 5,377 |
| ChEMBL pair evidence rows | 0 |
| Pair status rows | 353 |
| Candidate status rows | 532 |

Query HTTP status counts:

| HTTP status | Pair queries |
|---|---:|
| `200` | 353 |

Pair status counts:

| Status | Pair rows |
|---|---:|
| `chembl_pair_record_without_pair_match_still_blocked` | 339 |
| `chembl_pair_no_result_still_blocked` | 14 |

Candidate status counts:

| Status | Candidate rows |
|---|---:|
| `chembl_candidate_record_without_pair_match_still_blocked` | 487 |
| `chembl_candidate_no_result_still_blocked` | 45 |

Validation assertions:

| Assertion | Result |
|---|---|
| #1245 persisted readback all true | true |
| #1245 Calyx readback all true | true |
| Query response for every pair key | true |
| Pair status for every pair key | true |
| Candidate status for every candidate | true |
| All ChEMBL hits have evidence rows | true |
| Evidence rows have source hashes | true |
| Evidence rows have pair terms | true |
| Pair/candidate status values allowed | true |
| Rows carry the clinical boundary | true |
| Rows remain blocked | true |
| Bridge rows <= 1,000 | true |

## Native Calyx Materialization

```text
name: issue1254-chembl-source-mining-20260704t213500z
vault_id: 01KWQHTFE770MJKC5T8AMCT780
vault_dir: /home/croyse/calyx/vaults/01KWQHTFE770MJKC5T8AMCT780
```

Materialization readback:

| Item | Count / value |
|---|---:|
| Bridge rows | 885 |
| Bridge terms | 509 |
| Graph nodes | 1,394 |
| Graph edges | 7,080 |
| CSR persisted | true |
| Active vault index contains final name exactly once | true |
| Graph nodes match readback | true |
| Graph edges match readback | true |
| Vault manifest/CURRENT files present | true |
| Graph files present | true |

## Result

#1254 expanded external source coverage for the #1245 blocked remainder by
checking ChEMBL molecule-search records. ChEMBL returned 5,377 molecule records
across 339 pair queries, but no returned molecule record satisfied the physical
same-record two-term gate, so there were zero ChEMBL pair evidence rows. The
532 carried candidate rows remain blocked and now have explicit ChEMBL
no-result/no-pair-match status rows.

No efficacy, safety, treatment guidance, dosing guidance, clinical
recommendation, clinical actionability, pair-interaction proof, or cure claim is
made.
