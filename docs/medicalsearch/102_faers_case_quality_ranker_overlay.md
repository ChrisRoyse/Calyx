# #1261 FAERS Case-Quality Ranker Overlay

Status: complete for the #1190 drug-combination ranker feedback pass using the
#1260 metformin-trametinib FAERS case validation.

This slice reads sealed #1190 combination-ranker artifacts and sealed #1260
case-validation artifacts, joins the #1260 confounded serious FAERS case onto
the metformin/trametinib family in #1190, emits case-quality/confounder features
and blocked ranker overlay rows, and materializes the overlay into native Calyx.

Clinical boundary:

```text
FAERS case-quality ranker overlay is safety/source/falsification triage only; case-quality rows, confounder features, and rank penalties are blockers or review inputs, not causality, safety clearance, efficacy, treatment guidance, dosing guidance, recommendation, clinical actionability, pair-interaction proof, or cure evidence.
```

## Inputs

FSV root:

```text
/home/croyse/calyx/fsv/issue1261-faers-case-quality-ranker-overlay-20260705T030000Z
```

Sealed inputs:

| Input | Rows | SHA-256 |
|---|---:|---|
| #1190 `candidate_pair_inputs.jsonl` | 1,750 | `d61a04e62114d4124fade8a9f5a2a506c500a601d4c35615e56866aa3b697654` |
| #1190 `drug_combination_hypotheses.jsonl` | 1,750 | `f5bf39325a5905770203b7326ded955831cf5e44e1797f1e37139ae1a8bfd484` |
| #1190 `combination_safety_interaction_flags.jsonl` | 1,750 | `86b1a07aad0afd7a64bdc009bc7db18c147efe2ac226ea12612ac085acd575ab` |
| #1190 `persisted_readback.json` | - | `9520cbcd24138df535dbfdba7cd3468897f0cd3e2e194a13437da7fa7349fbe2` |
| #1190 `calyx_bridge_corpus_readback.json` | - | `444ba74aa3fc6fbf4ca41407d5b24ac086b64d4a5df7408c5f9fc67aa4579d0f` |
| #1260 `faers_query_rows.jsonl` | 1 | `6b2de6cfdad1ffbcd7b86427f54121972837b57a7e48155c8b55713191a8501a` |
| #1260 `faers_case_rows.jsonl` | 1 | `30f6908118eae47ba7fdec35a414766caea916a4762a910a45fc9aee7a1b3524` |
| #1260 `case_rollups.jsonl` | 1 | `f1f045ed882eae25a8da0fda8025a35dd6b9a02f0b7db368cce5d89c127777a5` |
| #1260 `candidate_case_status.jsonl` | 4 | `cc01e740f67c88ac8cd9d2da775310a86dbdf2c5c45b84f524b826a7ed5ca656` |
| #1260 `persisted_readback.json` | - | `c44de7d913ed74d44f91e5158ad547d314189f8b45ce1f6004fb59b2d2cf1ae0` |
| #1260 `calyx_bridge_corpus_readback.json` | - | `a4f070383487e8ad0d0d2878a0cf8f79d534992d7aee6ab582ca4f1006815045` |

Source contract:

- Input hashes were checked before processing.
- Pair normalization maps `trametinib dimethyl sulfoxide`, `trametinib`, and
  `Mekinist` to the ingredient-family key `trametinib`; metformin remains
  `metformin`.
- The #1260 case family key is `metformin||trametinib`.
- Direct #1260 candidate-status joins are preserved separately from the broader
  ingredient-family context row.
- The overlay does not mutate #1190 rows. It emits blocked overlay rows that can
  be consumed by a downstream ranker.

## Output Artifacts

| Artifact | Rows | Bytes | SHA-256 |
|---|---:|---:|---|
| `source_rows.jsonl` | 11 | 10,500 | `1a54cadd9687ed6936c20942f5e05520d7afcdfc8140a2fb608e056eb0f4bbf6` |
| `case_quality_features.jsonl` | 5 | 11,639 | `22e9f8336dffa55349fa7c94ea2a14e85b22cbcef565fee15955ad1173b44c3c` |
| `ranker_case_quality_overlay.jsonl` | 5 | 10,120 | `d5f31d382afc25584577aab7b58b6c0268a71e5840fa170fde1467b0e8e0cbb8` |
| `family_case_quality_summary.jsonl` | 1 | 1,578 | `27a703ce5bf4a8eb0e19ce703742265db2120628ecc52ab310d832a1bb8cd533` |
| `issue1261_bridge_rows.jsonl` | 22 | 25,800 | `1609dfb625c34aecb67ad8ba56b50858b9c5db6290cc40337b79a3f06502756d` |
| `validation_metrics.json` | - | 998 | `d21541fed315baa35881578a476fc403cc6249bb6f880d3ee14bf69e5818ba72` |
| `output_manifest.json` | - | 8,846 | `859f130028c8a0b5f7ec15c0861b1a509bc1e930be335f0c5d5a681bf46d9180` |
| `persisted_readback.json` | - | 3,125 | `b4322712d8725a33ab9968ba22488f17298e5c22924e4e3888f88dded7274b3b` |
| `calyx_bridge_corpus_stdout.json` | - | 758 | `7ec8e3b27bd75cfe94d589209e4aa43c6cdbe69ec1aa0ce5ccc56d43be3a7a05` |
| `calyx_bridge_corpus_stderr.txt` | - | 319 | `446d0eb3e60ab2ae0cb5e31992f5f93c6e1c47a0b39d884e534277c871d874b7` |
| `calyx_bridge_corpus_readback.json` | - | 1,728 | `f05e25dfca9af51b4850c9db06d5713c92950d369a790d39687a1de84846ad45` |

## Metrics

| Metric | Count |
|---|---:|
| Source input rows | 11 |
| Case-quality feature rows | 5 |
| Ranker overlay rows | 5 |
| Family summary rows | 1 |
| Direct #1260 candidate-status joins | 4 |
| Ingredient-family context rows | 1 |
| Bridge rows | 22 |

Domain counts:

| Domain | Rows |
|---|---:|
| `issue1261_source_input` | 11 |
| `issue1261_case_quality_feature` | 5 |
| `issue1261_ranker_overlay` | 5 |
| `issue1261_family_summary` | 1 |

Case-quality class:

| Class | Rows |
|---|---:|
| `confounded_case_report_blocker` | 5 |

Overlay status:

| Status | Rows |
|---|---:|
| `blocked_case_quality_confounded_still_blocked` | 5 |

## Affected Ranker Rows

The overlay applies a deterministic 0.45 case-quality penalty. This is a
fail-closed ranker feature, not a measured clinical effect size.

| Pair ID | Pair | Disease | Original rank | Original score | Adjusted overlay score | Direct #1260 join |
|---|---|---|---:|---:|---:|---|
| `issue1190:85c287a7a7864b3621b8bd39` | `TRAMETINIB DIMETHYL SULFOXIDE` + `METFORMIN` | Cardiofaciocutaneous syndrome 1 | 418 | 0.596529 | 0.146529 | true |
| `issue1190:0d987768bb01e33c82b5ebb9` | `TRAMETINIB DIMETHYL SULFOXIDE` + `METFORMIN` | Cardiofaciocutaneous syndrome | 609 | 0.563515 | 0.113515 | true |
| `issue1190:ebdda04e878f80cb9f3c0de4` | `TRAMETINIB DIMETHYL SULFOXIDE` + `METFORMIN` | Noonan syndrome | 736 | 0.546177 | 0.096177 | true |
| `issue1190:440d65be8fca3e11b9bd7dbf` | `TRAMETINIB DIMETHYL SULFOXIDE` + `METFORMIN` | Noonan syndrome 1 | 926 | 0.516483 | 0.066483 | true |
| `issue1190:c32560a52b99549f090012fe` | `Metformin` + `Trametinib` | Cancer | 1046 | 0.500000 | 0.050000 | false |

Summary:

```text
pair_family_key: metformin||trametinib
faers_source_id: 24608768
case_quality_class: confounded_case_report_blocker
case_quality_penalty_points: 0.45
affected_ranker_rows: 5
direct_issue1260_candidate_status_rows: 4
ingredient_family_context_rows: 1
all_rows_blocked: true
```

Reason codes added or carried forward:

- `serious_faers_report_found`
- `eliquis_primary_suspect_anticoagulant_confounder_present`
- `trametinib_metformin_concomitant_not_primary_suspect`
- `polypharmacy_case_report_not_pair_causality`
- `faers_case_quality_confounded_blocker`
- `faers_pair_concomitant_not_primary_suspect`
- `faers_anticoagulant_confounder_present`
- `faers_polypharmacy_case_report_not_pair_causality`

## Validation Assertions

| Assertion | Result |
|---|---|
| Expected input hashes match | true |
| Affected ranker rows = 5 | true |
| Direct #1260 rows = 4 | true |
| Ingredient-family context rows = 1 | true |
| Family summary rows = 1 | true |
| Family key matches | true |
| All overlay rows blocked and no-promotion | true |
| Case-quality class is confounded | true |
| Penalty is positive | true |
| Bridge terms present in text | true |
| Bridge metadata `source_dataset` present | true |

## Native Calyx Materialization

```text
name: issue1261-faers-case-quality-ranker-overlay-20260705t030000z
vault_id: 01KWQVREZW2SGTRE38KS8G27MA
vault_dir: /home/croyse/calyx/vaults/01KWQVREZW2SGTRE38KS8G27MA
rows: 22
bridge_terms: 35
graph_nodes_written: 57
graph_edges_written: 172
csr_persisted: true
graph_file_count: 226
graph_bytes: 200586
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
