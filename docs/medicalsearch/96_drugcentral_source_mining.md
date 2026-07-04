# #1255 DrugCentral Source Mining

Status: complete for the DrugCentral source-mining pass.

This slice continued #1254 after the ChEMBL no-hit result by snapshotting
DrugCentral source tables and checking structured drug-drug interaction rows
plus same-structure equivalence mappings. A hit required a DrugCentral DDI row
whose two participants normalized to the pair terms, or both pair terms mapping
to the same DrugCentral structure id. Search count, single-term mappings, and
source-table presence alone were not treated as evidence.

Clinical boundary:

```text
DrugCentral source mining is drug/source triage only; interaction rows are blockers/review inputs, not safety clearance, efficacy, treatment guidance, dosing guidance, recommendation, clinical actionability, pair-interaction proof, or cure evidence.
```

## Inputs

FSV root:

```text
/home/croyse/calyx/fsv/issue1255-drugcentral-source-mining-20260704T222500Z
```

Sealed upstream inputs:

```text
/home/croyse/calyx/fsv/issue1254-chembl-source-mining-20260704T213500Z/out/candidate_chembl_status.jsonl
sha256: 761ef7b2c8f644fe20b2cb6c2eee1f88f06d8116c0ccfe86a37cbb5b13156689

/home/croyse/calyx/fsv/issue1254-chembl-source-mining-20260704T213500Z/out/chembl_pair_status.jsonl
sha256: 4ea75759362b16db5a3470605d6321d93f2301fa4954b60bd02dc81c5bd2861f

/home/croyse/calyx/fsv/issue1254-chembl-source-mining-20260704T213500Z/out/persisted_readback.json
sha256: bffda29de180a185d13f04fc23aafe9ce2837af12db7b26e91d41f01327edfa9

/home/croyse/calyx/fsv/issue1254-chembl-source-mining-20260704T213500Z/out/calyx_bridge_corpus_readback.json
sha256: d50c44cab39c975e71d85abe66ee63eeb666b0fb963df0aa91745541b22b747f
```

Persisted source/API documentation:

| Source doc | Bytes | SHA-256 |
|---|---:|---|
| `drugcentral_download.html` | 14,175 | `a7ba9349a89f5986ed487c02688df47596c30f1f3721a199d852e7b59d76553a` |
| `drugcentral_active_download.html` | 9,095 | `193ac07195c08d0b52e0ff8a453892cb0597e447503ff4033dc6d40bae850f47` |
| `drugcentral_api_docs.html` | 943 | `4e18b81a68eee4e54babf520451ad6421ae4ba3cda16d50a5dcdcbbb31e3b80b` |
| `drugcentral_openapi.json` | 74,598 | `be98c09d44ca9279e47aa8d2ea56c0becbda8edbab3e3eab1add7c3125812e0f` |
| `drugcentral_schema.csv` | 3,321 | `8e8647fd00dfd4ac962086dce6b5d9ca36761c11242bfab03626eb7dcaae9ee1` |
| `drugcentral_counts.csv` | 148 | `6e73cf5f13c5420c43395dd9245e11249ca36aaab5e78ecf22d6c6dee4aeac1b` |

Runtime note: DrugCentral database access was supplied through the run
environment; credentials were not persisted in repo artifacts.

Source table snapshots:

| Table | Rows | Bytes | SHA-256 |
|---|---:|---:|---|
| `ddi` | 7,621 | 775,132 | `9f9abc65ffbfca813962c1c6fa270b38f701f3c57d80838d5f5c049a9719ac9f` |
| `ddi_risk` | 6 | 146 | `7791f63f73a6160458ef01c2da61aecb013224a713e05a86f6fbb43b9598c7a4` |
| `structures` | 4,995 | 540,863 | `d0b59bbce25f3693563e6d4baf7b7cb38f2dbb77a0b65a0e76c7ee7bd33a293f` |
| `synonyms` | 23,369 | 978,738 | `a6db6cb7ccbabe4ec9eaeeded2c2015612d24e192b91d57d6007b3c8317252d6` |
| `identifier` | 82,230 | 2,610,309 | `658c8699871056ea7f8d770056f48d7481996caa01964dfa0a40e4d94d53db39` |
| `approval` | 3,915 | 150,580 | `f84fec5f2c321d5c321b3aee6f39935649673911bc35120332dbdde9b4f61c65` |
| `omop_relationship` | 42,307 | 4,211,849 | `9fe3ddb5cbdcf91998610cd19ee193c2e536f6b7e456f083df6963ea33e99271` |
| `act_table_full` | 20,978 | 4,531,558 | `4760de223d667361886927af6b3d46aa786b2b061503bd9acece239193d422bb` |

Source contract:

- Input scope was the 532 #1254 candidate rows still blocked after ChEMBL.
- Those candidates represented 353 unique pair keys.
- Structured DDI evidence required both pair terms to match the two DDI
  participants by exact normalized text or DrugCentral structure-id mapping.
- Same-structure evidence required both pair terms to resolve to the same
  DrugCentral structure id through structures, synonyms, or identifiers.
- All rows remained blocked behind the clinical boundary.

## Output Artifacts

| Artifact | Rows | Bytes | SHA-256 |
|---|---:|---:|---|
| `drugcentral_source_rows.jsonl` | 10 | 7,472 | `cb3730a4d53cbd18704f89821e801f5f8c19a2e3a6d0bdb05b336458912036d8` |
| `drugcentral_pair_evidence.jsonl` | 0 | 0 | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |
| `drugcentral_pair_status.jsonl` | 353 | 529,705 | `07caa8201d7a9367864a52af4bae57afd9822e3e52aa69721e89f250cfe2ed81` |
| `candidate_drugcentral_status.jsonl` | 532 | 883,529 | `9dce3e78548d959b56b08f3a74f8e69ec936673f0d5b78924c387315c6eca099` |
| `drugcentral_bridge_rows.jsonl` | 895 | 1,285,878 | `eadfceb72720f9c0b67eebab52585ebc83b2fd63ad9d27be0f1d4008215f31e5` |
| `input_manifest.json` | - | 6,396 | `7bec2b5a9216fffe4608ba9ce30589202af5b89d91b9d52693ddb85a0521e4cb` |
| `validation_metrics.json` | - | 1,248 | `43c6588b5f75daf381ead3be51cf5374fe4da176b1976a05001f4f034881d34a` |
| `output_manifest.json` | - | 2,317 | `92e58d075a79e166cc3bfd05f24305640eadbc09949a3504beebd8e3b4353124` |
| `persisted_readback.json` | - | 3,552 | `661ef64c2daf25e5eb7483eeeb4754aa93fc2f83e223a7b8d114c14ed5fa9478` |
| `calyx_bridge_corpus_stdout.json` | - | 731 | `f2d034f296964cc44ffef482a8f78eb42cdc217e4791825af26d53cd480f2c6e` |
| `calyx_bridge_corpus_readback.json` | - | 5,696 | `c2c421392bb43469bbf7dc8f9be106c794b8c6697a39ceaf01c781ad225203bf` |

## Metrics

| Metric | Count |
|---|---:|
| #1254 blocked candidate rows checked | 532 |
| Unique pair keys checked | 353 |
| DrugCentral DDI source rows checked | 7,621 |
| DrugCentral structure rows checked | 4,995 |
| DrugCentral synonym rows checked | 23,369 |
| DrugCentral identifier rows checked | 82,230 |
| DrugCentral pair evidence rows | 0 |
| Pair status rows | 353 |
| Candidate status rows | 532 |

Pair status counts:

| Status | Pair rows |
|---|---:|
| `drugcentral_single_term_mappings_without_pair_match_still_blocked` | 189 |
| `drugcentral_no_term_mapping_still_blocked` | 164 |

Candidate status counts:

| Status | Candidate rows |
|---|---:|
| `drugcentral_candidate_single_term_mappings_without_pair_match_still_blocked` | 287 |
| `drugcentral_candidate_no_term_mapping_still_blocked` | 245 |

Validation assertions:

| Assertion | Result |
|---|---|
| #1254 persisted readback all true | true |
| #1254 Calyx readback all true | true |
| Source rows present | true |
| Pair status for every pair key | true |
| Candidate status for every candidate | true |
| All DrugCentral hits have evidence rows | true |
| DDI evidence rows have participant matches | true |
| Same-structure evidence rows have shared structure | true |
| Evidence rows have source hashes | true |
| Pair/candidate status values allowed | true |
| Rows carry the clinical boundary | true |
| Rows remain blocked | true |
| Bridge rows <= 1,000 | true |

## Native Calyx Materialization

```text
name: issue1255-drugcentral-source-mining-20260704t222500z
vault_id: 01KWQJX8XZJF7C1GF91699YM3Y
vault_dir: /home/croyse/calyx/vaults/01KWQJX8XZJF7C1GF91699YM3Y
```

Materialization readback:

| Item | Count / value |
|---|---:|
| Bridge rows | 895 |
| Bridge terms | 540 |
| Graph nodes | 1,435 |
| Graph edges | 7,160 |
| CSR persisted | true |
| Active vault index contains final name exactly once | true |
| Graph nodes match readback | true |
| Graph edges match readback | true |
| Vault manifest/CURRENT files present | true |
| Graph files present | true |

## Result

#1255 expanded external source coverage for the #1254 blocked remainder by
checking DrugCentral structures, synonyms, identifiers, approvals, indications,
activity rows, and 7,621 structured DDI rows. No candidate pair satisfied the
structured DDI participant gate or same-structure equivalence gate, so there
were zero DrugCentral pair evidence rows. The 532 carried candidate rows remain
blocked and now have explicit DrugCentral no-term-mapping or single-term-only
status rows.

No efficacy, safety clearance, treatment guidance, dosing guidance, clinical
recommendation, clinical actionability, pair-interaction proof, or cure claim is
made.
