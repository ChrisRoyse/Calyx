# #1243 Europe PMC Pair-Search Source Mining

Status: complete.

This slice continued external source mining after #1242 by rechecking the
remaining no-hit combination-candidate universe against Europe PMC Articles
REST API pair search and bounded PMCID full-text XML checks.

Clinical boundary:

```text
Europe PMC pair-search evidence is source-attributed literature/index co-mention only; not efficacy, safety, treatment guidance, dosing guidance, recommendation, clinical actionability, or cure evidence.
```

## Inputs

FSV root:

```text
/home/croyse/calyx/fsv/issue1243-europepmc-pair-search-20260704T181500Z
```

Sealed upstream input:

```text
/home/croyse/calyx/fsv/issue1242-dailymed-spl-title-mining-20260704T174500Z/out/candidate_dailymed_title_status.jsonl
sha256: e834783552a1b94172b1b424f15f9f6403ada53439e85a3bea67edd458f0bd26
```

Source contract:

- The input filter was `overall_external_source_status_after_issue1242 == no_external_hit`.
- Input rows after the filter: 1,019.
- Unique pair keys queried: 640.
- Source: Europe PMC Articles REST API `/search`.
- Query mode: `"<drug_a>" AND "<drug_b>"`, `format=json`, `resultType=core`, `pageSize=5`, `cursorMark=*`, `synonym=false`.
- Search hit counts were not promoted by themselves. A hit required both candidate terms to be physically present in returned metadata text or bounded fetched PMCID full-text XML.
- The source is literature/index co-mention only; it does not prove relation direction, mechanism, safety, efficacy, treatment actionability, or cure evidence.

## Method

The miner:

- persisted official Europe PMC REST, annotations, developer, and about pages;
- queried each remaining pair key once through Europe PMC Articles REST search;
- recorded every raw JSON response hash and schema fingerprint;
- checked returned title, abstract, keywords, publication types, journal metadata, and related metadata text for both terms;
- fetched up to three PMCID `fullTextXML` records per pair when metadata did not verify both terms;
- did not cache failed full-text responses;
- wrote exact/normalized/no-hit status rows with blocked promotion state;
- wrote a 1,000-row bridge-corpus slice for native Calyx materialization.

## Source Artifacts

| Source artifact | Bytes | SHA-256 |
|---|---:|---|
| `raw/europepmc_rest_docs.html` | 64,486 | `e3b45ced2caf9a35496a33ffbe79e745f79a48ba61d2fbc6996cf94d6bce4c97` |
| `raw/europepmc_annotations_docs.html` | 57,216 | `d8109ea920e0f5d0a13de65bb315125911fb22c020b06167c1346b32b9b15cc7` |
| `raw/europepmc_developers.html` | 54,465 | `d7f25854429706dc9a25b009a4616fadd8332ee47375917ab264974e3edbeace` |
| `raw/europepmc_about.html` | 75,965 | `69366291c25e6e895353dd6f038c5e747314678815964a1b131c603f4808eee1` |

Source URLs:

- https://europepmc.org/RestfulWebService
- https://europepmc.org/AnnotationsApi
- https://europepmc.org/developers
- https://europepmc.org/About

## Output Artifacts

| Artifact | Rows | Bytes | SHA-256 |
|---|---:|---:|---|
| `europepmc_pair_query_responses.jsonl` | 640 | 9,755,246 | `f7e367b08b4558443abd7ec82fc8b76be9b05db5518daa61e03b9228cbc8630b` |
| `europepmc_fulltext_fetches.jsonl` | 772 | 865,857 | `5acc828a3c8e6347a23adcafaeb12764fb3a5b40a2e2cc6aa979cef0003b4e35` |
| `europepmc_pair_evidence.jsonl` | 598 | 1,209,359 | `a393992fb1e5b0d8a83f6f99e27e6b1c6aeeb5563615bb2a729b0602075aa609` |
| `europepmc_pair_status.jsonl` | 640 | 847,926 | `153b7f222495f25244d6e221b5cc46ca1830335a12759867328b9a26b2384f9d` |
| `candidate_europepmc_status.jsonl` | 1,019 | 1,375,255 | `7a3b63dea0ff880374b68627e9465d1d11d1e7758e1bcf6cec59050611746c03` |
| `europepmc_bridge_rows.jsonl` | 1,000 | 1,190,452 | `785cd4bcb6c34b59bcff9d90290792fd4cc0a100ac7032ca4b9514471aadda73` |
| `input_manifest.json` | - | 4,018 | `1b528f52edae5cd3c323e7a1aaa0b862b4c337ac2836014dd1e614a594f279cb` |
| `validation_metrics.json` | - | 9,018 | `f3c4744fd405106951d93d04ade4bdf553632ed386456d9ea3b0ff6859c44abc` |
| `output_manifest.json` | - | 2,590 | `04e774bc62d15372012440258a53fb5f449e83199cb9d3c98e82faa9830569ba` |
| `persisted_readback.json` | - | 3,891 | `37e6ca92f1a3bfc5f0945f64c538bc38e96911a1281951fe92faf3f13828717a` |
| `calyx_bridge_corpus_stdout.json` | - | 683 | `210746fe8274746a1da591960ade38f64b0f61f2aaa3d745a9b89b9e95b65dbc` |
| `calyx_bridge_corpus_readback.json` | - | 5,103 | `781c312979e4a17f12654f8e550cf7127e2c08b169c59c231685e33ec9de0fed` |

## Metrics

| Metric | Count |
|---|---:|
| #1242 remaining no-hit candidate rows checked | 1,019 |
| Unique pair keys queried | 640 |
| Europe PMC query response rows | 640 |
| Search responses HTTP 200 | 640 |
| Pair keys with Europe PMC search hit count > 0 | 317 |
| Total Europe PMC search hit count | 6,140 |
| Returned result rows inspected | 1,137 |
| Full-text fetch rows | 772 |
| Full-text HTTP 200 rows | 649 |
| Full-text HTTP 404 rows | 123 |
| Verified Europe PMC evidence rows | 598 |
| Metadata-text evidence rows | 1 |
| PMCID full-text XML evidence rows | 597 |
| Candidate rows with #1243 hit | 487 |
| Remaining no-hit candidate rows after #1243 | 532 |

Status counts:

| Status scope | `exact_hit` | `normalized_hit` | `no_external_hit` |
|---|---:|---:|---:|
| Pair status rows | 280 | 7 | 353 |
| Candidate status rows | 474 | 13 | 532 |

Top evidence-count examples:

| Pair key | Status | Evidence rows | Search hit count | Source ids |
|---|---|---:|---:|---|
| `levetiracetam||thymidine` | `exact_hit` | 4 | 179 | `PMC11910025`, `PMC12332248`, `PMC12648844`, `PMC13099364` |
| `gentamicin||sunitinib` | `exact_hit` | 3 | 276 | `PMC11764070`, `PMC12969023`, `PMC13024260` |
| `alpelisib||rituximab` | `exact_hit` | 3 | 219 | `PMC11825581`, `PMC11946485`, `PMC13111025` |
| `dactolisib||metformin` | `exact_hit` | 3 | 144 | `PMC11442590`, `PMC13156086`, `PMC13238955` |
| `gentamicin||vemurafenib` | `exact_hit` | 3 | 128 | `PMC11601706`, `PMC12191169`, `PMC13158949` |

Validation assertions:

| Assertion | Result |
|---|---|
| #1242 persisted readback all true | true |
| #1242 Calyx readback all true | true |
| Candidate status rows cover every remaining no-hit row | true |
| Pair status rows cover every unique pair key | true |
| Query response exists for every queryable pair key | true |
| All query HTTP statuses are 200 | true |
| All status values are allowed | true |
| All status rows carry the clinical boundary | true |
| All hits have evidence | true |
| Full-text matches have evidence | true |
| All evidence rows have source ids | true |
| All candidate rows remain blocked | true |
| Bridge rows <= 1,000 | true |

## Native Calyx Materialization

```text
name: issue1243-europepmc-pair-search-20260704t181500z
vault_id: 01KWQ6CE6VEN48FFP2TDD4ESV0
vault_dir: /home/croyse/calyx/vaults/01KWQ6CE6VEN48FFP2TDD4ESV0
```

Materialization readback:

| Item | Count / value |
|---|---:|
| Bridge rows | 1,000 |
| Bridge terms | 1,109 |
| Graph nodes | 2,109 |
| Graph edges | 9,440 |
| CSR persisted | true |
| Active vault index contains final name exactly once | true |
| Graph nodes match readback | true |
| Graph edges match readback | true |
| Vault manifest/CURRENT files present | true |
| Graph files present | true |

## Result

Europe PMC pair-search source mining converted 487 of the 1,019 #1242
remaining no-hit candidate rows into source-attributed co-mention hits, with
598 verified evidence rows and native Calyx graph materialization. Another 532
candidate rows remain `no_external_hit`.

These rows remain research triage only. Promotion requires source-text relation
extraction, safety/outcome/falsification gates, and human review. The remaining
no-hit rows should continue through source expansion from
`candidate_europepmc_status.jsonl`, filtered to
`overall_external_source_status_after_issue1243 == no_external_hit`.
