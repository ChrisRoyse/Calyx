# #1240 RxNorm Combination-Product Source Mining

Status: complete.

This slice continued external source mining after #1236 by rechecking the
remaining no-hit combination-candidate universe against current NLM RxNorm
combination-product concept search.

Clinical boundary:

```text
RxNorm combination-product concept evidence is source-attributed vocabulary/product-concept mining only; not efficacy, safety, pair interaction clearance, treatment guidance, dosing guidance, recommendation, clinical actionability, or cure evidence.
```

## Inputs

FSV root:

```text
/home/croyse/calyx/fsv/issue1240-rxnorm-combination-products-20260704T172703Z
```

Sealed upstream input:

```text
/home/croyse/calyx/fsv/issue1236-openfda-label-source-mining-20260704T170534Z/out/candidate_openfda_label_status.jsonl
sha256: 0e8893d8bef722f66ae656ece2335a7f7eabcb226bf429c6658b0e5ad9ee791b
```

Source contract:

- The input filter was `overall_external_source_status_after_issue1236 == no_external_hit`.
- Input rows after the filter: 1,019.
- Unique pair keys queried: 640.
- Source: NLM RxNorm API `/REST/drugs.json`.
- RxNorm API version readback: `3.1.353`.
- RxNorm data version readback: `01-Jun-2026`.
- RxNav rate-limit contract used for scheduling: 20 requests/second/IP.
- The discontinued RxNav drug/drug interaction API was explicitly excluded.

## Method

The miner:

- persisted official RxNorm/RxNav API, terms, FAQ, overview, and version source bytes;
- queried each pair key in both deterministic directions, `drug_a / drug_b` and `drug_b / drug_a`;
- required both candidate names to appear in the returned RxNorm concept name or synonym before marking a hit;
- wrote exact/normalized/no-hit status rows with raw response hashes;
- preserved all candidate rows as blocked research triage rows;
- wrote a 640-row bridge-corpus slice for native Calyx materialization.

Each `rxnorm_combination_query_responses.jsonl` row represents one pair key and
contains both directional HTTP responses. The run made 1,280 HTTP requests, all
HTTP 200, and returned zero RxNorm concept rows for this candidate universe.

## Source Artifacts

| Source artifact | Bytes | SHA-256 |
|---|---:|---|
| `raw/rxnorm_api_docs.html` | 21,042 | `d037f07cac2e2f18225cff27f792c2d0133d945d67b175108d0866d4acfd49d8` |
| `raw/rxnav_terms.html` | 16,041 | `eebd61ce17cc426e231c159b10a925bdd4b39744141060d85acba5fe90b22911` |
| `raw/rxnav_faq.html` | 15,033 | `c5504507b52c82d079ffd3fd1e6761a212c5a36feafc09a5874020510221f05d` |
| `raw/rxnav_overview.html` | 26,791 | `2b58996bd16679950ac8916f68493641292d5a3dc1a0e007eaee3864dae6eb64` |
| `raw/rxnorm_version.json` | 48 | `dea926aeb5b147381336d15114555883ad9759d31056e57bc4818b26c3d9a461` |

Source URLs:

- https://lhncbc.nlm.nih.gov/RxNav/APIs/RxNormAPIs.html
- https://lhncbc.nlm.nih.gov/RxNav/TermsofService.html
- https://lhncbc.nlm.nih.gov/RxNav/information/FAQs.html
- https://lhncbc.nlm.nih.gov/RxNav/
- https://rxnav.nlm.nih.gov/REST/version.json

## Output Artifacts

| Artifact | Rows | Bytes | SHA-256 |
|---|---:|---:|---|
| `rxnorm_combination_query_responses.jsonl` | 640 | 959,263 | `f3c33477764790aa35f7a3bcc5f3d7c92b988ba1ff033b7a2369f3801c6c86c3` |
| `rxnorm_combination_evidence.jsonl` | 0 | 0 | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |
| `rxnorm_pair_status.jsonl` | 640 | 876,665 | `68c2d136ff82dc6ded1c141e17040197e9cd264914fa6594ae4f73737e02bfc1` |
| `candidate_rxnorm_status.jsonl` | 1,019 | 1,292,381 | `5293210308d19be37c976d55a34813352c7f473da152be6348991e35deb829f7` |
| `rxnorm_combination_bridge_rows.jsonl` | 640 | 829,487 | `39273aaddc150db2af62fffc95f5fceadf4176f165d2cecb0a8fd17db233c9f8` |
| `input_manifest.json` | - | 4,610 | `9ed6c4d205cf9e8e80f665651d11634d99fb76798e68ec1eb0521c6aa240d368` |
| `validation_metrics.json` | - | 1,063 | `a69afe7b745275e7326b66556d3a2907c766f52740d60463318863a5bdfe69f8` |
| `output_manifest.json` | - | 2,395 | `0687c62e326d010b98f905fb59bd8b33c0d1b4eb6df426bd3ef0cf6d66541b67` |
| `persisted_readback.json` | - | 3,581 | `b0be6e9b90c6cb9220e1ae0209ae9390ef89c12d29258ab00fbad4c81c0797e9` |
| `calyx_bridge_corpus_stdout.json` | - | 686 | `b3c0ba3daea69fb3be6254d5ac6cf6478670ebf1ef32b0bd65e1c8136c6ee61f` |
| `calyx_bridge_corpus_readback.json` | - | 3,812 | `b56967b025bf45d4befee9060012da8e4374a0a81c2e320ba558bf53cb87aef4` |

## Metrics

| Metric | Count |
|---|---:|
| #1236 remaining no-hit candidate rows checked | 1,019 |
| Unique pair keys queried | 640 |
| Directional HTTP requests | 1,280 |
| HTTP 200 responses | 1,280 |
| Query rows with returned RxNorm concepts | 0 |
| Verified RxNorm concept evidence rows | 0 |
| Candidate rows with #1240 hit | 0 |
| Remaining no-hit candidate rows after #1240 | 1,019 |

Status counts:

| Status scope | `exact_hit` | `normalized_hit` | `no_external_hit` |
|---|---:|---:|---:|
| Pair status rows | 0 | 0 | 640 |
| Candidate status rows | 0 | 0 | 1,019 |

Validation assertions:

| Assertion | Result |
|---|---|
| #1236 persisted readback all true | true |
| #1236 Calyx readback all true | true |
| Candidate status rows cover every remaining no-hit row | true |
| Pair status rows cover every unique pair key | true |
| Query response exists for every queryable pair key | true |
| All status values are allowed | true |
| All status rows carry the clinical boundary | true |
| All candidate rows remain blocked | true |
| All hits have evidence | true |
| Bridge rows <= 1,000 | true |

## Native Calyx Materialization

```text
name: issue1240-rxnorm-combination-products-20260704t172703z
vault_id: 01KWQ3AQAXTC6SXY3G57RVX4Q8
vault_dir: /home/croyse/calyx/vaults/01KWQ3AQAXTC6SXY3G57RVX4Q8
```

Materialization readback:

| Item | Count / value |
|---|---:|
| Bridge rows | 640 |
| Bridge terms | 205 |
| Graph nodes | 845 |
| Graph edges | 3,840 |
| CSR persisted | true |
| Active vault index contains final name exactly once | true |
| Graph nodes match readback | true |
| Graph edges match readback | true |
| Vault manifest/CURRENT files present | true |
| Graph files present | true |

## Result

RxNorm combination-product concept mining produced no verified source hits for
the #1236 remaining no-hit universe. The useful result is negative evidence:
1,019 candidate rows remain `no_external_hit` after this additional public
source check, with source bytes, raw API responses, hashes, status rows, and
Calyx graph materialization persisted.

Downstream work should continue source expansion from
`candidate_rxnorm_status.jsonl`, filtered to
`overall_external_source_status_after_issue1240 == no_external_hit`.
