# #1236 openFDA Drug Label Source Mining

## Scope

#1236 rechecks the #1234 remaining no-hit candidate-pair universe against
openFDA Human Drug Label. This source is distinct from DrugComb, NCI ALMANAC,
CDCDB, ClinicalTrials.gov v2, FDA Orange Book, FDA NDC, and PubMed
ESearch/ESummary.

The stage reads #1234 `candidate_external_source_status.jsonl`, filters rows
where `overall_external_source_status == no_external_hit`, queries one openFDA
label search per unique pair key, persists the raw query responses, and verifies
both candidate names in returned label-section text before marking a hit.

The output is source-attributed regulatory-label text mining only. It is not
efficacy proof, safety proof, treatment guidance, dosing guidance,
recommendation, clinical actionability, or cure evidence.

## Implementation

Script:

```text
scripts/medicalsearch/issue1236_openfda_label_source_mining.py
```

The script:

- reads #1234 `candidate_external_source_status.jsonl`;
- filters 1,041 remaining `no_external_hit` candidate rows;
- collapses them to 649 unique pair keys;
- persists openFDA docs/license/terms/download-manifest pages;
- queries openFDA Human Drug Label once per unique pair key;
- verifies both candidate names in returned label sections;
- emits deterministic `exact_hit`, `normalized_hit`, or `no_external_hit`
  status rows for all 1,041 candidate rows;
- emits one pair-status row for each of 649 unique pair keys;
- emits evidence rows only when source label text contains both candidate names;
- writes a 689-row bridge-corpus slice for native Calyx materialization.

Accepted source:

- openFDA download manifest: <https://api.fda.gov/download.json>
- openFDA drug-label overview: <https://open.fda.gov/apis/drug/label/>
- openFDA drug-label endpoint guide: <https://open.fda.gov/apis/drug/label/how-to-use-the-endpoint/>
- openFDA query syntax: <https://open.fda.gov/apis/query-syntax/>
- openFDA authentication/rate-limit docs: <https://open.fda.gov/apis/authentication/>
- openFDA license: <https://open.fda.gov/license/>
- openFDA terms: <https://open.fda.gov/terms/>

## Real FSV

Final FSV root:

```text
/home/croyse/calyx/fsv/issue1236-openfda-label-source-mining-20260704T170534Z
```

Primary readbacks:

```text
/home/croyse/calyx/fsv/issue1236-openfda-label-source-mining-20260704T170534Z/out/persisted_readback.json
sha256: 9e738bb57b5cf53d54981c82d61ae5bb44181d667bc19b23a15c5cde838677db

/home/croyse/calyx/fsv/issue1236-openfda-label-source-mining-20260704T170534Z/out/calyx_bridge_corpus_readback.json
sha256: 5bf8cc3e01e7c55d70d300b6a87f1a167e1d6d36e8257383b722d66ec4db3b6e
```

Native Calyx materialization:

```text
name: issue1236-openfda-label-source-mining-20260704t170534z
vault_id: 01KWQ25ZQ980MBGK7EAY93N5W9
vault_dir: /home/croyse/calyx/vaults/01KWQ25ZQ980MBGK7EAY93N5W9
```

Materialization readback:

| Item | Count / value |
|---|---:|
| Bridge rows | 689 |
| Bridge terms | 252 |
| Graph nodes | 941 |
| Graph edges | 4,364 |
| CSR persisted | true |
| Active vault index contains final name exactly once | true |
| Graph nodes match readback | true |
| Graph edges match readback | true |
| Vault manifest/CURRENT files present | true |
| Graph files present | true |

## Source Artifacts

| Source artifact | Bytes | SHA-256 |
|---|---:|---|
| `raw/openfda_download_manifest.json` | 582,744 | `546c94d92983a0b0c5f7ebe10bddfa2159f23aef9324db22d1ee3074b39c26d1` |
| `raw/openfda_label_overview.html` | 126,680 | `e538ce31106786b2f1e6432bf815804abda79889b74ee061ea34cea2d11df0be` |
| `raw/openfda_label_howto.html` | 117,945 | `532743e382cccafd6bf567148d9034ee1e8960aaa2c8287172ebac5ddccc1400` |
| `raw/openfda_query_syntax.html` | 124,308 | `b4fb28bf0791b6ebe7ccf8b7acd7218d7644bba97c0b70146cba33caf0f66c84` |
| `raw/openfda_authentication.html` | 116,494 | `d46c961be22f7eeb4e09f5c209eb81fee8a0119d5a242ac6baca8aac76bb898a` |
| `raw/openfda_license.html` | 120,790 | `9e906a722f7c4116154441bac21df15606fd2fe4335fb1a7b415b1acaf16da97` |
| `raw/openfda_terms.html` | 128,668 | `1a9217ccc118017674dc72ebce4e811706f2d895a82352ab38ffc2165ded0019` |

The openFDA download manifest reported drug-label export date `2026-07-04`,
260,158 total label records, and 14 partitions.

## Output Artifacts

| Artifact | Rows | Bytes | SHA-256 |
|---|---:|---:|---|
| `openfda_label_query_responses.jsonl` | 649 | 7,841,289 | `bc20dab983c0ba7ace46a33cccc2a18b9699ed974919070433700595456f841c` |
| `openfda_label_pair_evidence.jsonl` | 40 | 376,714 | `d2376732cea19e4f17d4f803e5e7c0b07cbd6ef4898fb7b527a6a1509c304ab3` |
| `openfda_label_pair_status.jsonl` | 649 | 1,722,675 | `9a1680d5243a90ab52af7bb93b76f8e7677c7b7cef338282bf1cd781b5d7524a` |
| `candidate_openfda_label_status.jsonl` | 1,041 | 1,296,930 | `0e8893d8bef722f66ae656ece2335a7f7eabcb226bf429c6658b0e5ad9ee791b` |
| `openfda_label_bridge_rows.jsonl` | 689 | 876,827 | `cbbde20b652f98afada1f111978f8711c7a0d46a825e2ab02d9390c5f2a31047` |
| `input_manifest.json` | - | 5,284 | `ced99b743efb759fc4369af4cc7acaa4c17e124cc740c04bb7e4dff4f6ded4d6` |
| `validation_metrics.json` | - | 7,263 | `76b53ba9966d3168230ff58d3a4c55ee31b1ce9ec3e3461bf1a2d1305c090d4c` |
| `output_manifest.json` | - | 2,357 | `c69cb405c3d36dce0536eb37361937aecaf4fd9d0d82d041a83b398053934420` |
| `persisted_readback.json` | - | 3,602 | `9e738bb57b5cf53d54981c82d61ae5bb44181d667bc19b23a15c5cde838677db` |
| `calyx_bridge_corpus_stdout.json` | - | 701 | `2bbc2bf40e909c345b641ef242dedf460e4391f1cee1f369b98d882bd234cfba` |
| `calyx_bridge_corpus_readback.json` | - | 3,555 | `5bf8cc3e01e7c55d70d300b6a87f1a167e1d6d36e8257383b722d66ec4db3b6e` |

## Metrics

| Metric | Count |
|---|---:|
| #1234 remaining no-hit candidate rows | 1,041 |
| Unique pair keys queried | 649 |
| openFDA query response rows | 649 |
| Query responses with HTTP 200 | 9 |
| Query responses with HTTP 404 | 640 |
| Query rows with verified evidence | 9 |
| openFDA label evidence rows | 40 |
| Candidate rows with #1236 hit | 22 |
| Remaining no-hit rows after #1236 | 1,019 |
| Evidence rows with safety-section match | 26 |
| Evidence rows with interaction-section match | 28 |

Pair status counts:

| Status | Unique pair keys |
|---|---:|
| `exact_hit` | 8 |
| `normalized_hit` | 1 |
| `no_external_hit` | 640 |

Candidate status counts:

| Status | Candidate rows |
|---|---:|
| `exact_hit` | 21 |
| `normalized_hit` | 1 |
| `no_external_hit` | 1,019 |

Matched section fields:

| Field | Evidence rows |
|---|---:|
| `drug_interactions` | 27 |
| `drug_interactions_table` | 20 |
| `precautions` | 24 |
| `warnings_and_cautions` | 2 |
| `clinical_pharmacology` | 1 |
| `pharmacokinetics` | 1 |

## Readback Assertions

| Assertion | Value |
|---|---:|
| #1234 persisted readback assertions all true | true |
| #1234 Calyx readback assertions all true | true |
| Candidate status row for every remaining no-hit row | true |
| Pair status row for every unique pair key | true |
| Query response for every queryable pair key | true |
| All pair/candidate statuses in allowed set | true |
| All evidence/status rows carry the clinical boundary | true |
| All hits have evidence | true |
| All evidence rows have matched sections and source IDs | true |
| All candidate rows remain blocked | true |
| Bridge rows <= 1,000 | true |
| Bridge row count matches materializer stdout | true |
| Bridge SHA matches materializer stdout | true |
| Active vault index contains exactly one final name | true |
| Active vault id matches materializer stdout | true |
| Vault manifest/CURRENT files present | true |
| Graph files present | true |

## Findings

- openFDA label mining found 9 unique pair keys with verified label-section
  evidence among the 649 unique #1234 remaining no-hit pair keys.
- These 9 pair keys map to 22 candidate rows.
- 40 openFDA label evidence rows were persisted.
- 26 evidence rows match safety-related label sections and 28 match
  interaction-related label sections; these are review inputs only and do not
  clear safety or pair-interaction gates.
- 1,019 candidate rows remain no-hit after #1236.

## Conclusion

#1236 is complete for openFDA Human Drug Label source mining. It adds
source-attributed label evidence for 22 previously no-hit candidate rows,
keeps all rows blocked, and materializes the result into Calyx.

No treatment claim, efficacy claim, safety claim, clinical-actionability claim,
recommendation, dosing guidance, or cure claim is made.
