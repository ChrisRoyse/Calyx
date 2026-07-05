# #1260 Metformin-Trametinib FAERS Case Validation

Status: complete for the case-level validation of the #1259 serious FAERS
co-report blocker for `metformin||trametinib dimethyl sulfoxide`.

This slice read the sealed #1259 artifacts, re-fetched the exact openFDA FAERS
case by `safetyreportid:24608768`, queried identity/label/literature context
sources, and materialized the result into native Calyx. The result is a
blocked safety-review artifact only.

Clinical boundary:

```text
FAERS case-level validation is safety/source/falsification triage only; case reports, label text, RxNorm identity rows, and literature rows are blockers or review inputs, not causality, safety clearance, efficacy, treatment guidance, dosing guidance, recommendation, clinical actionability, pair-interaction proof, or cure evidence.
```

## Inputs

FSV root:

```text
/home/croyse/calyx/fsv/issue1260-metformin-trametinib-faers-case-20260705T020000Z
```

Sealed #1259 inputs:

| Input | Rows | SHA-256 |
|---|---:|---|
| `independent_evidence_rows.jsonl` | 1 | `6e8643e558790549baba17cef531a6cfc3475c69654ce731ea2ccb519a45b0ca` |
| `pair_validation_rollups.jsonl` | 7 | `186227f7b52aa1433c22c1725a5de2d89e645af1198caa5134196225128b156c` |
| `candidate_validation_status.jsonl` | 16 | `b6a3eeaa15add12f23b57ae2956c6dfddfadb78ab45788e7a4f4c1e0de843700` |
| `calyx_bridge_corpus_readback.json` | - | `10462d52057fa323443d1e2ae8c0fe752c464b2424a1e726eb3dc1b6a1680ce1` |

Persisted source documentation snapshots:

| Source doc | HTTP | SHA-256 |
|---|---:|---|
| `openfda_event_docs` | 200 | `8a043cbfa4650d79191f05309b279687ada890e074432e6ea80574dcab0c61f8` |
| `openfda_label_docs` | 200 | `e538ce31106786b2f1e6432bf815804abda79889b74ee061ea34cea2d11df0be` |
| `dailymed_spls_api` | 200 | `727d6a6a7345430e54100f230ee545081f23fcffb120a78e1c047ecfdba27add` |
| `europepmc_rest_docs` | 200 | `e3b45ced2caf9a35496a33ffbe79e745f79a48ba61d2fbc6996cf94d6bce4c97` |
| `ncbi_eutilities_intro` | 200 | `e0c8f8d9563afb18adaaa7a0bed1f9ca58b33ef39f9dc4775e9d1ebda9cf88b4` |
| `rxnorm_find_rxcui` | 200 | `169ad799d3153b19e5d56597b7a8d2850b10a69f7fa3a3edd9e4d0562e9bbbae` |
| `rxnorm_related` | 200 | `9b0f92ac5831eb2549fb96e475254b6382a3532e0c87cf8ae8e1fc81edce37cf` |

Source contract:

- Scope was exactly the #1259 independent FAERS evidence row for
  `metformin||trametinib dimethyl sulfoxide`, source ID `24608768`, and the
  four downstream candidate rows touched by that pair.
- Expected #1259 hashes were verified before processing.
- The exact FAERS query row was persisted separately from the interpreted case
  row.
- RxNorm identity rows were gathered for metformin, trametinib/trametinib DMSO,
  Mekinist, Eliquis, and apixaban.
- Label context was queried through openFDA labels and DailyMed SPL metadata/XML.
- Literature context was queried through Europe PMC and PubMed E-utilities.
- Every output row remains blocked behind case-level safety falsification and
  human review.

## Output Artifacts

| Artifact | Rows | Bytes | SHA-256 |
|---|---:|---:|---|
| `source_rows.jsonl` | 7 | 8,535 | `6bece0dcb8f22e4af21608d69ba614d22d68df760ec741d20ac8aaa0d857a867` |
| `faers_query_rows.jsonl` | 1 | 1,053 | `6b2de6cfdad1ffbcd7b86427f54121972837b57a7e48155c8b55713191a8501a` |
| `faers_case_rows.jsonl` | 1 | 10,344 | `30f6908118eae47ba7fdec35a414766caea916a4762a910a45fc9aee7a1b3524` |
| `rxnorm_identity_rows.jsonl` | 12 | 17,222 | `59b797f289ea08380ee97bbaeec4e71c8f0305d5c9d96235048bf057acde2fbd` |
| `label_query_rows.jsonl` | 22 | 4,446,904 | `2735563b36974eaebedff9e2b94d2aab41fe453df620f9e7285f9d8a49eeadc1` |
| `label_evidence_rows.jsonl` | 154 | 848,168 | `551a10563115939917ab6bf8b54b99cebaf33794a5d3faed3139dfa7bae976a2` |
| `literature_query_rows.jsonl` | 10 | 173,531 | `bab78cbe58e3068d74e0f351d9d1a18f5d52918dbe809b5113c1b90f45dc8cff` |
| `literature_evidence_rows.jsonl` | 7 | 7,048 | `bf31398c4e1cf748c782b7f1a7390fa46c8d20db124cc471ad1f911369554585` |
| `case_rollups.jsonl` | 1 | 1,802 | `f1f045ed882eae25a8da0fda8025a35dd6b9a02f0b7db368cce5d89c127777a5` |
| `candidate_case_status.jsonl` | 4 | 4,552 | `cc01e740f67c88ac8cd9d2da775310a86dbdf2c5c45b84f524b826a7ed5ca656` |
| `issue1260_bridge_rows.jsonl` | 187 | 195,129 | `cfb2b6c7802ff025281c43441dd70b19d7b1b08135202840f7a56a10ff0eff18` |
| `validation_metrics.json` | - | 1,307 | `06971f05f0f9f9a70f606b605c2359854b3871ce4b855bc469033386693af194` |
| `output_manifest.json` | - | 6,021 | `b50fb116f79e30358ae42755fb121613c0bc196eeb1c3e4dbeb1c0f954dd967e` |
| `persisted_readback.json` | - | 4,990 | `c44de7d913ed74d44f91e5158ad547d314189f8b45ce1f6004fb59b2d2cf1ae0` |
| `calyx_bridge_corpus_stdout.json` | - | 873 | `4554af0e323d4f4a4f04e0d7f303798626313d7eda484ce3db0ade169e9ea788` |
| `calyx_bridge_corpus_stderr.txt` | - | 327 | `52480689a9534458b80e72c16a4da3b61e7b9c44704eb2dcc260c81a92f8fd8f` |
| `calyx_bridge_corpus_readback.json` | - | 1,869 | `a4f070383487e8ad0d0d2878a0cf8f79d534992d7aee6ab582ca4f1006815045` |

## Case Result

Exact FAERS query:

| Field | Value |
|---|---|
| URL | `https://api.fda.gov/drug/event.json?search=safetyreportid%3A24608768&limit=1` |
| HTTP status | 200 |
| Total | 1 |
| Returned | 1 |
| Raw response SHA-256 | `6820234835424c56b6834e2de7110944073ea2c1e1dc0955cbad8c432a43f284` |

Case rollup:

| Field | Value |
|---|---|
| Pair key | `metformin||trametinib dimethyl sulfoxide` |
| Source ID | `24608768` |
| Classification | `serious_faers_case_confounded_still_blocked` |
| Received date | `20241112` |
| Receipt date | `20241230` |
| Serious | true |
| Reactions | Off label use; Lower gastrointestinal haemorrhage |
| Drug count | 19 |
| Pair drugs present | true |
| Pair drugs are concomitant | true |
| Primary suspect drugs | Eliquis; Eliquis |
| Anticoagulant confounders | Eliquis; Eliquis |
| Candidate rows touched | 4 |

Reason codes:

- `serious_faers_report_found`
- `eliquis_primary_suspect_anticoagulant_confounder_present`
- `trametinib_metformin_concomitant_not_primary_suspect`
- `polypharmacy_case_report_not_pair_causality`
- `requires_human_review`

Interpretation: the case is a preserved safety blocker, not a validated
metformin-trametinib interaction. The case has both pair drugs, but they are
reported as concomitant. Eliquis appears twice as the primary suspect drug, and
the case has 19 total drugs.

Label context rows:

| Term | Rows |
|---|---:|
| `APIXABAN` | 47 |
| `ELIQUIS` | 47 |
| `METFORMIN` | 43 |
| `MEKINIST` | 6 |
| `TRAMETINIB` | 6 |
| `TRAMETINIB DIMETHYL SULFOXIDE` | 5 |

These are keyword safety-context rows only. They do not establish a pair
interaction or treatment/safety conclusion.

Literature context:

| Query | Total |
|---|---:|
| `metformin_trametinib` | 13 |
| `metformin_trametinib_bleeding` | 2 |
| `metformin_mekinist` | 0 |
| `metformin_trametinib_dimethyl_sulfoxide` | 0 |

Seven returned records passed the deterministic title/abstract both-term gate.
They are co-mention/context rows only, including preclinical oncology and drug
repositioning titles, and do not validate clinical actionability.

## Validation Assertions

| Assertion | Result |
|---|---|
| Expected #1259 input hashes matched | true |
| One exact FAERS query row | true |
| Exact FAERS query total one | true |
| Exact FAERS query returned one | true |
| Exact FAERS raw response exists | true |
| One FAERS case row | true |
| FAERS case source ID matches | true |
| FAERS case has both pair drugs | true |
| FAERS case is serious | true |
| FAERS case has anticoagulant confounder | true |
| One case rollup row | true |
| Four candidate status rows | true |
| Case rollup blocked | true |
| Candidate statuses blocked | true |
| Bridge terms present in text | true |
| Bridge rows <= 1,000 | true |

## Native Calyx Materialization

```text
name: issue1260-metformin-trametinib-faers-case-20260705t020000z
vault_id: 01KWQV29K8Y4KX17WTCCRFQ685
vault_dir: /home/croyse/calyx/vaults/01KWQV29K8Y4KX17WTCCRFQ685
rows: 187
bridge_terms: 55
graph_nodes_written: 242
graph_edges_written: 1918
csr_persisted: true
graph_file_count: 2163
graph_bytes: 1943124
```

Calyx readback assertions:

| Assertion | Result |
|---|---|
| Materialize exit zero | true |
| Materialize status ok | true |
| Row count matches read rows | true |
| Row SHA matches stdout | true |
| Vault directory exists | true |
| `CURRENT` and `MANIFEST` exist | true |
| Graph directory exists | true |
| Graph files present | true |
| CSR persisted | true |
| Index contains materialization name | true |
| Graph node count positive | true |
| Graph edge count positive | true |
| Domain counts match | true |
| Bridge metadata `source_dataset` present | true |

## Follow-Up State

This closes the #1259 independent FAERS signal at case-validation depth as a
confounded safety blocker. The next useful derived task is to feed FAERS role,
polypharmacy, and primary-suspect confounder features back into the
drug-combination ranker so serious co-reports are ranked by case quality rather
than raw co-presence.
